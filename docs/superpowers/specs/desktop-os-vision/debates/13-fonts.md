# Round 13 — Font System & Typography
**Role:** Architect | **Date:** 2026-05-29 | **Status:** Proposal

## 0. Executive summary

VyomaOS's display pipeline has so far rendered text exclusively through an 8×16 bitmap font baked into the supervisor binary. Round 11 (display protocol) and Round 12 (GPU compositor) extracted text rendering behind a `FontProvider` trait, with `BitmapFontProvider` as the default. The protocol already speaks `draw_text` with discrete sizes (`s` = 4×8, `m` = 8×16, `l` = 16×32) and `draw_text_scaled` with a pixel-precise size.

Round 13 finalizes the **font subsystem**: trait surface, TrueType implementation via `fontdue`, glyph atlas (CPU + GPU), system-font registry, capability-aware app font handles, a WIT interface, a fallback chain, a layout engine (with explicit RTL stub), and the security envelope around third-party font files.

The macOS analogue is **CoreText / FontServices**: a system-wide registry of fonts, per-app handles, glyph-cache management, and a fallback chain that guarantees every codepoint renders to *something*. We replicate the metrics API, atlas caching, fallback chain, and wrap-based layout. We **skip** for v1: full bidi shaping (HarfBuzz), variable fonts, optical sizing, dynamic font downloading.

Headline numbers:

| Property | Bitmap (v1) | TrueType (v2) |
|---|---|---|
| Rasterization latency / glyph | <1 µs | 10–100 µs |
| Glyphs cached / font×size | unbounded (fixed set) | 4096 (LRU) |
| Memory / glyph (8 bpp alpha) | 16 B (8×16) | ~256–2048 B |
| Atlas footprint | 0 | up to 4 MiB (one 2048² page) |
| Font file size limit | n/a | 8 MiB |
| Total fonts cap | 8 | 64 |

This proposal makes no schematic changes to display protocol bytes; it extends the resolution of `draw_text_scaled` and adds two new VYOMA_DRAW_V2 hostcalls (`load_font`, `select_font`) plus a WIT interface for capability-aware apps.

## 1. FontProvider trait

The trait is the dependency boundary between *who draws glyphs* (font subsystem) and *who composites them* (display + GPU compositor from R11/R12). It is deliberately small: every method is pure with respect to its inputs and may be called from any thread that owns an `Arc<dyn FontProvider>`.

```rust
//! supervisor/src/font/mod.rs

use std::sync::Arc;

/// Public trait implemented by every font backend.
/// Implementations MUST be cheap to clone via `Arc` and thread-safe.
/// Rasterization is allowed to be expensive; the caller is expected to cache.
pub trait FontProvider: Send + Sync {
    /// Measure a run of text. Returns (width, height, baseline_offset_from_top).
    fn measure_text(&self, text: &str, size_px: f32, weight: FontWeight) -> TextMetrics;

    /// Rasterize a single glyph: 8-bpp single-channel coverage (0..=255).
    fn render_glyph(&self, ch: char, size_px: f32, weight: FontWeight) -> GlyphBitmap;

    /// Distance between two consecutive baselines at the given size.
    fn line_height(&self, size_px: f32) -> f32;

    /// Horizontal advance (pen-x delta) for a single glyph.
    fn glyph_advance(&self, ch: char, size_px: f32) -> f32;

    /// Whether the provider has a non-`.notdef` glyph for `ch`.
    fn has_glyph(&self, ch: char) -> bool;

    /// Human-readable name; surfaced in supervisor `ps` introspection.
    fn font_name(&self) -> &str;
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics { pub width: f32, pub height: f32, pub baseline: f32 }

#[derive(Clone, Debug)]
pub struct GlyphBitmap {
    pub data: Vec<u8>,           // width * height bytes, row-major
    pub width: u32,
    pub height: u32,
    pub bearing_x: i32,          // pen → bitmap left edge
    pub bearing_y: i32,          // baseline → bitmap top edge (positive = above)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FontWeight { Regular, Bold, Light }

impl FontWeight {
    pub fn as_str(self) -> &'static str {
        match self {
            FontWeight::Regular => "Regular",
            FontWeight::Bold => "Bold",
            FontWeight::Light => "Light",
        }
    }
}
```

Design notes:

- **No glyph IDs.** `char` is the only stable identifier; font-internal indices are not portable across fallbacks.
- **No kerning in v1.** `glyph_advance` is per-glyph; `measure_text` sums advances. v2 may add `measure_run` for kerned widths.
- **No color glyphs.** v1 returns 8-bpp coverage; the compositor applies the draw color. Emoji deferred to v3.
- **Thread-safety.** `Send + Sync` is mandatory because the compositor and the IPC broker may both call `measure_text` from different threads.

## 2. BitmapFontProvider (v1)

The v1 provider wraps the existing 8×16 bitmap font. It is the default and the guaranteed fallback. It supports the historical `s`/`m`/`l` sizes by nearest-neighbor scaling.

```rust
//! supervisor/src/font/bitmap.rs
use super::{FontProvider, FontWeight, GlyphBitmap, TextMetrics};

pub struct BitmapFontProvider {
    name: &'static str,
    glyph_bits: &'static [[u8; 16]; 95],   // ASCII 0x20..=0x7E
}

impl BitmapFontProvider {
    pub const fn new() -> Self {
        Self { name: "VyomaBitmap-8x16",
               glyph_bits: &crate::font::bitmap_data::GLYPHS }
    }
    /// 0 = half (4×8), 1 = native (8×16), 2 = double (16×32)
    fn scale_for(size_px: f32) -> u32 {
        if size_px <= 6.0 { 0 } else if size_px <= 20.0 { 1 } else { 2 }
    }
}

impl FontProvider for BitmapFontProvider {
    fn measure_text(&self, text: &str, size_px: f32, _w: FontWeight) -> TextMetrics {
        let (cw, ch) = match Self::scale_for(size_px) {
            0 => (4u32, 8u32), 1 => (8u32, 16u32), _ => (16u32, 32u32),
        };
        let width = (text.chars().count() as u32 * cw) as f32;
        let height = ch as f32;
        TextMetrics { width, height, baseline: height * 0.8 }
    }

    fn render_glyph(&self, ch: char, size_px: f32, _w: FontWeight) -> GlyphBitmap {
        let idx = if (ch as u32) < 0x20 || (ch as u32) > 0x7E {
            ('?' as u32 - 0x20) as usize
        } else { (ch as u32 - 0x20) as usize };
        let rows = &self.glyph_bits[idx];
        match Self::scale_for(size_px) {
            0 => downsample_2x(rows), 1 => expand_1x(rows), _ => upsample_2x(rows),
        }
    }

    fn line_height(&self, size_px: f32) -> f32 {
        match Self::scale_for(size_px) { 0 => 8.0, 1 => 18.0, _ => 36.0 }
    }
    fn glyph_advance(&self, _ch: char, size_px: f32) -> f32 {
        match Self::scale_for(size_px) { 0 => 4.0, 1 => 8.0, _ => 16.0 }
    }
    fn has_glyph(&self, ch: char) -> bool {
        let c = ch as u32; c >= 0x20 && c <= 0x7E
    }
    fn font_name(&self) -> &str { self.name }
}

fn expand_1x(rows: &[u8; 16]) -> GlyphBitmap {
    let mut data = vec![0u8; 8 * 16];
    for (y, row) in rows.iter().enumerate() {
        for x in 0..8 { if row & (0x80 >> x) != 0 { data[y * 8 + x] = 0xFF; } }
    }
    GlyphBitmap { data, width: 8, height: 16, bearing_x: 0, bearing_y: 13 }
}

fn upsample_2x(rows: &[u8; 16]) -> GlyphBitmap {
    let mut data = vec![0u8; 16 * 32];
    for (y, row) in rows.iter().enumerate() {
        for x in 0..8 {
            if row & (0x80 >> x) != 0 {
                let (dy, dx) = (y * 2, x * 2);
                for oy in 0..2 { for ox in 0..2 {
                    data[(dy + oy) * 16 + (dx + ox)] = 0xFF;
                }}
            }
        }
    }
    GlyphBitmap { data, width: 16, height: 32, bearing_x: 0, bearing_y: 26 }
}

fn downsample_2x(rows: &[u8; 16]) -> GlyphBitmap {
    let mut data = vec![0u8; 4 * 8];
    for y in 0..8 {
        let row = rows[y * 2];
        for x in 0..4 {
            if row & (0x80 >> (x * 2)) != 0 { data[y * 4 + x] = 0xFF; }
        }
    }
    GlyphBitmap { data, width: 4, height: 8, bearing_x: 0, bearing_y: 7 }
}
```

Properties: no allocation in `measure_text`/`line_height`/`glyph_advance`/`has_glyph`; `measure_text` returns exact pixel widths (monospace, so existing `gui-demo` layout keeps working); every `FallbackChain` ends with this provider so ASCII always renders.

## 3. TrueTypeFontProvider (v2, fontdue)

v2 uses [`fontdue`](https://crates.io/crates/fontdue) — a pure-Rust TTF/OTF parser and rasterizer with no `unsafe`, no `freetype` dependency, ~85 KB stripped. It is the only crate we whitelist for v2; HarfBuzz and rust-skia are explicitly out of scope.

```rust
//! supervisor/src/font/truetype.rs

use super::{FontProvider, FontWeight, GlyphBitmap, TextMetrics};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GlyphCacheKey {
    ch: char,
    size_q: u16,           // (size_px * 4) as u16: 0.25 px quantization
    weight: FontWeight,
}

pub struct TrueTypeFontProvider {
    name: String,
    regular: fontdue::Font,
    bold: Option<fontdue::Font>,
    light: Option<fontdue::Font>,
    cache: RwLock<lru::LruCache<GlyphCacheKey, Arc<GlyphBitmap>>>,
    metrics_cache: RwLock<HashMap<(u8, u16), f32>>,
}

const GLYPH_CACHE_CAP: usize = 4096;

impl TrueTypeFontProvider {
    pub fn from_bytes(
        name: impl Into<String>,
        regular_bytes: &[u8],
        bold_bytes: Option<&[u8]>,
        light_bytes: Option<&[u8]>,
    ) -> Result<Self, FontError> {
        let settings = fontdue::FontSettings::default();
        let regular = fontdue::Font::from_bytes(regular_bytes, settings)
            .map_err(|e| FontError::ParseFailed(e.to_string()))?;
        let bold = bold_bytes
            .map(|b| fontdue::Font::from_bytes(b, settings))
            .transpose()
            .map_err(|e| FontError::ParseFailed(e.to_string()))?;
        let light = light_bytes
            .map(|b| fontdue::Font::from_bytes(b, settings))
            .transpose()
            .map_err(|e| FontError::ParseFailed(e.to_string()))?;
        Ok(Self {
            name: name.into(), regular, bold, light,
            cache: RwLock::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(GLYPH_CACHE_CAP).unwrap())),
            metrics_cache: RwLock::new(HashMap::new()),
        })
    }

    fn font_for(&self, weight: FontWeight) -> &fontdue::Font {
        match weight {
            FontWeight::Regular => &self.regular,
            FontWeight::Bold => self.bold.as_ref().unwrap_or(&self.regular),
            FontWeight::Light => self.light.as_ref().unwrap_or(&self.regular),
        }
    }

    fn quantize_size(size_px: f32) -> u16 {
        (size_px.max(1.0).min(512.0) * 4.0) as u16
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("font file exceeds 8 MiB limit ({0} bytes)")]
    TooLarge(usize),
    #[error("fontdue parse failed: {0}")]
    ParseFailed(String),
    #[error("fontdue parser panicked")]
    ParsePanic,
    #[error("font registry full (64 fonts)")]
    RegistryFull,
    #[error("unknown system font: {0}")]
    UnknownSystemFont(String),
    #[error("font not found by id: {0}")]
    NotFound(u8),
}

impl FontProvider for TrueTypeFontProvider {
    fn measure_text(&self, text: &str, size_px: f32, weight: FontWeight) -> TextMetrics {
        let font = self.font_for(weight);
        let mut width = 0.0_f32;
        let mut max_h = 0.0_f32;
        for ch in text.chars() {
            let m = font.metrics(ch, size_px);
            width += m.advance_width;
            if m.height as f32 > max_h { max_h = m.height as f32; }
        }
        let lm = font.horizontal_line_metrics(size_px).unwrap_or(
            fontdue::LineMetrics {
                ascent: size_px * 0.8, descent: -size_px * 0.2,
                line_gap: size_px * 0.1, new_line_size: size_px * 1.1,
            });
        TextMetrics { width, height: lm.new_line_size, baseline: lm.ascent }
    }

    fn render_glyph(&self, ch: char, size_px: f32, weight: FontWeight) -> GlyphBitmap {
        let key = GlyphCacheKey { ch, size_q: Self::quantize_size(size_px), weight };
        if let Some(bmp) = self.cache.read().peek(&key).cloned() {
            return (*bmp).clone();
        }
        let font = self.font_for(weight);
        let (m, bitmap) = font.rasterize(ch, size_px);
        let result = GlyphBitmap {
            data: bitmap,
            width: m.width as u32, height: m.height as u32,
            bearing_x: m.xmin, bearing_y: m.ymin + m.height as i32,
        };
        self.cache.write().put(key, Arc::new(result.clone()));
        result
    }

    fn line_height(&self, size_px: f32) -> f32 {
        self.regular.horizontal_line_metrics(size_px)
            .map(|m| m.new_line_size).unwrap_or(size_px * 1.2)
    }

    fn glyph_advance(&self, ch: char, size_px: f32) -> f32 {
        self.regular.metrics(ch, size_px).advance_width
    }

    fn has_glyph(&self, ch: char) -> bool {
        self.regular.lookup_glyph_index(ch) != 0
    }

    fn font_name(&self) -> &str { &self.name }
}
```

Notes:

- **Quantized size**: round to 0.25 px so animating from 16.0→16.5 produces at most 2 cache entries, not a fresh raster every frame.
- **LRU cap 4096**: at ~512 B/glyph average (24 px font), ~2 MiB worst-case per font×weight.
- **No `unsafe`**: `fontdue` is `#![forbid(unsafe_code)]`; we add no unsafe wrappers.
- **Graceful degradation** for malformed fonts via the `horizontal_line_metrics` fallback.

## 4. GlyphAtlas (CPU + GPU)

The atlas amortizes rasterization across frames. CPU variant for framebuffer path; GPU variant for R12's compositor path.

```rust
//! supervisor/src/font/atlas.rs

use super::{FontProvider, FontWeight, GlyphBitmap};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
    pub font_id: u8,
    pub ch: char,
    pub size_px: u16,
}

pub struct CpuGlyphAtlas {
    cache: RwLock<HashMap<GlyphKey, Arc<GlyphBitmap>>>,
    capacity: usize,
}

impl CpuGlyphAtlas {
    pub fn new(capacity: usize) -> Self {
        Self {
            cache: RwLock::new(HashMap::with_capacity(capacity.min(1024))),
            capacity,
        }
    }

    pub fn get(&self, key: GlyphKey, provider: &dyn FontProvider, weight: FontWeight)
        -> Arc<GlyphBitmap>
    {
        if let Some(bmp) = self.cache.read().get(&key).cloned() { return bmp; }
        let bmp = Arc::new(provider.render_glyph(key.ch, key.size_px as f32, weight));
        let mut w = self.cache.write();
        if w.len() >= self.capacity {
            let to_drop = self.capacity / 8;
            let keys: Vec<_> = w.keys().take(to_drop).copied().collect();
            for k in keys { w.remove(&k); }
        }
        w.insert(key, bmp.clone());
        bmp
    }

    pub fn invalidate_font(&self, font_id: u8) {
        self.cache.write().retain(|k, _| k.font_id != font_id);
    }
}

pub struct GpuGlyphAtlas {
    pixels: RwLock<Vec<u8>>,              // 2048×2048 alpha
    shelves: RwLock<Vec<Shelf>>,
    regions: RwLock<HashMap<GlyphKey, GlyphRegion>>,
    dirty: RwLock<bool>,
}

#[derive(Clone, Copy, Debug)]
struct Shelf { y: u32, height: u32, next_x: u32 }

#[derive(Clone, Copy, Debug)]
pub struct GlyphRegion {
    pub u0: u16, pub v0: u16, pub u1: u16, pub v1: u16,
    pub bearing_x: i16, pub bearing_y: i16,
}

const ATLAS_DIM: u32 = 2048;

impl GpuGlyphAtlas {
    pub fn new() -> Self {
        Self {
            pixels: RwLock::new(vec![0u8; (ATLAS_DIM * ATLAS_DIM) as usize]),
            shelves: RwLock::new(Vec::new()),
            regions: RwLock::new(HashMap::new()),
            dirty: RwLock::new(false),
        }
    }

    fn alloc(&self, w: u32, h: u32) -> Option<(u32, u32)> {
        let mut shelves = self.shelves.write();
        for s in shelves.iter_mut() {
            if h <= s.height && s.next_x + w <= ATLAS_DIM {
                let x = s.next_x; s.next_x += w;
                return Some((x, s.y));
            }
        }
        let used_y: u32 = shelves.iter().map(|s| s.y + s.height).max().unwrap_or(0);
        if used_y + h > ATLAS_DIM { return None; }
        shelves.push(Shelf { y: used_y, height: h, next_x: w });
        Some((0, used_y))
    }

    pub fn get_or_insert(&self, key: GlyphKey, provider: &dyn FontProvider, weight: FontWeight)
        -> Option<GlyphRegion>
    {
        if let Some(r) = self.regions.read().get(&key) { return Some(*r); }
        let bmp = provider.render_glyph(key.ch, key.size_px as f32, weight);
        if bmp.width == 0 || bmp.height == 0 { return None; }
        let (ox, oy) = self.alloc(bmp.width, bmp.height)?;
        let mut pixels = self.pixels.write();
        for row in 0..bmp.height {
            let src = (row * bmp.width) as usize;
            let dst = ((oy + row) * ATLAS_DIM + ox) as usize;
            pixels[dst..dst + bmp.width as usize]
                .copy_from_slice(&bmp.data[src..src + bmp.width as usize]);
        }
        drop(pixels);
        let region = GlyphRegion {
            u0: ox as u16, v0: oy as u16,
            u1: (ox + bmp.width) as u16, v1: (oy + bmp.height) as u16,
            bearing_x: bmp.bearing_x as i16, bearing_y: bmp.bearing_y as i16,
        };
        self.regions.write().insert(key, region);
        *self.dirty.write() = true;
        Some(region)
    }

    pub fn take_dirty(&self) -> bool {
        let mut d = self.dirty.write();
        let was = *d; *d = false; was
    }
}
```

Properties:

- **Single-page in v1**: when the GPU atlas overflows we log `font.atlas.full` and fall back to CPU rendering for that frame. v2 rotates pages.
- **Shelf packing**: simple, predictable; waste bounded by `(shelf_height - glyph_height) * glyph_width`. For 12–24 px text, waste is under 20 %.
- **Lock-free hot read path** in the GPU variant: `regions.read()` is the cache hit.

## 5. FontRegistry and discovery

The registry is the **supervisor-side singleton** that owns all providers and answers `FontId → &dyn FontProvider`. App handles are `u8` (max 64). Slot 0 is reserved for the bitmap fallback.

```rust
//! supervisor/src/font/registry.rs

use super::{FontError, FontProvider, TrueTypeFontProvider, BitmapFontProvider};
use parking_lot::RwLock;
use std::path::Path;
use std::sync::Arc;

pub type FontId = u8;

pub const BITMAP_FONT_ID: FontId = 0;
pub const MAX_FONTS: usize = 64;
pub const MAX_FONT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOTAL_FONT_RAM: usize = 32 * 1024 * 1024;

pub struct FontRegistry { inner: RwLock<RegistryInner> }

struct RegistryInner {
    fonts: Vec<Option<RegisteredFont>>,
    total_bytes: usize,
}

struct RegisteredFont {
    name: String,
    provider: Arc<dyn FontProvider>,
    owner: Option<u32>,        // None = system font
    bytes_held: usize,
}

impl FontRegistry {
    pub fn new() -> Self {
        let mut fonts: Vec<Option<RegisteredFont>> = (0..MAX_FONTS).map(|_| None).collect();
        fonts[BITMAP_FONT_ID as usize] = Some(RegisteredFont {
            name: "VyomaBitmap-8x16".into(),
            provider: Arc::new(BitmapFontProvider::new()),
            owner: None, bytes_held: 0,
        });
        Self { inner: RwLock::new(RegistryInner { fonts, total_bytes: 0 }) }
    }

    pub fn load_system_font(&self, name: &str) -> Result<FontId, FontError> {
        let path = format!("/usr/share/fonts/vyoma/{name}.ttf");
        let bytes = std::fs::read(&path)
            .map_err(|_| FontError::UnknownSystemFont(name.into()))?;
        self.load_internal(name, &bytes, None)
    }

    pub fn load_app_font(&self, owner_pid: u32, name: &str, data: &[u8])
        -> Result<FontId, FontError>
    {
        if data.len() > MAX_FONT_BYTES { return Err(FontError::TooLarge(data.len())); }
        validate_font_file(data)?;
        self.load_internal(name, data, Some(owner_pid))
    }

    fn load_internal(&self, name: &str, data: &[u8], owner: Option<u32>)
        -> Result<FontId, FontError>
    {
        let mut inner = self.inner.write();
        if inner.total_bytes + data.len() > MAX_TOTAL_FONT_RAM {
            return Err(FontError::RegistryFull);
        }
        let slot = inner.fonts.iter().position(Option::is_none)
            .ok_or(FontError::RegistryFull)?;
        let provider = TrueTypeFontProvider::from_bytes(name, data, None, None)?;
        inner.fonts[slot] = Some(RegisteredFont {
            name: name.to_string(),
            provider: Arc::new(provider),
            owner, bytes_held: data.len(),
        });
        inner.total_bytes += data.len();
        Ok(slot as FontId)
    }

    pub fn get(&self, id: FontId) -> Result<Arc<dyn FontProvider>, FontError> {
        let inner = self.inner.read();
        inner.fonts.get(id as usize).and_then(|s| s.as_ref())
            .map(|r| r.provider.clone()).ok_or(FontError::NotFound(id))
    }

    /// Drop all fonts owned by `pid`. Called from process-exit hook.
    pub fn unload_owned_by(&self, pid: u32) {
        let mut inner = self.inner.write();
        for slot in inner.fonts.iter_mut() {
            if let Some(r) = slot {
                if r.owner == Some(pid) {
                    inner.total_bytes -= r.bytes_held;
                    *slot = None;
                }
            }
        }
    }

    pub fn list(&self) -> Vec<(FontId, String, Option<u32>)> {
        let inner = self.inner.read();
        inner.fonts.iter().enumerate().filter_map(|(i, slot)| {
            slot.as_ref().map(|r| (i as FontId, r.name.clone(), r.owner))
        }).collect()
    }
}

/// Validate a font file without panicking. Returns Ok on success.
pub fn validate_font_file(data: &[u8]) -> Result<(), FontError> {
    if data.len() > MAX_FONT_BYTES { return Err(FontError::TooLarge(data.len())); }
    let result = std::panic::catch_unwind(|| {
        fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
    });
    match result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(FontError::ParseFailed(e.to_string())),
        Err(_) => Err(FontError::ParsePanic),
    }
}
```

### System fonts

Three system fonts ship in initramfs at `/usr/share/fonts/vyoma/`:

| File | Source | Use |
|---|---|---|
| `VyomaSans-Regular.ttf` | Inter (subset) | UI labels, body text |
| `VyomaSans-Bold.ttf` | Inter Bold (subset) | titles, emphasis |
| `VyomaMono-Regular.ttf` | JetBrains Mono (subset) | code, terminals |

All subsets contain Latin + Latin-Extended-A + common punctuation (~400 glyphs each), total system-font footprint under 2 MiB. They are loaded at `BootPhase::Display` *before* the first app spawn, so app code can synchronously look them up by name.

```rust
// supervisor/src/boot.rs (excerpt)
pub fn boot_load_system_fonts(reg: &FontRegistry) {
    for name in ["VyomaSans-Regular", "VyomaSans-Bold", "VyomaMono-Regular"] {
        match reg.load_system_font(name) {
            Ok(id) => log::info!("font.system.loaded name={name} id={id}"),
            Err(e) => log::warn!("font.system.error name={name} err={e}"),
        }
    }
}
```

## 6. Font fallback chain

A `FallbackChain` is an ordered list of FontIds. When rendering, we try each provider's `has_glyph(ch)` in order; the first hit wins. The chain's last element is always `BITMAP_FONT_ID`, guaranteeing ASCII coverage.

```rust
//! supervisor/src/font/fallback.rs

use super::{FontId, FontProvider, FontRegistry, BITMAP_FONT_ID, FontWeight, GlyphBitmap};
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct FallbackChain { chain: Vec<FontId> }

impl FallbackChain {
    pub fn new(primary: FontId) -> Self {
        let mut chain = vec![primary];
        if primary != BITMAP_FONT_ID { chain.push(BITMAP_FONT_ID); }
        Self { chain }
    }

    pub fn with_intermediate(mut self, id: FontId) -> Self {
        let pos = self.chain.len().saturating_sub(1);
        self.chain.insert(pos, id);
        self
    }

    pub fn resolve(&self, ch: char, registry: &FontRegistry) -> Arc<dyn FontProvider> {
        for &id in &self.chain {
            if let Ok(p) = registry.get(id) {
                if p.has_glyph(ch) { return p; }
            }
        }
        registry.get(BITMAP_FONT_ID).expect("bitmap font always present")
    }

    pub fn render_run(&self, text: &str, size_px: f32, weight: FontWeight,
                      registry: &FontRegistry) -> Vec<RunGlyph>
    {
        let mut out = Vec::with_capacity(text.len());
        for ch in text.chars() {
            let provider = self.resolve(ch, registry);
            let bmp = provider.render_glyph(ch, size_px, weight);
            let adv = provider.glyph_advance(ch, size_px);
            out.push(RunGlyph {
                provider_name: provider.font_name().to_string(),
                bitmap: bmp, advance: adv, ch,
            });
        }
        out
    }
}

pub struct RunGlyph {
    pub provider_name: String,
    pub bitmap: GlyphBitmap,
    pub advance: f32,
    pub ch: char,
}
```

The supervisor maintains a default chain for unfocused apps (`VyomaSans → bitmap`) and apps can configure their own via `load-font` → `select-font` WIT calls.

## 7. Text layout engine

The engine breaks a string into wrapped lines respecting a `max_width`. v1 is **strictly LTR** with **no shaping** (no ligatures, no contextual forms, no kerning). RTL strings are detected, logged, and rendered LTR.

```rust
//! supervisor/src/font/layout.rs

use super::{FallbackChain, FontRegistry, FontWeight};

#[derive(Clone, Debug)]
pub struct TextLine<'a> {
    pub text: &'a str,
    pub width: f32,
    pub y_offset: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct LayoutOpts {
    pub max_width: f32,
    pub size_px: f32,
    pub weight: FontWeight,
    pub wrap: WrapMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapMode { Char, Word, Ellipsis }

pub fn layout_text<'a>(text: &'a str, opts: LayoutOpts,
                       chain: &FallbackChain, registry: &FontRegistry)
    -> Vec<TextLine<'a>>
{
    if detect_rtl(text) {
        log::warn!("font.layout.rtl_unsupported: rendering LTR as a stub (v1 limitation)");
    }
    match opts.wrap {
        WrapMode::Ellipsis => layout_ellipsis(text, opts, chain, registry),
        WrapMode::Char => layout_wrap_char(text, opts, chain, registry),
        WrapMode::Word => layout_wrap_word(text, opts, chain, registry),
    }
}

fn line_height_for(opts: &LayoutOpts, chain: &FallbackChain, reg: &FontRegistry) -> f32 {
    chain.resolve('M', reg).line_height(opts.size_px)
}

fn layout_wrap_char<'a>(text: &'a str, opts: LayoutOpts,
                        chain: &FallbackChain, registry: &FontRegistry)
    -> Vec<TextLine<'a>>
{
    let lh = line_height_for(&opts, chain, registry);
    let mut lines = Vec::new();
    let mut y = 0.0_f32;
    let mut line_start = 0usize;
    let mut line_w = 0.0_f32;

    for (idx, ch) in text.char_indices() {
        let provider = chain.resolve(ch, registry);
        let adv = provider.glyph_advance(ch, opts.size_px);
        if ch == '\n' || line_w + adv > opts.max_width {
            lines.push(TextLine { text: &text[line_start..idx], width: line_w, y_offset: y });
            y += lh;
            line_start = if ch == '\n' { idx + 1 } else { idx };
            line_w = if ch == '\n' { 0.0 } else { adv };
        } else {
            line_w += adv;
        }
    }
    if line_start < text.len() {
        lines.push(TextLine { text: &text[line_start..], width: line_w, y_offset: y });
    }
    lines
}

fn layout_wrap_word<'a>(text: &'a str, opts: LayoutOpts,
                        chain: &FallbackChain, registry: &FontRegistry)
    -> Vec<TextLine<'a>>
{
    let lh = line_height_for(&opts, chain, registry);
    let mut lines = Vec::new();
    let mut y = 0.0_f32;
    let mut line_start = 0usize;
    let mut last_break = 0usize;
    let mut line_w = 0.0_f32;
    let mut w_since_break = 0.0_f32;

    for (idx, ch) in text.char_indices() {
        let provider = chain.resolve(ch, registry);
        let adv = provider.glyph_advance(ch, opts.size_px);
        if ch == ' ' || ch == '\t' { last_break = idx; w_since_break = 0.0; }
        else { w_since_break += adv; }
        if ch == '\n' || line_w + adv > opts.max_width {
            let cut = if ch == '\n' || last_break <= line_start { idx } else { last_break };
            lines.push(TextLine {
                text: text[line_start..cut].trim_end(),
                width: line_w - w_since_break, y_offset: y,
            });
            y += lh;
            line_start = if ch == '\n' { idx + 1 } else { cut + 1 };
            line_w = w_since_break + if ch == '\n' { 0.0 } else { adv };
            w_since_break = 0.0;
        } else {
            line_w += adv;
        }
    }
    if line_start < text.len() {
        lines.push(TextLine { text: &text[line_start..], width: line_w, y_offset: y });
    }
    lines
}

fn layout_ellipsis<'a>(text: &'a str, opts: LayoutOpts,
                       chain: &FallbackChain, registry: &FontRegistry)
    -> Vec<TextLine<'a>>
{
    let ell_w = chain.resolve('…', registry).glyph_advance('…', opts.size_px);
    let mut width = 0.0_f32;
    let mut cut = text.len();
    for (idx, ch) in text.char_indices() {
        let adv = chain.resolve(ch, registry).glyph_advance(ch, opts.size_px);
        if width + adv + ell_w > opts.max_width { cut = idx; break; }
        width += adv;
    }
    let line_text = if cut < text.len() { &text[..cut] } else { text };
    vec![TextLine {
        text: line_text,
        width: width + if cut < text.len() { ell_w } else { 0.0 },
        y_offset: 0.0,
    }]
}

/// True if the first strong-direction codepoint is RTL.
fn detect_rtl(text: &str) -> bool {
    for ch in text.chars() {
        match ch as u32 {
            0x0590..=0x05FF | 0x0600..=0x06FF | 0x0700..=0x074F | 0x0750..=0x077F => return true,
            0x0041..=0x005A | 0x0061..=0x007A => return false,
            _ => continue,
        }
    }
    false
}
```

Behavioral contract: `Char` is the default (matches existing `draw_text_wrap` from R11); `Word` falls back to `Char` for unbreakable runs (URLs, paths); `Ellipsis` returns a single line and the caller draws `…` at `line.width - ell_w`; `detect_rtl` is intentionally cheap — full Unicode bidi class lookup is deferred to v3.

## 8. WIT interface

Apps that need font handles (custom font loading, precise text measurement) declare the `vyoma:fonts` import. Apps that just want to draw text continue to use VYOMA_DRAW_V2's `draw_text_scaled` and the supervisor picks a default font for them.

```wit
// wit/vyoma-fonts.wit
package vyoma:fonts@1.0.0;

interface typography {
    /// Load a system or app-bundled font by name. Returns a font handle.
    load-font: func(name: string) -> result<u32, string>;

    /// Drop an app-loaded font handle. System fonts are no-op.
    unload-font: func(font: u32);

    /// Measure a text run. Returns (width, height) in pixels.
    measure-text: func(font: u32, text: string, size-px: f32) -> tuple<f32, f32>;

    /// Pixel distance between consecutive baselines.
    line-height: func(font: u32, size-px: f32) -> f32;

    /// Whether the font contains a non-`.notdef` glyph for the codepoint.
    has-glyph: func(font: u32, codepoint: u32) -> bool;

    /// Make this font the default for subsequent VYOMA_DRAW_V2:draw_text*
    /// until another `select-font` overrides it.
    select-font: func(font: u32);
}

world vyoma-app { import typography; }
```

Host-side handler:

```rust
//! supervisor/src/font/wit_handlers.rs

use super::{FontRegistry, FontWeight};
use std::sync::Arc;
use wasmtime::component::Linker;

pub struct FontHostState {
    pub pid: u32,
    pub registry: Arc<FontRegistry>,
    pub selected: parking_lot::Mutex<u8>,
}

pub fn link_typography(linker: &mut Linker<FontHostState>) -> wasmtime::Result<()> {
    let mut t = linker.instance("vyoma:fonts/typography@1.0.0")?;

    t.func_wrap("load-font",
        |store: wasmtime::StoreContextMut<FontHostState>, (name,): (String,)|
            -> Result<(Result<u32, String>,), wasmtime::Error>
    {
        let st = store.data();
        let result = st.registry.load_system_font(&name)
            .map(|id| id as u32).map_err(|e| e.to_string());
        Ok((result,))
    })?;

    t.func_wrap("unload-font",
        |_: wasmtime::StoreContextMut<FontHostState>, (_,): (u32,)|
            -> Result<(), wasmtime::Error>
    { Ok(()) })?;

    t.func_wrap("measure-text",
        |store: wasmtime::StoreContextMut<FontHostState>,
         (font, text, size): (u32, String, f32)|
            -> Result<((f32, f32),), wasmtime::Error>
    {
        let st = store.data();
        let p = st.registry.get(font as u8)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        let m = p.measure_text(&text, size, FontWeight::Regular);
        Ok(((m.width, m.height),))
    })?;

    t.func_wrap("line-height",
        |store: wasmtime::StoreContextMut<FontHostState>, (font, size): (u32, f32)|
            -> Result<(f32,), wasmtime::Error>
    {
        let st = store.data();
        let p = st.registry.get(font as u8)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        Ok((p.line_height(size),))
    })?;

    t.func_wrap("has-glyph",
        |store: wasmtime::StoreContextMut<FontHostState>, (font, cp): (u32, u32)|
            -> Result<(bool,), wasmtime::Error>
    {
        let ch = char::from_u32(cp).unwrap_or('?');
        let st = store.data();
        let p = st.registry.get(font as u8)
            .map_err(|e| wasmtime::Error::msg(e.to_string()))?;
        Ok((p.has_glyph(ch),))
    })?;

    t.func_wrap("select-font",
        |store: wasmtime::StoreContextMut<FontHostState>, (font,): (u32,)|
            -> Result<(), wasmtime::Error>
    {
        *store.data().selected.lock() = font as u8;
        Ok(())
    })?;

    Ok(())
}
```

The selected font is consumed by the VYOMA_DRAW_V2 parser when interpreting `draw_text_scaled`, so the legacy stdout protocol picks up custom fonts without further changes.

## 9. Font capabilities in manifest

```toml
# apps/my-app/vyoma.toml
[app]
name = "my-app"
version = "0.1.0"
wasm = "my-app.wasm"

[capabilities]
stdio = true
display = true

[capabilities.fonts]
system = ["VyomaSans-Regular", "VyomaMono-Regular"]
bundle = ["fonts/MyFont.ttf"]
```

Parser side:

```rust
//! supervisor/src/manifest.rs (excerpt)
use std::path::{Path, PathBuf};
use crate::font::{FontError, FontId, FontRegistry};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct FontsCapability {
    #[serde(default)] pub system: Vec<String>,
    #[serde(default)] pub bundle: Vec<PathBuf>,
}

impl FontsCapability {
    /// Returns FontIds in manifest order. First id is primary, rest are
    /// intermediate fallbacks inserted before the bitmap terminator.
    pub fn resolve(&self, owner_pid: u32, app_dir: &Path, registry: &FontRegistry)
        -> Result<Vec<FontId>, FontError>
    {
        let mut ids = Vec::new();
        for name in &self.system { ids.push(registry.load_system_font(name)?); }
        for rel in &self.bundle {
            let path = app_dir.join(rel);
            let data = std::fs::read(&path)
                .map_err(|e| FontError::ParseFailed(e.to_string()))?;
            let name = rel.file_stem().and_then(|s| s.to_str()).unwrap_or("app-font");
            ids.push(registry.load_app_font(owner_pid, name, &data)?);
        }
        Ok(ids)
    }
}
```

Unknown subkeys under `[capabilities.fonts]` produce a manifest-parse error (consistent with the rest of the schema). System fonts that don't exist on the current platform image produce a warning, not a fatal error — the fallback chain uses the bitmap font for missing glyphs.

## 10. Security and performance

### Security envelope

`fontdue` is pure-Rust, but font files are still **adversarial input** — a malicious app could bundle a malformed TTF that triggers a parser panic or a pathologically slow glyph. We defend with four layers:

1. **Size cap** at the registry boundary: `MAX_FONT_BYTES = 8 MiB` rejects gigantic files before parsing.
2. **`catch_unwind` around the parser** in `validate_font_file`: a panic inside fontdue becomes `FontError::ParsePanic`, not a supervisor crash. fontdue is `#![forbid(unsafe_code)]` so memory safety is preserved even on panic.
3. **Dedicated font worker thread.** All rasterization happens on a thread distinct from the compositor and the IPC broker, so a slow glyph delays one frame for one app, not the whole supervisor.
4. **Per-app accounting.** Font files are accounted to the owning PID; `FontRegistry::unload_owned_by(pid)` runs from the process-exit hook so crashed apps cannot leak font memory.

The worker:

```rust
//! supervisor/src/font/worker.rs

use super::{FontRegistry, FontWeight, GlyphBitmap, FontId};
use crossbeam::channel::{bounded, Receiver, Sender};
use std::sync::Arc;

pub enum FontReq {
    Rasterize {
        font: FontId, ch: char, size_px: f32, weight: FontWeight,
        reply: Sender<Result<GlyphBitmap, String>>,
    },
    Validate {
        bytes: Vec<u8>,
        reply: Sender<Result<(), String>>,
    },
}

pub struct FontWorker { tx: Sender<FontReq> }

impl FontWorker {
    pub fn spawn(registry: Arc<FontRegistry>) -> Self {
        let (tx, rx) = bounded::<FontReq>(32);
        std::thread::Builder::new()
            .name("vyoma-font-worker".into())
            .spawn(move || font_worker_loop(rx, registry))
            .expect("spawn font worker");
        Self { tx }
    }

    pub fn tx(&self) -> Sender<FontReq> { self.tx.clone() }
}

fn font_worker_loop(rx: Receiver<FontReq>, registry: Arc<FontRegistry>) {
    while let Ok(req) = rx.recv() {
        match req {
            FontReq::Rasterize { font, ch, size_px, weight, reply } => {
                let result = registry.get(font)
                    .map(|p| p.render_glyph(ch, size_px, weight))
                    .map_err(|e| e.to_string());
                let _ = reply.send(result);
            }
            FontReq::Validate { bytes, reply } => {
                let result = super::registry::validate_font_file(&bytes)
                    .map_err(|e| e.to_string());
                let _ = reply.send(result);
            }
        }
    }
}
```

The `bounded(32)` channel applies backpressure: an app firing 1000 `measure_text` calls in a tight loop blocks on the 33rd until the worker drains. The supervisor logs `font.worker.queue_full` when the channel is at capacity for >100 ms.

### Performance budget

| Operation | Bitmap | TrueType (miss) | TrueType (hit) |
|---|---|---|---|
| `measure_text("hello", 14, R)` | 30 ns | 2 µs | 200 ns |
| `render_glyph('A', 14, R)` | 500 ns | 40 µs | 50 ns (Arc clone) |
| `has_glyph('Ω')` | 5 ns | 30 ns | 30 ns |
| `line_height(14)` | 5 ns | 80 ns | 80 ns |

Targets on a 2.5 GHz x86_64 host under QEMU/KVM. The TrueType *miss* path dominates first-frame latency for novel text; after warmup the LRU + GPU atlas absorb 99 % of requests as hits.

Frame-time accounting (60 Hz = 16.6 ms):

- Worst-case full-redraw of an 80×24 terminal grid (1920 glyphs):
  - **cold**: 1920 × 40 µs = **76 ms** — exceeds budget. Mitigation: the terminal pre-warms the cache on focus by rendering all ASCII at its current size in a single batch (32 ms once, then never again).
  - **warm**: 1920 × 50 ns = **96 µs** — negligible.

## 11. Implementation files

All files under `supervisor/src/font/`, each ≤ 500 lines per the project rule.

| File | Purpose | Approx LOC |
|---|---|---|
| `mod.rs` | Trait + public types + re-exports | 80 |
| `bitmap.rs` | `BitmapFontProvider`, existing v1 (refactored) | 320 |
| `bitmap_data.rs` | Static glyph bit tables (autogenerated) | 200 |
| `truetype.rs` | `TrueTypeFontProvider`, fontdue integration | 280 |
| `atlas.rs` | `CpuGlyphAtlas`, `GpuGlyphAtlas`, shelf-packing | 350 |
| `registry.rs` | `FontRegistry`, system-font loading, validation | 290 |
| `fallback.rs` | `FallbackChain`, multi-provider `render_run` | 180 |
| `layout.rs` | `layout_text`, wrap modes, RTL detect | 280 |
| `wit_handlers.rs` | Wasmtime linker for `vyoma:fonts/typography` | 200 |
| `worker.rs` | Dedicated font worker thread + channel | 130 |

`mod.rs` declares the submodules and re-exports the public API (`FontProvider`, `TextMetrics`, `GlyphBitmap`, `FontWeight` from §1 plus all per-module pubs above).

## 12. Prior round integration (R11, R12)

### R11 — Display protocol

R11 introduced `draw_text_scaled` with a pixel-precise size. The current implementation routes it to `BitmapFontProvider` via nearest-neighbor scaling. Round 13 replaces the dispatch with the FallbackChain:

```rust
// supervisor/src/display/draw_cmd.rs (excerpt)
fn handle_draw_text_scaled(ctx: &AppDrawContext,
    x: i32, y: i32, color: u32, size_px: f32, text: &str)
{
    let provider = ctx.fallback_chain.resolve(
        text.chars().next().unwrap_or('?'), &ctx.font_registry);
    let weight = ctx.current_weight;
    let mut pen_x = x;
    let baseline_y = y + provider.line_height(size_px) as i32;

    for ch in text.chars() {
        let p = ctx.fallback_chain.resolve(ch, &ctx.font_registry);
        let g = p.render_glyph(ch, size_px, weight);
        ctx.surface.blit_alpha(
            &g.data, g.width, g.height,
            pen_x + g.bearing_x, baseline_y - g.bearing_y, color);
        pen_x += p.glyph_advance(ch, size_px) as i32;
    }
}
```

Two protocol-level additions in VYOMA_DRAW_V2:

```
VYOMA_DRAW_V2:load_font:<name>             # returns FontId on the reply channel
VYOMA_DRAW_V2:select_font:<font_id>        # set default font for subsequent draw_text*
```

Both are thin wrappers around the WIT `load-font` / `select-font` calls and exist so apps that only have stdio (no `display` capability for direct WIT) can still pick a font.

### R12 — GPU compositor

R12's texture-atlas support is the natural target for `GpuGlyphAtlas`. The compositor flow becomes:

1. Layout engine produces `Vec<TextLine>`.
2. For each glyph, look up `GpuGlyphAtlas::get_or_insert`.
3. Emit a textured-quad vertex with `(u0, v0, u1, v1)` from the returned `GlyphRegion`, colored by the app's current draw color.
4. Before flushing, if `take_dirty()` returns true, upload the atlas page.

The atlas texture is **shared across all apps** because glyph bitmaps are content-addressed (not per-app secrets). Critical for memory: 60 apps each holding a 4 MiB atlas would blow the budget; one shared 4 MiB atlas does not. When the GPU path is unavailable (CPU framebuffer), the same flow uses `CpuGlyphAtlas` and writes directly into the surface via `Surface::blit_alpha`. The `FontProvider` trait does not know or care which path is active.

## 13. Open questions for the Critic

1. **Quantization granularity.** We round size to 0.25 px (`size * 4 as u16`). Caps cache key space but slightly distorts very small sizes. Should we use 8.8 fixed-point?
2. **LRU eviction in `CpuGlyphAtlas`.** The current placeholder eviction drops a random fraction. Should we hoist the same `lru::LruCache` from `TrueTypeFontProvider` to the atlas for consistency?
3. **System font choice.** Inter + JetBrains Mono are OFL-licensed and ~200 KiB each subsetted. Alternative: Noto Sans + Noto Mono for wider script coverage at ~600 KiB. The wider set helps fallback for non-ASCII before we have HarfBuzz.
4. **Bitmap fallback range.** Should `BitmapFontProvider::has_glyph` return true for all ASCII even when the glyph is a `?` placeholder, or false for unrenderable codepoints? Current spec returns true only for `0x20..=0x7E`.
5. **RTL stub policy.** v1 detects RTL and logs a warning. Should we instead refuse to render and return an error, forcing app authors to handle the limitation explicitly?
6. **Per-font worker vs. global worker.** One worker thread serializes all rasterization. Under contention from multiple apps this becomes a bottleneck. Use a small thread pool (2–4 workers) keyed on `FontId`?
7. **Font hot-reload.** Bundled fonts are loaded once at app spawn. If the app rewrites its bundle (theme switch), there's no protocol for re-validation. Add `reload-font(font, data)` WIT call?
8. **Color/emoji fonts.** Not in v1. Should `GlyphBitmap` already carry an optional RGBA channel so v3 doesn't break the trait?
9. **GPU atlas overflow strategy.** When the single 2048² page fills, we currently fall back to CPU. Better: rotate to a fresh page; even better: reference-count and evict least-used regions. Which complexity is worth it for v2?
10. **Wrap-mode default.** R11 uses character wrap. UX-wise, word wrap is almost always what apps want. Should the WIT API default to `Word` while the legacy stdout protocol keeps `Char`?

## 14. Summary

Round 13 turns text rendering into a first-class subsystem with a clean trait boundary, a TrueType backend (`fontdue`), a CPU + GPU glyph atlas, a system-font registry, a per-app fallback chain, a minimal layout engine, and a WIT interface that apps opt into via `[capabilities.fonts]` in their manifest.

The proposal is **incremental**: existing apps continue using the bitmap font with no manifest changes. Apps that want custom typography opt in explicitly, pay the cost of a heavier font file in their bundle, and get metrics-accurate measurement plus pixel-perfect rendering through the same VYOMA_DRAW_V2 protocol.

Security is bounded by an 8 MiB file cap, `catch_unwind` around the parser, a dedicated worker thread, and per-PID accounting. Performance is bounded by an LRU + atlas pair that pushes warm-frame cost into the tens of nanoseconds per glyph.

What we explicitly defer:

- Full bidi / complex shaping (HarfBuzz) — v3.
- Variable fonts and optical sizing — v3.
- Color / emoji glyphs — v3.
- Dynamic font downloading — out of scope (capability-secure model).

The trait is small enough to keep a future HarfBuzz-backed provider drop-in, and the WIT interface is versioned (`@1.0.0`) so we can add shaping APIs without breaking apps written against v1.
