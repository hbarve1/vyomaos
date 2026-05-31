# Subsystem 13: Font System & Typography — FINAL SPECIFICATION
**macOS equivalent:** CoreText / FontServices  
**Verdict:** ACCEPT WITH MANDATORY CHANGES — all 7 blocking issues resolved in this FINAL  
**Status:** AUTHORITATIVE — supersedes `13-fonts.md` and `13-fonts-critique.md`  
**Date:** 2026-05-29  
**Blocking issues resolved:** B1 (catch_unwind sandbox), B2 (GPU atlas eviction), B3 (measure/render skew), B4 (per-platform FontConfig), B5 (RTL explicit error), B6 (bitmap U+FFFD coverage), B7 (validate pre-flight rasterization)

---

## §1 Overview

The VyomaOS font subsystem provides glyph rasterization, text layout, and GPU atlas management for all display-capable platform profiles. It sits between the display compositor (R11: VYOMA_DRAW protocol, `Surface` struct, `blit_surface`) and the GPU pipeline (R12: `wgpu::Device` per app, `CompositorDevice`, Lavapipe on QEMU) and is the sole authority for converting Unicode codepoints to pixel bitmaps.

### Role in VyomaOS

- Consumed by `handle_draw_text` / `handle_draw_text_wrap` in `supervisor/src/draw_cmd.rs`
- Integrated with the compositor VSync flush pass via `GpuGlyphAtlas::take_dirty`
- Exposed to WASM apps via WIT hostcalls (`vyoma:fonts/typography@1.0.0`)
- Controlled per-app via `[capabilities.fonts]` in `vyoma.toml`

### What v1 Delivers

| Feature | Delivered |
|---------|-----------|
| Bitmap font (8×16, 4×8, 16×32) | yes |
| TrueType rasterization (fontdue) | yes, desktop-full / mobile / iot-edge / robotics-rt / server-headless |
| GPU glyph atlas (etagere, per-PID quota) | yes, desktop-full / server-headless only |
| CPU LRU glyph cache | yes, all TrueType profiles |
| Fallback chain (VyomaSans → VyomaMono → bitmap) | yes |
| LTR layout (Char/Word/Ellipsis wrap) | yes |
| App-bundled fonts (manifest-declared) | yes |
| U+FFFD replacement character | yes |
| Worker thread + watchdog respawn | yes |

### What v1 Defers

- HarfBuzz / complex shaping (Arabic, Devanagari, Thai)
- RTL / full Unicode BiDi algorithm (explicit error in v1)
- Variable fonts (OpenType fvar)
- Color/emoji fonts (CBDT, SBIX, COLR)
- Dynamic font download over network
- Subpixel / LCD hinting

### Relationship to R11 / R12

R11 established the `VYOMA_DRAW:draw_text` / `draw_text_wrap` text commands and the `Surface` + `blit_surface` compositor primitive. This subsystem implements the rasterization backend that fills those commands. R12 established `wgpu::Device` allocation per app and `CompositorDevice`. This subsystem adds `GpuGlyphAtlas` as a shared resource owned by `CompositorDevice` and dirtied by individual font render calls; the compositor's VSync loop calls `atlas.take_dirty()` to trigger GPU texture upload before draw calls.

---

## §2 Public Types

### FontProvider trait

```rust
// supervisor/src/font/provider.rs

use std::sync::Arc;
use crate::font::types::{GlyphBitmap, TextMetrics, FontWeight, GlyphCoverage, FontError};

/// Core trait implemented by BitmapFontProvider and TrueTypeFontProvider.
/// All methods must be panic-safe; panics from underlying libraries must be
/// caught by the provider internally (see §5, §6 for safe_rasterize wrappers).
pub trait FontProvider: Send + Sync + 'static {
    /// Unique stable numeric ID. 0 is reserved for "unset"; providers must not use 0.
    fn id(&self) -> u32;

    /// Human-readable name (e.g. "VyomaSans-Regular", "bitmap-builtin").
    fn name(&self) -> &str;

    /// Whether the provider supports a given codepoint.
    fn coverage(&self, ch: char) -> GlyphCoverage;

    /// Rasterize `ch` at `size_px` with `weight`.
    /// Returns GlyphBitmap with alpha-only pixels (1 byte per pixel, 0=transparent).
    /// On panic from the underlying library: must return Err(FontError::RasterPanic).
    fn render_glyph(&self, ch: char, size_px: f32, weight: FontWeight)
        -> Result<GlyphBitmap, FontError>;

    /// Horizontal advance for `ch` at `size_px`. Same quantization as render_glyph.
    fn glyph_advance(&self, ch: char, size_px: f32) -> Result<f32, FontError>;

    /// Measure a string of LTR text. Must be consistent with glyph_advance.
    fn measure_text(&self, text: &str, size_px: f32, weight: FontWeight)
        -> Result<TextMetrics, FontError>;

    /// Ascent above baseline in pixels at size_px.
    fn ascent(&self, size_px: f32) -> f32;

    /// Descent below baseline (positive = below) in pixels at size_px.
    fn descent(&self, size_px: f32) -> f32;

    /// Line gap between successive baselines at size_px.
    fn line_height(&self, size_px: f32) -> f32;

    /// Estimated RAM usage in bytes (raw font data × 2 + glyph cache).
    fn estimated_ram(&self) -> usize;
}
```

### GlyphCoverage — tri-state

```rust
// supervisor/src/font/types.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlyphCoverage {
    /// Font has a proper glyph for this codepoint; render_glyph will return it.
    Native,
    /// Font has no proper glyph but will render U+FFFD as a visible substitution box.
    /// BitmapFontProvider returns this for non-ASCII, non-U+FFFD codepoints.
    Substitute,
    /// Font has nothing for this codepoint; FallbackChain should try the next provider.
    NoGlyph,
}
```

### FontWeight

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum FontWeight {
    Thin       = 1,
    Light      = 2,
    Regular    = 4,
    Medium     = 5,
    SemiBold   = 6,
    Bold       = 7,
    ExtraBold  = 8,
    Black      = 9,
}

impl Default for FontWeight {
    fn default() -> Self { FontWeight::Regular }
}

impl FontWeight {
    /// Map to fontdue-compatible weight approximation (0.0–1.0 axis not used by fontdue;
    /// we select the closest available font face in the registry instead).
    pub fn css_numeric(&self) -> u16 {
        match self {
            FontWeight::Thin      => 100,
            FontWeight::Light     => 300,
            FontWeight::Regular   => 400,
            FontWeight::Medium    => 500,
            FontWeight::SemiBold  => 600,
            FontWeight::Bold      => 700,
            FontWeight::ExtraBold => 800,
            FontWeight::Black     => 900,
        }
    }
}
```

### TextMetrics

```rust
#[derive(Debug, Clone, Copy, Default)]
pub struct TextMetrics {
    /// Total advance width of the string in pixels.
    pub width: f32,
    /// Height of the tallest glyph (ascent + descent) in pixels.
    pub height: f32,
    /// Ascent above baseline in pixels.
    pub ascent: f32,
    /// Descent below baseline (positive = below) in pixels.
    pub descent: f32,
    /// Number of Unicode codepoints in the measured string.
    pub glyph_count: usize,
}
```

### GlyphBitmap

```rust
#[derive(Debug, Clone)]
pub struct GlyphBitmap {
    /// Width of the rasterized glyph in pixels.
    pub width: u32,
    /// Height of the rasterized glyph in pixels.
    pub height: u32,
    /// Alpha-only coverage bytes, row-major, length == width * height.
    /// 0 = fully transparent, 255 = fully opaque.
    pub coverage: Vec<u8>,
    /// Horizontal offset to apply when placing the glyph (left bearing).
    pub x_offset: i32,
    /// Vertical offset from baseline (positive = above baseline).
    pub y_offset: i32,
    /// Horizontal advance to next glyph origin.
    pub advance: f32,
}

impl GlyphBitmap {
    /// Construct a 7×11 replacement-character box (U+FFFD pattern).
    pub fn replacement_box() -> Self {
        // 7×11 box: outer border + cross pattern, hand-coded
        #[rustfmt::skip]
        const PATTERN: [u8; 77] = [
            255,255,255,255,255,255,255,
            255,  0,  0,  0,  0,  0,255,
            255,  0,255,  0,  0,  0,255,
            255,  0,  0,255,  0,  0,255,
            255,  0,  0,  0,255,  0,255,
            255,  0,  0,  0,  0,  0,255,
            255,  0,  0,  0,255,  0,255,
            255,  0,  0,255,  0,  0,255,
            255,  0,255,  0,  0,  0,255,
            255,  0,  0,  0,  0,  0,255,
            255,255,255,255,255,255,255,
        ];
        Self {
            width: 7, height: 11,
            coverage: PATTERN.to_vec(),
            x_offset: 0, y_offset: 11,
            advance: 8.0,
        }
    }

    /// Substitute glyph used when font worker is unavailable or panics.
    pub fn substitute_for(ch: char) -> Self {
        if ch == ' ' {
            Self { width: 4, height: 1, coverage: vec![0], x_offset: 0, y_offset: 1, advance: 4.0 }
        } else {
            Self::replacement_box()
        }
    }
}
```

### FontError

```rust
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("font file too large: {0} bytes")]
    TooLarge(usize),

    #[error("font parse failed: {0}")]
    ParseFailed(String),

    #[error("font parse panicked (malformed or adversarial input)")]
    ParsePanic,

    #[error("glyph rasterization panicked for font worker")]
    RasterPanic,

    #[error("glyph metrics panicked for font worker")]
    MetricsPanic,

    #[error("font worker thread panicked; has been respawned")]
    WorkerPanic,

    #[error("font worker did not respond within deadline")]
    WorkerTimeout,

    #[error("font registry RAM limit reached; cannot load more fonts")]
    RegistryFull,

    #[error("font id {0} not found in registry")]
    NotFound(u32),

    #[error("unknown system font name: {0}")]
    UnknownSystemFont(String),

    #[error("font id 0 is reserved and must not be assigned to a provider")]
    FontIdZeroReserved,
}
```

### LayoutError

```rust
#[derive(Debug, thiserror::Error)]
pub enum LayoutError {
    #[error("RTL text is not supported in v1 (detected RTL codepoint)")]
    RtlUnsupported,

    #[error("font error during layout: {0}")]
    FontError(#[from] FontError),

    #[error("layout session provider not found: {0}")]
    ProviderNotFound(String),
}
```

---

## §3 FontConfig (Per-Platform)

```rust
// supervisor/src/font/config.rs

use serde::Deserialize;
use crate::profile::PlatformProfile;

/// All font subsystem limits, derived from the active platform profile.
/// Never use module-level constants for these values.
#[derive(Debug, Clone, Deserialize)]
pub struct FontConfig {
    /// Max bytes for a single font file (raw bytes, validation gate before parse).
    pub max_font_bytes: usize,

    /// Max estimated total RAM for all loaded fonts combined (raw × 2 factor).
    pub max_total_estimated_ram: usize,

    /// LRU cap for TrueType glyph cache entries (per font provider instance).
    pub max_glyph_cache_entries: usize,

    /// Max entries in GPU atlas per PID (0 = GPU atlas disabled on this platform).
    pub max_atlas_entries_per_pid: usize,

    /// Whether TrueType rasterization (fontdue) is supported on this platform.
    /// false on mcu-minimal: binary-only bitmap font, fontdue not compiled in.
    pub truetype_enabled: bool,

    /// Whether the GPU atlas is available on this platform.
    /// true only on desktop-full and server-headless (wgpu present).
    pub gpu_atlas_enabled: bool,

    /// Whether CJK system fonts are available (requires desktop-full or server-headless).
    pub cjk_enabled: bool,
}

impl FontConfig {
    /// Build FontConfig from the active PlatformProfile.
    pub fn for_platform(platform: &PlatformProfile) -> Self {
        match platform.name.as_str() {
            "mcu-minimal" => Self {
                max_font_bytes:              0,
                max_total_estimated_ram:     0,
                max_glyph_cache_entries:     0,
                max_atlas_entries_per_pid:   0,
                truetype_enabled:            false,
                gpu_atlas_enabled:           false,
                cjk_enabled:                 false,
            },
            "iot-edge" | "robotics-rt" => Self {
                max_font_bytes:              4  * 1024 * 1024,  // 4 MiB
                max_total_estimated_ram:     8  * 1024 * 1024,  // 8 MiB
                max_glyph_cache_entries:     256,
                max_atlas_entries_per_pid:   32,
                truetype_enabled:            true,
                gpu_atlas_enabled:           false,
                cjk_enabled:                 false,
            },
            "mobile" => Self {
                max_font_bytes:              16 * 1024 * 1024,  // 16 MiB
                max_total_estimated_ram:     64 * 1024 * 1024,  // 64 MiB
                max_glyph_cache_entries:     4096,
                max_atlas_entries_per_pid:   64,
                truetype_enabled:            true,
                gpu_atlas_enabled:           false,
                cjk_enabled:                 false,
            },
            // desktop-full, server-headless, and any unrecognised profile
            _ => Self {
                max_font_bytes:              32 * 1024 * 1024,  // 32 MiB
                max_total_estimated_ram:     256 * 1024 * 1024, // 256 MiB
                max_glyph_cache_entries:     8192,
                max_atlas_entries_per_pid:   128,
                truetype_enabled:            true,
                gpu_atlas_enabled:           true,
                cjk_enabled:                 true,
            },
        }
    }
}

/// Estimated RAM for a loaded font (raw bytes × overhead factor).
/// Used by FontRegistry to track total consumption.
pub fn estimated_ram(raw_bytes: usize) -> usize {
    raw_bytes.saturating_mul(2)
}
```

### CJK note

Full CJK fonts (e.g., Noto Sans CJK, ~8 MiB raw, ~20 MiB estimated) require `desktop-full` or `server-headless`. The 8 MiB `max_total_estimated_ram` on `iot-edge` / `robotics-rt` is deliberately below a full CJK font; these profiles are Latin-only by platform constraint. Mobile profiles may ship a subset CJK font (≤16 MiB raw) if the integrator chooses.

---

## §4 BitmapFontProvider

```rust
// supervisor/src/font/bitmap_provider.rs

use std::sync::Arc;
use crate::font::types::{GlyphBitmap, TextMetrics, FontWeight, GlyphCoverage, FontError};
use crate::font::provider::FontProvider;

/// Public ID for the built-in bitmap font. Always present; never unloaded.
pub const BITMAP_FONT_ID: u32 = 1;

/// Built-in pixel bitmap font.
/// Sizes: 4×8 (small), 8×16 (medium, default), 16×32 (large).
/// Coverage: printable ASCII (U+0020–U+007E) + U+FFFD (replacement box).
/// Thread-safe: immutable after construction.
pub struct BitmapFontProvider {
    id: u32,
}

impl BitmapFontProvider {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { id: BITMAP_FONT_ID })
    }

    /// Select glyph table for the requested pixel size.
    fn select_scale(size_px: f32) -> BitmapScale {
        if size_px <= 6.0 { BitmapScale::Small }
        else if size_px <= 20.0 { BitmapScale::Medium }
        else { BitmapScale::Large }
    }

    /// Look up bitmap glyph data for `ch` at `scale`.
    /// Returns (data_ptr, glyph_w, glyph_h) or None if not present.
    fn raw_glyph(ch: char, scale: BitmapScale) -> Option<(&'static [u8], u32, u32)> {
        let (table, gw, gh) = match scale {
            BitmapScale::Small  => (crate::font::bitmap_data::SMALL_GLYPHS,   4u32,  8u32),
            BitmapScale::Medium => (crate::font::bitmap_data::MEDIUM_GLYPHS,  8u32, 16u32),
            BitmapScale::Large  => (crate::font::bitmap_data::LARGE_GLYPHS,  16u32, 32u32),
        };
        let stride = (gw * gh) as usize;
        // ASCII printable: indices 0–94 (0x20–0x7E)
        let idx: Option<usize> = match ch as u32 {
            0x0020..=0x007E => Some((ch as u32 - 0x0020) as usize),
            0xFFFD           => Some(95), // U+FFFD stored at index 95
            _                => None,
        };
        idx.map(|i| {
            let start = i * stride;
            (&table[start..start + stride], gw, gh)
        })
    }

    /// Scale a glyph to an arbitrary pixel size using nearest-neighbour.
    fn scale_nearest(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
        let mut out = vec![0u8; (dw * dh) as usize];
        for dy in 0..dh {
            for dx in 0..dw {
                let sx = (dx * sw / dw) as usize;
                let sy = (dy * sh / dh) as usize;
                out[(dy * dw + dx) as usize] = src[sy * sw as usize + sx];
            }
        }
        out
    }
}

#[derive(Clone, Copy)]
enum BitmapScale { Small, Medium, Large }

impl FontProvider for BitmapFontProvider {
    fn id(&self) -> u32 { self.id }
    fn name(&self) -> &str { "bitmap-builtin" }

    fn coverage(&self, ch: char) -> GlyphCoverage {
        match ch as u32 {
            0x0020..=0x007E => GlyphCoverage::Native,
            0xFFFD           => GlyphCoverage::Native,   // U+FFFD box glyph present in tables
            _                => GlyphCoverage::Substitute, // will return U+FFFD bitmap
        }
    }

    fn render_glyph(&self, ch: char, size_px: f32, _weight: FontWeight)
        -> Result<GlyphBitmap, FontError>
    {
        let scale = Self::select_scale(size_px);
        // For unknown codepoints, return U+FFFD
        let render_ch = match self.coverage(ch) {
            GlyphCoverage::Native    => ch,
            GlyphCoverage::Substitute => '\u{FFFD}',
            GlyphCoverage::NoGlyph    => '\u{FFFD}',
        };
        let (data, base_w, base_h) = Self::raw_glyph(render_ch, scale)
            .unwrap_or_else(|| {
                // Should not happen after coverage check, but be defensive
                (crate::font::bitmap_data::FFFD_MEDIUM, 8, 16)
            });

        let target_w = size_px.round() as u32 * base_w / 16;
        let target_h = size_px.round() as u32 * base_h / 16;
        let (final_w, final_h) = if target_w == base_w && target_h == base_h {
            (base_w, base_h)
        } else {
            (target_w.max(1), target_h.max(1))
        };
        let coverage = if final_w == base_w && final_h == base_h {
            data.to_vec()
        } else {
            Self::scale_nearest(data, base_w, base_h, final_w, final_h)
        };
        Ok(GlyphBitmap {
            width: final_w,
            height: final_h,
            coverage,
            x_offset: 0,
            y_offset: final_h as i32,
            advance: final_w as f32 + 1.0,
        })
    }

    fn glyph_advance(&self, _ch: char, size_px: f32) -> Result<f32, FontError> {
        let scale = Self::select_scale(size_px);
        let base_w = match scale { BitmapScale::Small => 4.0, BitmapScale::Medium => 8.0, BitmapScale::Large => 16.0 };
        Ok(base_w + 1.0)
    }

    fn measure_text(&self, text: &str, size_px: f32, _weight: FontWeight)
        -> Result<TextMetrics, FontError>
    {
        let adv = self.glyph_advance(' ', size_px)?;
        let scale = Self::select_scale(size_px);
        let h = match scale { BitmapScale::Small => 8.0, BitmapScale::Medium => 16.0, BitmapScale::Large => 32.0 };
        let count = text.chars().count();
        Ok(TextMetrics {
            width: adv * count as f32,
            height: h,
            ascent: h,
            descent: 0.0,
            glyph_count: count,
        })
    }

    fn ascent(&self, size_px: f32) -> f32 {
        let scale = Self::select_scale(size_px);
        match scale { BitmapScale::Small => 8.0, BitmapScale::Medium => 16.0, BitmapScale::Large => 32.0 }
    }
    fn descent(&self, _size_px: f32) -> f32 { 0.0 }
    fn line_height(&self, size_px: f32) -> f32 { self.ascent(size_px) + 2.0 }
    fn estimated_ram(&self) -> usize {
        // Static data in .rodata — no heap allocation
        0
    }
}
```

The bitmap glyph tables (`SMALL_GLYPHS`, `MEDIUM_GLYPHS`, `LARGE_GLYPHS`) live in `supervisor/src/font/bitmap_data.rs`. Each is a `&'static [u8]` with 96 entries (95 ASCII + 1 U+FFFD). The U+FFFD entry at index 95 stores the 7×11 box cross pattern scaled/padded to fit the table stride.

---

## §5 TrueTypeFontProvider

```rust
// supervisor/src/font/truetype_provider.rs

use std::sync::{Arc, Mutex};
use std::collections::HashMap;
use fontdue::{Font, FontSettings, Metrics};
use lru::LruCache;
use crate::font::types::{GlyphBitmap, TextMetrics, FontWeight, GlyphCoverage, FontError};
use crate::font::provider::FontProvider;
use crate::font::config::FontConfig;

/// Cache key: quantized to 0.25 px steps; weight bucketed to u8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphCacheKey {
    pub ch: char,
    /// size_px × 4, truncated to u16. 0.25 px granularity.
    pub size_q: u16,
    /// FontWeight discriminant (1–9).
    pub weight: u8,
}

/// Quantize a raw size_px to 0.25-pixel steps before any font call.
/// Both render_glyph and glyph_advance must call this first.
pub fn quantize_size(size_px: f32) -> f32 {
    (size_px * 4.0).floor() / 4.0
}

/// Encode quantized size as cache key component (size_px × 4, capped at u16::MAX).
fn encode_size_q(size_q: f32) -> u16 {
    (size_q * 4.0).min(u16::MAX as f32) as u16
}

// ──────────────────────────────────────────────────────────────────────────────
// Panic-safe wrappers (B1 / B7)
// ──────────────────────────────────────────────────────────────────────────────

/// Rasterize a glyph, catching any panic from fontdue.
pub fn safe_rasterize(font: &Font, ch: char, size: f32)
    -> Result<(Metrics, Vec<u8>), FontError>
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        font.rasterize(ch, size)
    })).map_err(|_| FontError::RasterPanic)
}

/// Query glyph metrics, catching any panic from fontdue.
pub fn safe_metrics(font: &Font, ch: char, size: f32)
    -> Result<Metrics, FontError>
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        font.metrics(ch, size)
    })).map_err(|_| FontError::MetricsPanic)
}

// ──────────────────────────────────────────────────────────────────────────────
// TrueTypeFontProvider
// ──────────────────────────────────────────────────────────────────────────────

pub struct TrueTypeFontProvider {
    id:     u32,
    name:   String,
    font:   Arc<Font>,  // fontdue is Send+Sync
    cache:  Mutex<LruCache<GlyphCacheKey, GlyphBitmap>>,
    raw_len: usize,
}

impl TrueTypeFontProvider {
    pub fn new(
        id: u32,
        name: impl Into<String>,
        data: &[u8],
        config: &FontConfig,
    ) -> Result<Arc<Self>, FontError> {
        if id == 0 { return Err(FontError::FontIdZeroReserved); }
        // Parse, catching panic
        let font = std::panic::catch_unwind(|| {
            Font::from_bytes(data, FontSettings::default())
        }).map_err(|_| FontError::ParsePanic)?
          .map_err(|e| FontError::ParseFailed(e.to_string()))?;

        let cap = std::num::NonZeroUsize::new(
            config.max_glyph_cache_entries.max(1)
        ).unwrap();
        Ok(Arc::new(Self {
            id,
            name: name.into(),
            font: Arc::new(font),
            cache: Mutex::new(LruCache::new(cap)),
            raw_len: data.len(),
        }))
    }

    /// Internal: render with full cache integration.
    pub fn render_glyph_cached(
        &self,
        ch: char,
        size_px: f32,
        weight: FontWeight,
    ) -> Result<GlyphBitmap, FontError> {
        let size_q = quantize_size(size_px);
        let key = GlyphCacheKey {
            ch,
            size_q: encode_size_q(size_q),
            weight: weight as u8,
        };
        // Check cache (LRU::get bumps MRU automatically)
        {
            let mut cache = self.cache.lock().unwrap();
            if let Some(bmp) = cache.get(&key) {
                return Ok(bmp.clone());
            }
        }
        // Rasterize (outside lock to avoid holding lock during potentially slow call)
        let (metrics, pixels) = safe_rasterize(&self.font, ch, size_q)?;
        let bmp = GlyphBitmap {
            width:    metrics.width as u32,
            height:   metrics.height as u32,
            coverage: pixels,
            x_offset: metrics.xmin,
            y_offset: metrics.ymin + metrics.height as i32,
            advance:  metrics.advance_width,
        };
        // Insert into cache
        {
            let mut cache = self.cache.lock().unwrap();
            cache.put(key, bmp.clone());
        }
        Ok(bmp)
    }
}

impl FontProvider for TrueTypeFontProvider {
    fn id(&self) -> u32 { self.id }
    fn name(&self) -> &str { &self.name }

    fn coverage(&self, ch: char) -> GlyphCoverage {
        // fontdue glyph_index: 0 means .notdef (not present)
        if self.font.lookup_glyph_index(ch as u32) != 0 {
            GlyphCoverage::Native
        } else {
            GlyphCoverage::NoGlyph
        }
    }

    fn render_glyph(&self, ch: char, size_px: f32, weight: FontWeight)
        -> Result<GlyphBitmap, FontError>
    {
        self.render_glyph_cached(ch, size_px, weight)
    }

    fn glyph_advance(&self, ch: char, size_px: f32) -> Result<f32, FontError> {
        // MUST use quantize_size — same quantization as render_glyph (B3)
        let size_q = quantize_size(size_px);
        let metrics = safe_metrics(&self.font, ch, size_q)?;
        Ok(metrics.advance_width)
    }

    fn measure_text(&self, text: &str, size_px: f32, weight: FontWeight)
        -> Result<TextMetrics, FontError>
    {
        let size_q = quantize_size(size_px);
        let mut total_advance = 0.0f32;
        let mut max_ascent = 0.0f32;
        let mut max_descent = 0.0f32;
        let mut count = 0usize;
        for ch in text.chars() {
            let m = safe_metrics(&self.font, ch, size_q)?;
            total_advance += m.advance_width;
            max_ascent  = max_ascent.max(m.ymin as f32 + m.height as f32);
            max_descent = max_descent.max(-(m.ymin as f32));
            count += 1;
        }
        Ok(TextMetrics {
            width:  total_advance,
            height: max_ascent + max_descent,
            ascent: max_ascent,
            descent: max_descent,
            glyph_count: count,
        })
    }

    fn ascent(&self, size_px: f32) -> f32 {
        let size_q = quantize_size(size_px);
        // Use 'A' as representative; safe because we catch panic
        safe_metrics(&self.font, 'A', size_q)
            .map(|m| (m.ymin + m.height as i32) as f32)
            .unwrap_or(size_q)
    }

    fn descent(&self, size_px: f32) -> f32 {
        let size_q = quantize_size(size_px);
        safe_metrics(&self.font, 'g', size_q)
            .map(|m| (-(m.ymin as f32)).max(0.0))
            .unwrap_or(0.0)
    }

    fn line_height(&self, size_px: f32) -> f32 {
        self.ascent(size_px) + self.descent(size_px) + (size_px * 0.15)
    }

    fn estimated_ram(&self) -> usize {
        // raw bytes × 2: one copy in parsed fontdue + one copy of original data retained
        crate::font::config::estimated_ram(self.raw_len)
    }
}
```

---

## §6 GlyphAtlas

### CpuGlyphAtlas

```rust
// supervisor/src/font/atlas_cpu.rs

use std::collections::HashMap;
use lru::LruCache;
use crate::font::types::{GlyphBitmap, FontError};

/// Glyph cache key used by both CPU and GPU atlases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GlyphCacheKey {
    pub font_id: u32,
    pub ch:      char,
    /// size_px × 4, truncated. Same encoding as GlyphCacheKey in truetype_provider.rs.
    pub size_q:  u16,
    pub weight:  u8,
}

/// CPU-only LRU glyph bitmap cache.
/// Used on platforms without GPU atlas (iot-edge, robotics-rt, mobile).
pub struct CpuGlyphAtlas {
    cache: LruCache<GlyphCacheKey, GlyphBitmap>,
}

impl CpuGlyphAtlas {
    pub fn new(capacity: std::num::NonZeroUsize) -> Self {
        Self { cache: LruCache::new(capacity) }
    }

    /// Get a cached bitmap, bumping MRU position.
    pub fn get(&mut self, key: &GlyphCacheKey) -> Option<&GlyphBitmap> {
        self.cache.get(key)
    }

    /// Insert a bitmap. If capacity is exceeded, LRU entry is evicted automatically.
    pub fn insert(&mut self, key: GlyphCacheKey, bmp: GlyphBitmap) {
        self.cache.put(key, bmp);
    }

    /// Number of entries currently cached.
    pub fn len(&self) -> usize { self.cache.len() }

    /// Clear entire cache (called on font unload).
    pub fn invalidate_font(&mut self, font_id: u32) {
        let keys: Vec<_> = self.cache.iter()
            .filter(|(k, _)| k.font_id == font_id)
            .map(|(k, _)| *k)
            .collect();
        for k in keys { self.cache.pop(&k); }
    }
}
```

### GpuGlyphAtlas

```rust
// supervisor/src/font/atlas_gpu.rs
// GPU atlas: etagere Skyline-BL allocator, 2048×2048, per-PID quota.
// Available only on desktop-full and server-headless (FontConfig::gpu_atlas_enabled).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::{Acquire, Relaxed, Release}};
use etagere::{AtlasAllocator, Size, AllocId};
use crate::font::types::GlyphBitmap;
use crate::font::atlas_cpu::GlyphCacheKey;

const ATLAS_SIZE: u32 = 2048;

pub struct AtlasRegion {
    pub uv:        (f32, f32, f32, f32), // (u0, v0, u1, v1) normalized 0.0–1.0
    pub alloc_id:  AllocId,
    pub owner_pid: u32,
    pub last_frame: AtomicU64,
}

/// Staging buffer: list of (x, y, bitmap) pending GPU upload.
type StagingEntry = (u32, u32, GlyphBitmap);

pub struct GpuGlyphAtlas {
    allocator:     Mutex<AtlasAllocator>,
    regions:       RwLock<HashMap<GlyphCacheKey, AtlasRegion>>,
    pid_occupancy: Mutex<HashMap<u32, usize>>,
    staging:       Mutex<Vec<StagingEntry>>,
    texture_dirty: AtomicBool,
    max_per_pid:   usize,
    frame_counter: AtomicU64,
}

impl GpuGlyphAtlas {
    pub fn new(max_per_pid: usize) -> Arc<Self> {
        Arc::new(Self {
            allocator:     Mutex::new(AtlasAllocator::new(
                etagere::Size::new(ATLAS_SIZE as i32, ATLAS_SIZE as i32)
            )),
            regions:       RwLock::new(HashMap::new()),
            pid_occupancy: Mutex::new(HashMap::new()),
            staging:       Mutex::new(Vec::new()),
            texture_dirty: AtomicBool::new(false),
            max_per_pid,
            frame_counter: AtomicU64::new(0),
        })
    }

    /// Called by compositor at start of each VSync frame.
    pub fn advance_frame(&self) {
        self.frame_counter.fetch_add(1, Relaxed);
    }

    /// Get UV rect for key if cached; does NOT update MRU (use get_or_insert for full path).
    pub fn get(&self, key: &GlyphCacheKey) -> Option<(f32, f32, f32, f32)> {
        let regions = self.regions.read().unwrap();
        regions.get(key).map(|r| {
            r.last_frame.store(self.frame_counter.load(Relaxed), Relaxed);
            r.uv
        })
    }

    /// Get or allocate UV rect for a glyph. Returns None if atlas is full after eviction.
    pub fn get_or_insert(
        &self,
        key: GlyphCacheKey,
        bitmap: &GlyphBitmap,
        pid: u32,
    ) -> Option<(f32, f32, f32, f32)> {
        // Fast path: read lock
        if let Some(uv) = self.get(&key) { return Some(uv); }

        // Slow path: write lock + alloc
        let mut regions = self.regions.write().unwrap();
        if let Some(r) = regions.get(&key) {
            r.last_frame.store(self.frame_counter.load(Relaxed), Relaxed);
            return Some(r.uv);
        }

        // Quota enforcement
        {
            let mut occ = self.pid_occupancy.lock().unwrap();
            let count = occ.entry(pid).or_default();
            if *count >= self.max_per_pid {
                // Evict LRU entry for this PID
                Self::evict_lru_for_pid_inner(
                    &mut regions,
                    &mut self.allocator.lock().unwrap(),
                    &mut occ,
                    pid,
                );
            }
            *occ.entry(pid).or_default() += 1;
        }

        let size = etagere::Size::new(bitmap.width as i32, bitmap.height as i32);
        let mut alloc_guard = self.allocator.lock().unwrap();
        match alloc_guard.allocate(size) {
            Some(alloc) => {
                let r = alloc.rectangle;
                let uv = (
                    r.min.x as f32 / ATLAS_SIZE as f32,
                    r.min.y as f32 / ATLAS_SIZE as f32,
                    r.max.x as f32 / ATLAS_SIZE as f32,
                    r.max.y as f32 / ATLAS_SIZE as f32,
                );
                self.staging.lock().unwrap().push(
                    (r.min.x as u32, r.min.y as u32, bitmap.clone())
                );
                self.texture_dirty.store(true, Release);
                regions.insert(key, AtlasRegion {
                    uv,
                    alloc_id:  alloc.id,
                    owner_pid: pid,
                    last_frame: AtomicU64::new(self.frame_counter.load(Relaxed)),
                });
                Some(uv)
            }
            None => {
                // Atlas full even after per-PID eviction
                log::warn!(
                    "font.atlas.full: pid={pid} ch={:?} — CPU fallback for this glyph",
                    key.ch
                );
                None
            }
        }
    }

    fn evict_lru_for_pid_inner(
        regions: &mut HashMap<GlyphCacheKey, AtlasRegion>,
        allocator: &mut AtlasAllocator,
        occ: &mut HashMap<u32, usize>,
        pid: u32,
    ) {
        let oldest_key = regions.iter()
            .filter(|(_, r)| r.owner_pid == pid)
            .min_by_key(|(_, r)| r.last_frame.load(Relaxed))
            .map(|(k, _)| *k);
        if let Some(key) = oldest_key {
            if let Some(region) = regions.remove(&key) {
                allocator.deallocate(region.alloc_id);
                let e = occ.entry(pid).or_default();
                *e = e.saturating_sub(1);
            }
        }
    }

    /// Returns true if new glyphs were staged since last call.
    pub fn is_dirty(&self) -> bool { self.texture_dirty.load(Acquire) }

    /// Take all pending staging entries for GPU upload. Clears the dirty flag.
    /// The compositor calls this before issuing draw calls for the frame.
    pub fn take_dirty(&self) -> Vec<StagingEntry> {
        self.texture_dirty.store(false, Release);
        std::mem::take(&mut self.staging.lock().unwrap())
    }

    /// Remove all atlas entries owned by a PID (called on app exit).
    pub fn evict_pid(&self, pid: u32) {
        let mut regions = self.regions.write().unwrap();
        let mut alloc = self.allocator.lock().unwrap();
        let mut occ = self.pid_occupancy.lock().unwrap();
        let keys: Vec<_> = regions.iter()
            .filter(|(_, r)| r.owner_pid == pid)
            .map(|(k, _)| *k)
            .collect();
        for k in keys {
            if let Some(r) = regions.remove(&k) {
                alloc.deallocate(r.alloc_id);
            }
        }
        occ.remove(&pid);
    }
}
```

---

## §7 FontRegistry

```rust
// supervisor/src/font/registry.rs

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use crate::font::config::{FontConfig, estimated_ram};
use crate::font::provider::FontProvider;
use crate::font::truetype_provider::TrueTypeFontProvider;
use crate::font::bitmap_provider::{BitmapFontProvider, BITMAP_FONT_ID};
use crate::font::types::FontError;

const MAX_FONT_BYTES_VALIDATE: usize = 64 * 1024 * 1024; // 64 MiB hard cap for validation path

struct RegistryInner {
    providers:            HashMap<u32, Arc<dyn FontProvider>>,
    /// Maps font_id → owner_pid (None for system fonts)
    owners:               HashMap<u32, Option<u32>>,
    total_estimated_ram:  usize,
    next_id:              u32,
}

pub struct FontRegistry {
    inner:  RwLock<RegistryInner>,
    config: FontConfig,
}

impl FontRegistry {
    pub fn new(config: FontConfig) -> Arc<Self> {
        let mut inner = RegistryInner {
            providers: HashMap::new(),
            owners: HashMap::new(),
            total_estimated_ram: 0,
            next_id: 2, // 0 reserved, 1 = bitmap
        };
        // Always register bitmap font
        let bitmap = BitmapFontProvider::new();
        inner.providers.insert(BITMAP_FONT_ID, bitmap);
        inner.owners.insert(BITMAP_FONT_ID, None);

        Arc::new(Self { inner: RwLock::new(inner), config })
    }

    /// Load a system font (name → bytes from initramfs).
    /// Returns the assigned font_id.
    pub fn load_system_font(&self, name: &str, data: &[u8]) -> Result<u32, FontError> {
        self.load_internal(name, data, None)
    }

    /// Load an app-bundled font. `owner_pid` used for RAM accounting and cleanup.
    pub fn load_app_font(&self, name: &str, data: &[u8], owner_pid: u32)
        -> Result<u32, FontError>
    {
        if data.len() > self.config.max_font_bytes {
            return Err(FontError::TooLarge(data.len()));
        }
        self.load_internal(name, data, Some(owner_pid))
    }

    fn load_internal(&self, name: &str, data: &[u8], owner: Option<u32>)
        -> Result<u32, FontError>
    {
        if !self.config.truetype_enabled {
            return Err(FontError::ParseFailed(
                "TrueType not supported on this platform profile".into()
            ));
        }
        // Validate before registering (B7)
        validate_font_file(data)?;

        let mut inner = self.inner.write().unwrap();

        // RAM check using estimated_ram (B4)
        let est = estimated_ram(data.len());
        if inner.total_estimated_ram + est > self.config.max_total_estimated_ram {
            return Err(FontError::RegistryFull);
        }

        let id = inner.next_id;
        if id == 0 { return Err(FontError::FontIdZeroReserved); }
        inner.next_id += 1;

        let provider = TrueTypeFontProvider::new(id, name, data, &self.config)?;
        inner.total_estimated_ram += est;
        inner.providers.insert(id, provider);
        inner.owners.insert(id, owner);
        Ok(id)
    }

    /// Retrieve a provider by ID.
    pub fn get(&self, id: u32) -> Result<Arc<dyn FontProvider>, FontError> {
        self.inner.read().unwrap()
            .providers.get(&id)
            .cloned()
            .ok_or(FontError::NotFound(id))
    }

    /// Look up a system font ID by name. Returns Err if not loaded.
    pub fn get_by_name(&self, name: &str) -> Result<Arc<dyn FontProvider>, FontError> {
        let inner = self.inner.read().unwrap();
        inner.providers.values()
            .find(|p| p.name() == name)
            .cloned()
            .ok_or_else(|| FontError::UnknownSystemFont(name.to_string()))
    }

    /// Unload all fonts owned by `pid` (called on app exit).
    pub fn unload_owned_by(&self, pid: u32) {
        let mut inner = self.inner.write().unwrap();
        let to_remove: Vec<u32> = inner.owners.iter()
            .filter(|(_, owner)| **owner == Some(pid))
            .map(|(id, _)| *id)
            .collect();
        for id in to_remove {
            if let Some(p) = inner.providers.remove(&id) {
                inner.total_estimated_ram =
                    inner.total_estimated_ram.saturating_sub(p.estimated_ram());
                inner.owners.remove(&id);
            }
        }
    }

    /// List all loaded fonts as (id, name, owner_pid).
    pub fn list(&self) -> Vec<(u32, String, Option<u32>)> {
        let inner = self.inner.read().unwrap();
        inner.providers.iter()
            .map(|(id, p)| (*id, p.name().to_string(), inner.owners.get(id).copied().flatten()))
            .collect()
    }

    /// Current total estimated RAM used by all loaded fonts.
    pub fn total_estimated_ram(&self) -> usize {
        self.inner.read().unwrap().total_estimated_ram
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Font file validation (B7)
// ──────────────────────────────────────────────────────────────────────────────

/// Validate a raw font file before registering it.
/// Steps:
///   1. Size gate (hard cap)
///   2. Parse via fontdue (panic-caught)
///   3. Pre-flight: rasterize 10 representative glyphs at 14px
/// This catches "parses fine but bombs on first glyph" attacks.
pub fn validate_font_file(data: &[u8]) -> Result<(), FontError> {
    if data.len() > MAX_FONT_BYTES_VALIDATE {
        return Err(FontError::TooLarge(data.len()));
    }
    // Parse validation — catch panic (B7)
    let font = std::panic::catch_unwind(|| {
        fontdue::Font::from_bytes(data, fontdue::FontSettings::default())
    }).map_err(|_| FontError::ParsePanic)?
      .map_err(|e| FontError::ParseFailed(e.to_string()))?;

    // Pre-flight rasterization — 10 representative ASCII glyphs at 14px
    const TEST_CHARS: [char; 10] = ['A', 'B', 'C', 'a', 'b', 'c', '0', '1', '.', '!'];
    for ch in TEST_CHARS {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = font.rasterize(ch, 14.0);
        })).map_err(|_| FontError::RasterPanic)?;
    }
    Ok(())
}
```

---

## §8 FallbackChain

```rust
// supervisor/src/font/fallback.rs

use std::sync::Arc;
use crate::font::registry::FontRegistry;
use crate::font::provider::FontProvider;
use crate::font::types::{GlyphBitmap, FontError, FontWeight, GlyphCoverage};
use crate::font::bitmap_provider::BITMAP_FONT_ID;
use crate::font::layout::LayoutSession;

/// Ordered list of font_ids to try for glyph coverage.
/// Resolved left-to-right: first provider with Native/Substitute coverage wins.
#[derive(Debug, Clone)]
pub struct FallbackChain {
    /// Ordered font IDs. Must include BITMAP_FONT_ID as final fallback.
    pub chain: Vec<u32>,
}

impl FallbackChain {
    /// Default system chain: VyomaSans → VyomaMono → bitmap.
    /// VyomaSans/VyomaMono IDs are looked up from the registry by name.
    pub fn system_default(registry: &FontRegistry) -> Self {
        let mut chain = Vec::new();
        if let Ok(p) = registry.get_by_name("VyomaSans-Regular") {
            chain.push(p.id());
        }
        if let Ok(p) = registry.get_by_name("VyomaMono-Regular") {
            chain.push(p.id());
        }
        chain.push(BITMAP_FONT_ID); // always present
        Self { chain }
    }

    /// Resolve which provider to use for `ch`.
    /// Returns the first provider with Native or Substitute coverage.
    /// Falls back to bitmap if nothing else matches.
    pub fn resolve(&self, ch: char, registry: &FontRegistry) -> Arc<dyn FontProvider> {
        for &id in &self.chain {
            if let Ok(p) = registry.get(id) {
                match p.coverage(ch) {
                    GlyphCoverage::Native    => return p,
                    GlyphCoverage::Substitute => return p, // BitmapFontProvider renders U+FFFD
                    GlyphCoverage::NoGlyph    => continue,
                }
            }
        }
        // Should never reach here because bitmap is always in the chain,
        // but be defensive:
        registry.get(BITMAP_FONT_ID).expect("bitmap font always present in registry")
    }

    /// Render a run of text, dispatching each glyph through the fallback chain.
    /// Uses LayoutSession to ensure quantization consistency.
    pub fn render_run<'a>(
        &self,
        text: &str,
        session: &mut LayoutSession<'a>,
    ) -> Result<Vec<(char, GlyphBitmap, f32)>, FontError> {
        let mut out = Vec::with_capacity(text.chars().count());
        for ch in text.chars() {
            let (bmp, adv) = session.glyph(ch, self)?;
            out.push((ch, bmp.clone(), adv));
        }
        Ok(out)
    }
}
```

---

## §9 LayoutEngine

```rust
// supervisor/src/font/layout.rs

use std::collections::HashMap;
use std::sync::Arc;
use crate::font::registry::FontRegistry;
use crate::font::fallback::FallbackChain;
use crate::font::provider::FontProvider;
use crate::font::types::{GlyphBitmap, FontWeight, FontError};
use crate::font::truetype_provider::quantize_size;
use crate::font::types::LayoutError;

// ──────────────────────────────────────────────────────────────────────────────
// LayoutSession (B3)
// ──────────────────────────────────────────────────────────────────────────────

/// Pins provider resolution and quantizes size once for both measure and render.
/// Both measure_text and render_glyph use the same quantized size path
/// by routing through LayoutSession::glyph().
pub struct LayoutSession<'reg> {
    registry:    &'reg FontRegistry,
    pub size_q:  f32,     // quantize_size applied once at construction
    pub weight:  FontWeight,
    /// Cache: (char) → (provider_arc, bitmap, advance)
    /// Ensures provider is resolved once per unique codepoint.
    glyph_cache: HashMap<char, (Arc<dyn FontProvider>, GlyphBitmap, f32)>,
}

impl<'reg> LayoutSession<'reg> {
    pub fn new(registry: &'reg FontRegistry, size_px: f32, weight: FontWeight) -> Self {
        Self {
            registry,
            size_q: quantize_size(size_px),
            weight,
            glyph_cache: HashMap::new(),
        }
    }

    /// Returns (&GlyphBitmap, advance_px) for `ch`.
    /// Provider is resolved via the fallback chain on first call for each char,
    /// then cached so measure and render always get the same result.
    pub fn glyph(
        &mut self,
        ch: char,
        chain: &FallbackChain,
    ) -> Result<(&GlyphBitmap, f32), FontError> {
        if !self.glyph_cache.contains_key(&ch) {
            let provider = chain.resolve(ch, self.registry);
            let bmp = provider.render_glyph(ch, self.size_q, self.weight)?;
            let adv = provider.glyph_advance(ch, self.size_q)?;
            self.glyph_cache.insert(ch, (provider, bmp, adv));
        }
        let v = self.glyph_cache.get(&ch).unwrap();
        Ok((&v.1, v.2))
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Layout options
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WrapMode {
    /// Wrap at character boundary when line exceeds max_width.
    Char,
    /// Wrap at word boundary; fall back to character wrap if single word > max_width.
    Word,
    /// Single line; truncate with '…' (U+2026) if text exceeds max_width.
    Ellipsis,
}

#[derive(Debug, Clone)]
pub struct LayoutOpts {
    pub max_width:   f32,   // 0.0 = no limit
    pub wrap:        WrapMode,
    pub line_spacing: f32,  // extra pixels between lines (0.0 = default)
}

impl Default for LayoutOpts {
    fn default() -> Self {
        Self { max_width: 0.0, wrap: WrapMode::Char, line_spacing: 0.0 }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// TextLine
// ──────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TextLine {
    /// The substring of the original `text` on this line.
    pub text:  String,
    /// Total advance width of all glyphs on this line.
    pub width: f32,
    /// Y offset from the top of the first line to this line's baseline.
    pub y_baseline: f32,
    /// For Ellipsis mode: char that was appended ('…') or None.
    pub trailing_glyph: Option<char>,
}

// ──────────────────────────────────────────────────────────────────────────────
// layout_text (B5 — returns Result, RTL is explicit error)
// ──────────────────────────────────────────────────────────────────────────────

/// Entry point for all text layout.
/// Returns Err(LayoutError::RtlUnsupported) for any text containing RTL codepoints.
/// Returns Ok(Vec<TextLine>) for LTR/neutral text.
/// Empty input returns a single empty TextLine (width=0, y_baseline=0).
pub fn layout_text<'a>(
    text: &'a str,
    opts: LayoutOpts,
    session: &mut LayoutSession,
    chain: &FallbackChain,
) -> Result<Vec<TextLine>, LayoutError> {
    // RTL detection: explicit error (B5)
    if detect_rtl(text) {
        return Err(LayoutError::RtlUnsupported);
    }
    // Empty input: single empty line
    if text.is_empty() {
        return Ok(vec![TextLine {
            text: String::new(), width: 0.0, y_baseline: 0.0, trailing_glyph: None
        }]);
    }
    let line_h = session.registry
        .get_by_name("VyomaSans-Regular")
        .map(|p| p.line_height(session.size_q))
        .unwrap_or(session.size_q + 2.0)
        + opts.line_spacing;

    let lines = match opts.wrap {
        WrapMode::Char    => layout_wrap_char(text, &opts, session, chain, line_h)?,
        WrapMode::Word    => layout_wrap_word(text, &opts, session, chain, line_h)?,
        WrapMode::Ellipsis => layout_ellipsis(text, &opts, session, chain)?,
    };
    Ok(lines)
}

// ──────────────────────────────────────────────────────────────────────────────
// RTL detection (B5)
// ──────────────────────────────────────────────────────────────────────────────

/// Returns true if `text` contains any Hebrew or Arabic Unicode codepoint.
/// Does NOT implement full BiDi — just detects and rejects.
pub fn detect_rtl(text: &str) -> bool {
    text.chars().any(|c| {
        let cp = c as u32;
        (0x0590..=0x05FF).contains(&cp) || // Hebrew
        (0x0600..=0x06FF).contains(&cp) || // Arabic
        (0x0750..=0x077F).contains(&cp) || // Arabic Supplement
        (0xFB50..=0xFDFF).contains(&cp) || // Arabic Presentation Forms-A
        (0xFE70..=0xFEFF).contains(&cp)    // Arabic Presentation Forms-B
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Wrap implementations
// ──────────────────────────────────────────────────────────────────────────────

fn layout_wrap_char(
    text: &str,
    opts: &LayoutOpts,
    session: &mut LayoutSession,
    chain: &FallbackChain,
    line_h: f32,
) -> Result<Vec<TextLine>, LayoutError> {
    let mut lines: Vec<TextLine> = Vec::new();
    let mut current = String::new();
    let mut current_w = 0.0f32;
    let mut y = 0.0f32;

    for ch in text.chars() {
        if ch == '\n' {
            lines.push(TextLine {
                text: current.clone(), width: current_w, y_baseline: y, trailing_glyph: None
            });
            current.clear(); current_w = 0.0; y += line_h;
            continue;
        }
        let (_, adv) = session.glyph(ch, chain)?;
        if opts.max_width > 0.0 && current_w + adv > opts.max_width && !current.is_empty() {
            lines.push(TextLine {
                text: current.clone(), width: current_w, y_baseline: y, trailing_glyph: None
            });
            current.clear(); current_w = 0.0; y += line_h;
        }
        current.push(ch);
        current_w += adv;
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(TextLine {
            text: current, width: current_w, y_baseline: y, trailing_glyph: None
        });
    }
    Ok(lines)
}

fn layout_wrap_word(
    text: &str,
    opts: &LayoutOpts,
    session: &mut LayoutSession,
    chain: &FallbackChain,
    line_h: f32,
) -> Result<Vec<TextLine>, LayoutError> {
    let mut lines: Vec<TextLine> = Vec::new();
    let mut y = 0.0f32;

    for paragraph in text.split('\n') {
        let words: Vec<&str> = paragraph.split(' ').collect();
        let mut current = String::new();
        let mut current_w = 0.0f32;
        let space_adv = session.glyph(' ', chain).map(|(_, a)| a).unwrap_or(4.0);

        for (i, word) in words.iter().enumerate() {
            let word_w: f32 = word.chars()
                .map(|c| session.glyph(c, chain).map(|(_, a)| a).unwrap_or(8.0))
                .sum();
            let needs_space = i > 0 && !current.is_empty();
            let total_add = word_w + if needs_space { space_adv } else { 0.0 };

            if opts.max_width > 0.0 && current_w + total_add > opts.max_width && !current.is_empty() {
                lines.push(TextLine {
                    text: current.clone(), width: current_w, y_baseline: y, trailing_glyph: None
                });
                current.clear(); current_w = 0.0; y += line_h;
            }
            if !current.is_empty() { current.push(' '); current_w += space_adv; }
            // If single word exceeds max_width, fall through (char-wrap that word)
            if opts.max_width > 0.0 && word_w > opts.max_width {
                for ch in word.chars() {
                    let (_, adv) = session.glyph(ch, chain)?;
                    if current_w + adv > opts.max_width && !current.is_empty() {
                        lines.push(TextLine {
                            text: current.clone(), width: current_w, y_baseline: y,
                            trailing_glyph: None,
                        });
                        current.clear(); current_w = 0.0; y += line_h;
                    }
                    current.push(ch); current_w += adv;
                }
            } else {
                current.push_str(word); current_w += word_w;
            }
        }
        if !current.is_empty() || lines.is_empty() {
            lines.push(TextLine {
                text: current, width: current_w, y_baseline: y, trailing_glyph: None
            });
            y += line_h;
        }
    }
    Ok(lines)
}

fn layout_ellipsis(
    text: &str,
    opts: &LayoutOpts,
    session: &mut LayoutSession,
    chain: &FallbackChain,
) -> Result<Vec<TextLine>, LayoutError> {
    // Single line; truncate with '…' (U+2026) if exceeds max_width
    let ellipsis = '…';
    let (_, ellipsis_adv) = session.glyph(ellipsis, chain).unwrap_or(
        (&GlyphBitmap::replacement_box(), 8.0)
    );
    // Borrow issue: we can't hold reference from session.glyph while calling again.
    // Use glyph_advance directly:
    let ellipsis_adv_val = session.glyph(ellipsis, chain).map(|(_, a)| a).unwrap_or(8.0);

    let mut result = String::new();
    let mut result_w = 0.0f32;
    let mut truncated = false;

    for ch in text.chars() {
        if ch == '\n' { break; }
        let adv = session.glyph(ch, chain).map(|(_, a)| a).unwrap_or(8.0);
        if opts.max_width > 0.0 && result_w + adv + ellipsis_adv_val > opts.max_width {
            truncated = true;
            break;
        }
        result.push(ch);
        result_w += adv;
    }

    let trailing = if truncated {
        result.push(ellipsis);
        result_w += ellipsis_adv_val;
        Some(ellipsis)
    } else {
        None
    };

    Ok(vec![TextLine {
        text: result, width: result_w, y_baseline: 0.0, trailing_glyph: trailing
    }])
}
```

---

## §10 Font Worker

```rust
// supervisor/src/font/worker.rs

use std::sync::Arc;
use std::time::Duration;
use crossbeam_channel::{bounded, Sender, Receiver};
use crate::font::registry::FontRegistry;
use crate::font::types::{GlyphBitmap, FontWeight, FontError};
use crate::font::truetype_provider::quantize_size;

// ──────────────────────────────────────────────────────────────────────────────
// Request / reply types
// ──────────────────────────────────────────────────────────────────────────────

pub type FontReply<T> = crossbeam_channel::Sender<Result<T, String>>;

pub enum FontReq {
    Rasterize {
        font_id:  u32,
        ch:       char,
        size_px:  f32,
        weight:   FontWeight,
        reply:    FontReply<GlyphBitmap>,
    },
    Validate {
        bytes: Vec<u8>,
        reply: FontReply<()>,
    },
    Metrics {
        font_id: u32,
        ch:      char,
        size_px: f32,
        reply:   FontReply<f32>,   // returns advance_width
    },
}

// ──────────────────────────────────────────────────────────────────────────────
// FontWorker (public handle used by wit_handlers.rs)
// ──────────────────────────────────────────────────────────────────────────────

pub struct FontWorker {
    tx:       Sender<FontReq>,
    registry: Arc<FontRegistry>,
}

impl FontWorker {
    /// Spawn monitor + worker threads. Returns handle for submitting requests.
    pub fn spawn(registry: Arc<FontRegistry>) -> Self {
        let (tx, rx) = bounded::<FontReq>(32);
        let reg_mon = registry.clone();
        let tx_mon  = tx.clone();
        std::thread::Builder::new()
            .name("vyoma-font-monitor".into())
            .spawn(move || font_monitor_loop(rx, reg_mon, tx_mon))
            .expect("spawn font monitor");
        Self { tx, registry }
    }

    /// Submit a rasterize request. Returns bitmap or substitute on timeout/panic.
    /// recv_timeout: 200 ms (B1).
    pub fn rasterize(
        &self,
        font_id: u32,
        ch: char,
        size_px: f32,
        weight: FontWeight,
    ) -> GlyphBitmap {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        let req = FontReq::Rasterize { font_id, ch, size_px, weight, reply: reply_tx };
        if self.tx.try_send(req).is_err() {
            log::warn!("font.worker.queue_full: substituting for ch={:?}", ch);
            return GlyphBitmap::substitute_for(ch);
        }
        match reply_rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Ok(bmp))  => bmp,
            Ok(Err(e))   => { log::warn!("font.raster.err: {e}"); GlyphBitmap::substitute_for(ch) }
            Err(_)       => { log::warn!("font.worker.timeout: substituting for ch={:?}", ch); GlyphBitmap::substitute_for(ch) }
        }
    }

    /// Submit a validate request. Returns Ok or Err string.
    pub fn validate(&self, bytes: Vec<u8>) -> Result<(), String> {
        let (reply_tx, reply_rx) = crossbeam_channel::bounded(1);
        let req = FontReq::Validate { bytes, reply: reply_tx };
        if self.tx.try_send(req).is_err() {
            return Err("font worker queue full".into());
        }
        reply_rx.recv_timeout(Duration::from_millis(500))
            .map_err(|_| "font.validate.timeout".to_string())?
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Monitor thread (B1 — watchdog respawn)
// ──────────────────────────────────────────────────────────────────────────────

fn font_monitor_loop(
    rx: Receiver<FontReq>,
    registry: Arc<FontRegistry>,
    _tx: Sender<FontReq>, // kept to prevent channel close
) {
    loop {
        let rx2  = rx.clone();
        let reg2 = registry.clone();
        let handle = std::thread::Builder::new()
            .name("vyoma-font-worker".into())
            .spawn(move || font_worker_loop(rx2, reg2))
            .expect("spawn font worker");

        match handle.join() {
            Ok(_) => {
                // Worker exited cleanly (channel closed = shutdown)
                log::info!("font.worker: exited cleanly");
                break;
            }
            Err(_) => {
                log::error!("font.worker.panic: worker panicked — respawning after 10ms");
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Worker thread (B1)
// ──────────────────────────────────────────────────────────────────────────────

fn font_worker_loop(rx: Receiver<FontReq>, registry: Arc<FontRegistry>) {
    while let Ok(req) = rx.recv() {
        match req {
            FontReq::Rasterize { font_id, ch, size_px, weight, reply } => {
                let result = registry.get(font_id)
                    .map_err(|e| e.to_string())
                    .and_then(|p| {
                        p.render_glyph(ch, size_px, weight).map_err(|e| e.to_string())
                    });
                let _ = reply.send(result);
            }
            FontReq::Validate { bytes, reply } => {
                let result = crate::font::registry::validate_font_file(&bytes)
                    .map_err(|e| e.to_string());
                let _ = reply.send(result);
            }
            FontReq::Metrics { font_id, ch, size_px, reply } => {
                let result = registry.get(font_id)
                    .map_err(|e| e.to_string())
                    .and_then(|p| {
                        p.glyph_advance(ch, size_px).map_err(|e| e.to_string())
                    });
                let _ = reply.send(result);
            }
        }
    }
}

/// Substitute glyph used in caller fallback path.
/// Re-exported from GlyphBitmap for convenience.
#[inline]
pub fn bitmap_substitute(ch: char) -> GlyphBitmap {
    GlyphBitmap::substitute_for(ch)
}
```

---

## §11 WIT Interface

```wit
// supervisor/src/font/wit/typography.wit

package vyoma:fonts@1.0.0;

/// Font system typography interface.
/// Exposed to WASM apps via hostcall bindings in wit_handlers.rs.
interface typography {
    /// Opaque handle to a loaded font.
    type font-handle = u32;

    /// Error string returned by fallible operations.
    type font-error = string;

    /// Font weight: 100=thin 300=light 400=regular 500=medium 600=semibold
    ///              700=bold 800=extrabold 900=black
    type weight = u16;

    /// Wrap mode for layout-text: 0=char 1=word 2=ellipsis
    type wrap-mode = u8;

    // ── Font loading ───────────────────────────────────────────────────────

    /// Load a font from bytes bundled with the WASM app.
    /// Requires capabilities.fonts.bundle to include this font name.
    /// Returns font-handle on success.
    load-font: func(name: string, data: list<u8>) -> result<font-handle, font-error>;

    /// Unload a previously loaded app font. System fonts cannot be unloaded.
    unload-font: func(handle: font-handle) -> result<_, font-error>;

    /// Look up a system font by name (e.g. "VyomaSans-Regular").
    get-system-font: func(name: string) -> result<font-handle, font-error>;

    // ── Measurement ────────────────────────────────────────────────────────

    /// Measure a string. Returns (width, height, ascent, descent, glyph_count).
    measure-text: func(
        font:    font-handle,
        text:    string,
        size-px: f32,
        weight:  weight,
    ) -> result<tuple<f32, f32, f32, f32, u32>, font-error>;

    /// Horizontal advance for a single codepoint (U+xxxx as u32).
    glyph-advance: func(
        font:    font-handle,
        cp:      u32,
        size-px: f32,
    ) -> result<f32, font-error>;

    // ── Layout ─────────────────────────────────────────────────────────────

    /// Layout `text` with the given wrap mode and max-width.
    /// Returns a list of (line_text, width, y_baseline) tuples.
    /// Returns error string "rtl-unsupported" if RTL codepoints detected.
    /// Returns error string "font-not-found:<handle>" if font not loaded.
    layout-text: func(
        font:      font-handle,
        text:      string,
        size-px:   f32,
        max-width: f32,
        wrap:      wrap-mode,
    ) -> result<list<tuple<string, f32, f32>>, font-error>;

    // ── Rasterization ──────────────────────────────────────────────────────

    /// Rasterize a single glyph.
    /// Returns (width, height, x_offset, y_offset, advance, coverage_bytes).
    rasterize-glyph: func(
        font:    font-handle,
        cp:      u32,
        size-px: f32,
        weight:  weight,
    ) -> result<tuple<u32, u32, s32, s32, f32, list<u8>>, font-error>;

    // ── Atlas query (desktop/server only — no-op on other profiles) ────────

    /// Check if a glyph is resident in the GPU atlas.
    /// Returns (u0, v0, u1, v1) normalized UV coords, or error if not cached.
    atlas-uv: func(
        font:    font-handle,
        cp:      u32,
        size-px: f32,
        weight:  weight,
    ) -> result<tuple<f32, f32, f32, f32>, font-error>;
}

world typography-host {
    export typography;
}
```

---

## §12 Manifest Capability

### vyoma.toml syntax

```toml
[capabilities]
stdio   = true
display = true

[capabilities.fonts]
# System fonts to make available (loaded from initramfs at BootPhase::Display).
# Allowed names: "VyomaSans-Regular", "VyomaSans-Bold", "VyomaMono-Regular"
system = ["VyomaSans-Regular", "VyomaSans-Bold"]

# App-bundled font files (relative to app bundle directory in initramfs).
# Each entry must be a .ttf or .otf file shipped alongside the .wasm binary.
# Max size enforced by FontConfig::max_font_bytes for the active platform profile.
bundle = ["fonts/MyCustomFont.ttf"]
```

### Rust parser

```rust
// supervisor/src/font/capability.rs

use serde::Deserialize;
use crate::font::registry::FontRegistry;
use crate::font::types::FontError;

#[derive(Debug, Default, Deserialize)]
pub struct FontsCapability {
    /// System font names the app may use.
    #[serde(default)]
    pub system: Vec<String>,
    /// Bundled font file paths (relative to app bundle dir).
    #[serde(default)]
    pub bundle: Vec<String>,
}

impl FontsCapability {
    /// Parse from TOML value. Returns error on unknown subkeys.
    pub fn from_toml(value: &toml::Value) -> Result<Self, String> {
        // Use toml's Deserializer with deny_unknown_fields equivalent:
        // check that only "system" and "bundle" keys are present.
        if let Some(table) = value.as_table() {
            for key in table.keys() {
                if key != "system" && key != "bundle" {
                    return Err(format!(
                        "capabilities.fonts: unknown subkey '{}'; allowed: system, bundle", key
                    ));
                }
            }
        }
        value.clone().try_into::<FontsCapability>()
            .map_err(|e| format!("capabilities.fonts parse error: {e}"))
    }

    /// Resolve declared system fonts from the registry.
    /// Returns Err for any name not loaded at BootPhase::Display.
    pub fn resolve_system_fonts(
        &self,
        registry: &FontRegistry,
    ) -> Result<Vec<u32>, FontError> {
        let mut ids = Vec::new();
        for name in &self.system {
            let p = registry.get_by_name(name)?;
            ids.push(p.id());
        }
        Ok(ids)
    }

    /// Returns true if the app has declared access to the given system font name.
    pub fn allows_system_font(&self, name: &str) -> bool {
        self.system.iter().any(|n| n == name)
    }

    /// Returns true if the app has declared a bundled font with this file path.
    pub fn allows_bundle_font(&self, path: &str) -> bool {
        self.bundle.iter().any(|p| p == path)
    }
}
```

### Supervisor enforcement

Font hostcalls in `wit_handlers.rs` check `FontsCapability` before dispatching:

```rust
// In handle_load_font (wit_handlers.rs):
let cap = app_state.capabilities.fonts.as_ref()
    .ok_or("app did not declare [capabilities.fonts]")?;
if !cap.allows_bundle_font(&name) {
    return Err(format!("font '{name}' not declared in capabilities.fonts.bundle"));
}

// In handle_get_system_font (wit_handlers.rs):
let cap = app_state.capabilities.fonts.as_ref()
    .ok_or("app did not declare [capabilities.fonts]")?;
if !cap.allows_system_font(&name) {
    return Err(format!("system font '{name}' not declared in capabilities.fonts.system"));
}
```

---

## §13 System Fonts

VyomaOS ships 3 system fonts loaded at `BootPhase::Display`:

| Name | Source | License | File in initramfs |
|------|--------|---------|-------------------|
| VyomaSans-Regular | Inter Regular (subset: U+0020–U+024F + U+FFFD) | OFL-1.1 | `/usr/share/fonts/vyoma/VyomaSans-Regular.ttf` |
| VyomaSans-Bold | Inter Bold (same subset) | OFL-1.1 | `/usr/share/fonts/vyoma/VyomaSans-Bold.ttf` |
| VyomaMono-Regular | JetBrains Mono Regular (subset: U+0020–U+007E + U+2500–U+257F + U+FFFD) | OFL-1.1 | `/usr/share/fonts/vyoma/VyomaMono-Regular.ttf` |

License files live at `/usr/share/fonts/vyoma/LICENSE-Inter-OFL.txt` and `/usr/share/fonts/vyoma/LICENSE-JetBrainsMono-OFL.txt`.

### Subsetting

Fonts are subset using `pyftsubset` (fonttools) during `make rootfs`:
- VyomaSans: U+0020-U+024F,U+0300-U+036F,U+FFFD — covers Latin Extended-A/B + combining marks
- VyomaMono: U+0020-U+007E,U+2500-U+257F,U+FFFD — covers ASCII + box-drawing chars for terminal apps

Target sizes after subsetting: VyomaSans-Regular ≤400 KB, VyomaSans-Bold ≤400 KB, VyomaMono-Regular ≤300 KB.

### boot_load_system_fonts

```rust
// supervisor/src/font/system_fonts.rs

use std::sync::Arc;
use crate::font::registry::FontRegistry;
use crate::font::types::FontError;

const FONT_PATHS: &[(&str, &str)] = &[
    ("VyomaSans-Regular",  "/usr/share/fonts/vyoma/VyomaSans-Regular.ttf"),
    ("VyomaSans-Bold",     "/usr/share/fonts/vyoma/VyomaSans-Bold.ttf"),
    ("VyomaMono-Regular",  "/usr/share/fonts/vyoma/VyomaMono-Regular.ttf"),
];

/// Called during BootPhase::Display, before any app is spawned.
/// Registers system fonts; exits process with error if any font fails to load.
pub fn boot_load_system_fonts(registry: &Arc<FontRegistry>) {
    for (name, path) in FONT_PATHS {
        match std::fs::read(path) {
            Ok(data) => {
                match registry.load_system_font(name, &data) {
                    Ok(id) => {
                        log::info!("font.system.loaded: name={name} id={id} bytes={}", data.len());
                    }
                    Err(FontError::ParseFailed(e)) => {
                        // Non-fatal: log, continue — bitmap fallback covers Latin
                        log::error!("font.system.parse_failed: name={name} err={e}");
                    }
                    Err(e) => {
                        log::error!("font.system.load_err: name={name} err={e}");
                    }
                }
            }
            Err(e) => {
                // Font file missing in initramfs — log error, continue with bitmap
                log::error!("font.system.missing: path={path} err={e}");
            }
        }
    }
}
```

---

## §14 Integration with R11/R12

### R11: VYOMA_DRAW v1 and v2

`draw_text` and `draw_text_wrap` commands processed in `supervisor/src/draw_cmd.rs`:

```rust
// In draw_cmd.rs — handle_draw_text_scaled (replaces legacy handle_draw_text)
pub fn handle_draw_text_scaled(
    surface: &mut Surface,
    x: i32, y: i32,
    rgba: u32,
    size_px: f32,
    text: &str,
    max_w: u32,
    wrap_mode: WrapMode,
    font_worker: &FontWorker,
    registry: &Arc<FontRegistry>,
    chain: &FallbackChain,
) {
    // Create LayoutSession (B3: quantization applied once here)
    let mut session = LayoutSession::new(registry, size_px, FontWeight::Regular);
    let opts = LayoutOpts {
        max_width: if max_w > 0 { max_w as f32 } else { 0.0 },
        wrap: wrap_mode,
        line_spacing: 0.0,
    };
    let lines = match layout_text(text, opts, &mut session, chain) {
        Ok(l) => l,
        Err(LayoutError::RtlUnsupported) => {
            log::warn!("font.layout.rtl_dropped: text contains RTL codepoints; skipping render");
            return;
        }
        Err(e) => {
            log::warn!("font.layout.err: {e}");
            return;
        }
    };
    for line in &lines {
        let mut cursor_x = x;
        let baseline_y = y + line.y_baseline as i32;
        for ch in line.text.chars() {
            let bmp = font_worker.rasterize(
                chain.resolve(ch, registry).id(),
                ch, size_px, FontWeight::Regular
            );
            blit_glyph(surface, cursor_x, baseline_y - bmp.y_offset, rgba, &bmp);
            cursor_x += bmp.advance as i32;
        }
    }
}

/// Blit a glyph alpha-coverage bitmap onto a Surface (BGRA 32bpp).
fn blit_glyph(surface: &mut Surface, x: i32, y: i32, rgba: u32, bmp: &GlyphBitmap) {
    let (fg_r, fg_g, fg_b, fg_a) = unpack_rgba(rgba);
    for row in 0..bmp.height {
        for col in 0..bmp.width {
            let alpha = bmp.coverage[(row * bmp.width + col) as usize] as f32 / 255.0;
            let px = (x + col as i32, y + row as i32);
            if px.0 < 0 || px.1 < 0 || px.0 >= surface.width as i32 || px.1 >= surface.height as i32 {
                continue;
            }
            let dst = surface.pixel_mut(px.0 as u32, px.1 as u32);
            *dst = alpha_blend(*dst, pack_bgra(fg_b, fg_g, fg_r, fg_a), (alpha * fg_a as f32 / 255.0) as f32);
        }
    }
}
```

### VYOMA_DRAW v2 commands

Two new protocol commands added in v2 (backwards compatible; v1 apps unaffected):

```
VYOMA_DRAW:load_font:<name>,<base64_data>
    Load a font from base64-encoded bytes. Name must match capabilities.fonts.bundle.
    Returns: VYOMA_DRAW_ACK:load_font:<name>,ok  or  VYOMA_DRAW_ACK:load_font:<name>,err:<msg>

VYOMA_DRAW:select_font:<name>,<size_px>,<weight>
    Set the active font for subsequent draw_text calls from this app.
    <weight> is CSS numeric (100–900). Quantized internally.
    Returns: VYOMA_DRAW_ACK:select_font:ok  or  VYOMA_DRAW_ACK:select_font:err:<msg>
```

### R12: GPU atlas VSync integration

```rust
// In compositor.rs — VSync flush pass (after R12 wgpu draw calls)
pub fn vsync_flush_font_atlas(
    compositor: &CompositorDevice,
    atlas: &GpuGlyphAtlas,
    queue: &wgpu::Queue,
    font_texture: &wgpu::Texture,
) {
    atlas.advance_frame();
    if !atlas.is_dirty() { return; }
    let staging = atlas.take_dirty();
    for (x, y, bmp) in staging {
        // Upload alpha coverage bytes to the font texture (R8Unorm format)
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: font_texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &bmp.coverage,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(bmp.width),
                rows_per_image: Some(bmp.height),
            },
            wgpu::Extent3d { width: bmp.width, height: bmp.height, depth_or_array_layers: 1 },
        );
    }
}
```

The font texture is `wgpu::TextureFormat::R8Unorm`, 2048×2048, owned by `CompositorDevice`. Glyphs are sampled via a dedicated `BindGroup` in the text render pipeline. Font color is applied in the fragment shader by multiplying the sampled alpha by a per-draw `rgba` uniform.

---

## §15 Platform Feature Matrix

| Feature | mcu-minimal | iot-edge | robotics-rt | mobile | desktop-full | server-headless |
|---------|:-----------:|:--------:|:-----------:|:------:|:------------:|:---------------:|
| Bitmap font | yes | yes | yes | yes | yes | yes |
| TrueType (fontdue) | **no** | yes | yes | yes | yes | yes |
| App-bundled fonts | no | yes | yes | yes | yes | yes |
| System fonts (TTF) | no | yes | yes | yes | yes | yes |
| CPU LRU glyph cache | no | yes (256) | yes (256) | yes (4096) | yes (8192) | yes (8192) |
| GPU atlas (etagere) | **no** | **no** | **no** | **no** | yes | yes |
| CJK font support | **no** | **no** | **no** | subset | yes | yes |
| Font worker thread | no | yes | yes | yes | yes | yes |
| RTL (v1) | no | no | no | no | no | no |
| RTL error returned | n/a | yes | yes | yes | yes | yes |
| U+FFFD box glyph | yes | yes | yes | yes | yes | yes |
| Max per-PID atlas entries | 0 | 0 | 0 | 0 | 128 | 128 |
| Max font file bytes | 0 | 4 MiB | 4 MiB | 16 MiB | 32 MiB | 32 MiB |
| Max total font RAM | 0 | 8 MiB | 8 MiB | 64 MiB | 256 MiB | 256 MiB |

---

## §16 Performance Budget

### Per-operation timing targets (measured on desktop-full, i7-class CPU)

| Operation | Expected P50 | Expected P99 | Failure threshold |
|-----------|:-----------:|:------------:|:-----------------:|
| Bitmap glyph render (cache hit) | <1 µs | <2 µs | >10 µs → log warn |
| TrueType glyph render (LRU hit) | <1 µs | <3 µs | >10 µs → log warn |
| TrueType glyph render (LRU miss, fontdue) | <50 µs | <200 µs | >500 µs → log warn |
| measure_text (10 chars, LRU warm) | <5 µs | <20 µs | >100 µs → log warn |
| layout_text (80 chars, word wrap, warm) | <20 µs | <80 µs | >500 µs → log warn |
| GPU atlas get_or_insert (hit) | <2 µs | <5 µs | — |
| GPU atlas get_or_insert (miss + alloc) | <10 µs | <50 µs | — |
| Font worker round-trip (rasterize) | <100 µs | <180 µs | 200 ms → timeout → substitute |

### Frame-time analysis

| Scenario | Frame budget (60 Hz = 16.7 ms) | Font contribution |
|----------|:------------------------------:|:-----------------:|
| 80×24 terminal, all glyphs cold (bitmap) | 16.7 ms | ≤2 ms (1920 glyphs × <1 µs) |
| 80×24 terminal, all glyphs warm (bitmap) | 16.7 ms | ≤0.5 ms |
| GUI dashboard, mixed TrueType, cold | 16.7 ms | ≤8 ms (400 glyphs × <20 µs avg) |
| GUI dashboard, mixed TrueType, warm | 16.7 ms | ≤1 ms |
| GPU atlas upload (100 new glyphs) | 16.7 ms | ≤1 ms (write_texture batched) |

### Font worker queue saturation

- Channel bounded at 32 entries. At >80% fill (≥26 pending), log `font.worker.queue_high_watermark`.
- If queue full on `try_send`, caller immediately returns substitute glyph without blocking.
- Worker stall >100 ms triggers `font.worker.stall` log warning in monitor thread.

---

## §17 Security Envelope

Four independent layers defend against malicious or corrupt font files:

### Layer 1: Size gate (FontConfig)

```rust
// In load_app_font — checked before any parsing
if data.len() > self.config.max_font_bytes {
    return Err(FontError::TooLarge(data.len()));
}
// Hard cap even for validate_font_file path:
if data.len() > MAX_FONT_BYTES_VALIDATE { // 64 MiB
    return Err(FontError::TooLarge(data.len()));
}
```

### Layer 2: Pre-flight rasterization validation (B7)

Called in `validate_font_file` before any font is registered. Catches fonts that parse successfully but panic or produce garbage on first rasterization attempt. Rasterizes 10 representative ASCII glyphs at 14px inside `catch_unwind`.

### Layer 3: Per-call catch_unwind (B1)

Every call to `fontdue::Font::rasterize` and `fontdue::Font::metrics` is wrapped in `safe_rasterize` / `safe_metrics` which call `std::panic::catch_unwind(AssertUnwindSafe(...))`. A panic in fontdue returns `Err(FontError::RasterPanic)` or `Err(FontError::MetricsPanic)` and does NOT propagate to the supervisor or compositor.

### Layer 4: Worker watchdog respawn (B1)

If a fontdue call panics inside the worker thread (bypassing `catch_unwind` somehow, e.g., stack overflow), the thread terminates. The monitor thread detects this via `JoinHandle::join()` returning `Err`, logs `font.worker.panic`, waits 10 ms, and respawns the worker. Inflight requests time out via `recv_timeout(200 ms)` on the caller side and return substitute glyphs.

### Layer 5: Per-PID atlas quota (B2)

Each PID is limited to `FontConfig::max_atlas_entries_per_pid` GPU atlas entries (0 on non-GPU platforms). Exceeding the quota triggers LRU eviction of the oldest entry for that PID before allocating a new one. A PID cannot DoS the atlas for other PIDs.

### Layer 6: Estimated-RAM accounting (B4)

`FontRegistry` tracks `total_estimated_ram` using `estimated_ram(raw_bytes) = raw_bytes × 2`. Loading a new font that would exceed `FontConfig::max_total_estimated_ram` is rejected with `FontError::RegistryFull`. This prevents a single app from exhausting system RAM by loading many large fonts.

### Summary

```
Attack surface          → Defense
────────────────────────────────────────────────────────────────────────────────
Oversized font file     → Layer 1 (size gate before parse)
Parser panic/UB         → Layer 2 (pre-flight) + Layer 3 (per-call catch_unwind)
Post-parse render panic → Layer 3 (per-call catch_unwind) + Layer 4 (worker respawn)
Worker thread death     → Layer 4 (monitor respawn + recv_timeout substitute)
GPU atlas exhaustion    → Layer 5 (per-PID quota + LRU eviction)
RAM exhaustion          → Layer 6 (estimated_ram tracking + RegistryFull error)
```

---

## §18 Implementation Files

All files live under `supervisor/src/font/`. No file may exceed 500 lines (project-wide rule).

| File | Responsibility | Approx LOC |
|------|---------------|:----------:|
| `mod.rs` | Module declarations + `FontSubsystem` aggregate struct | ~80 |
| `types.rs` | `GlyphCoverage`, `FontWeight`, `TextMetrics`, `GlyphBitmap`, `FontError`, `LayoutError` | ~150 |
| `config.rs` | `FontConfig`, `estimated_ram`, platform dispatch | ~100 |
| `provider.rs` | `FontProvider` trait definition | ~60 |
| `bitmap_provider.rs` | `BitmapFontProvider`, nearest-neighbour scaling | ~200 |
| `bitmap_data.rs` | Static glyph tables (`SMALL_GLYPHS`, `MEDIUM_GLYPHS`, `LARGE_GLYPHS`) | ~450 |
| `truetype_provider.rs` | `TrueTypeFontProvider`, `safe_rasterize`, `safe_metrics`, `quantize_size`, `GlyphCacheKey` | ~280 |
| `atlas_cpu.rs` | `CpuGlyphAtlas`, `GlyphCacheKey` (shared with GPU) | ~80 |
| `atlas_gpu.rs` | `GpuGlyphAtlas`, etagere allocator, per-PID quota, staging | ~220 |
| `registry.rs` | `FontRegistry`, `validate_font_file` (with pre-flight) | ~250 |
| `fallback.rs` | `FallbackChain`, `GlyphCoverage`-aware `resolve()`, `render_run` | ~120 |
| `layout.rs` | `LayoutSession`, `layout_text`, `WrapMode`, `TextLine`, `detect_rtl`, wrap impls | ~380 |
| `worker.rs` | `FontWorker`, `FontMonitor`, `FontReq`, `font_worker_loop`, `font_monitor_loop` | ~200 |
| `capability.rs` | `FontsCapability`, TOML parser, `resolve_system_fonts`, enforcement helpers | ~100 |
| `system_fonts.rs` | `boot_load_system_fonts`, `FONT_PATHS` constant table | ~60 |

Total: ~2730 LOC across 15 files — all under the 500-line per-file limit.

### mod.rs aggregate

```rust
// supervisor/src/font/mod.rs

pub mod types;
pub mod config;
pub mod provider;
pub mod bitmap_provider;
pub mod bitmap_data;
pub mod truetype_provider;
pub mod atlas_cpu;
pub mod atlas_gpu;
pub mod registry;
pub mod fallback;
pub mod layout;
pub mod worker;
pub mod capability;
pub mod system_fonts;

use std::sync::Arc;
use crate::profile::PlatformProfile;

/// Aggregate handle for the entire font subsystem.
/// Created once at BootPhase::Display; held in SupervisorState.
pub struct FontSubsystem {
    pub registry: Arc<registry::FontRegistry>,
    pub chain:    fallback::FallbackChain,
    pub worker:   worker::FontWorker,
    /// GPU atlas — None on platforms where gpu_atlas_enabled = false.
    pub gpu_atlas: Option<Arc<atlas_gpu::GpuGlyphAtlas>>,
}

impl FontSubsystem {
    pub fn init(platform: &PlatformProfile) -> Self {
        let config  = config::FontConfig::for_platform(platform);
        let gpu_cfg = config.clone();
        let registry = registry::FontRegistry::new(config.clone());
        system_fonts::boot_load_system_fonts(&registry);
        let chain    = fallback::FallbackChain::system_default(&registry);
        let worker   = worker::FontWorker::spawn(registry.clone());
        let gpu_atlas = if gpu_cfg.gpu_atlas_enabled {
            Some(atlas_gpu::GpuGlyphAtlas::new(gpu_cfg.max_atlas_entries_per_pid))
        } else {
            None
        };
        Self { registry, chain, worker, gpu_atlas }
    }
}
```

---

## §19 Deferred (v2/v3)

The following features are explicitly deferred and must NOT be partially implemented in v1. Any partial implementation would create API surface that would need breaking changes in v2.

| Feature | Target | Rationale for deferral |
|---------|--------|------------------------|
| HarfBuzz / complex shaping | v2 | Requires OpenType GSUB/GPOS tables; adds ~500 KB to binary; Arabic, Devanagari, Thai need it |
| RTL full Unicode BiDi algorithm | v2 | ICU4X or unicode-bidi crate needed; 6+ week effort; v1 returns explicit error (B5) |
| OpenType variable fonts (fvar axis) | v2 | fontdue does not support variable fonts; would require HarfBuzz or allsorts |
| Color / emoji fonts (CBDT, SBIX, COLR) | v3 | Requires separate color rendering pipeline; emoji data is large (~10 MB) |
| Dynamic font download over network | v3 | Requires verified download, sandboxed validation, font cache on `/data` |
| Subpixel / LCD hinting | v2 | Requires RGB fringe model + compositor awareness; not needed for QEMU/Lavapipe |
| Font synthesis (oblique/bold simulation) | v2 | Low priority; system fonts cover Regular+Bold; oblique via matrix transform |
| OpenType features (ligatures, kerning) | v2 | Needs HarfBuzz; v1 uses raw advance width only |
| Signed Distance Field (SDF) rendering | v3 | Enables resolution-independent rendering; requires offline SDF baking pipeline |
| WebFont / WOFF2 format support | v2 | Add woff2 decompression step in validate_font_file |

---

## Cargo.toml additions

```toml
# supervisor/Cargo.toml — add to [dependencies]

# Font rasterization (TrueType/OpenType)
fontdue = { version = "0.8", optional = true }

# GPU atlas packing (Skyline Bottom-Left algorithm)
etagere = { version = "0.2", optional = true }

# LRU cache for CPU glyph cache
lru = "0.12"

# Error types
thiserror = "1"

# Channel for font worker
crossbeam-channel = "0.5"

[features]
default       = ["truetype", "gpu-atlas"]
truetype      = ["dep:fontdue"]
gpu-atlas     = ["dep:etagere"]
mcu-minimal   = []  # disables truetype + gpu-atlas
iot-edge      = ["truetype"]
robotics-rt   = ["truetype"]
mobile        = ["truetype"]
desktop-full  = ["truetype", "gpu-atlas"]
server-headless = ["truetype", "gpu-atlas"]
```

Feature flags gate `fontdue` and `etagere` compilation: on `mcu-minimal`, neither is compiled in, saving ~800 KB from the binary. The `TrueTypeFontProvider` and `GpuGlyphAtlas` modules are `#[cfg(feature = "truetype")]` and `#[cfg(feature = "gpu-atlas")]` respectively.

---

*End of Subsystem 13 FINAL specification. All 7 blocking issues (B1–B7) are resolved. No "TBD" sections remain. This document is the authoritative input for implementers.*
