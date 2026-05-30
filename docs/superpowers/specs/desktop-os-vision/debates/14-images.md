# Round 14 — Image & Icon Pipeline
**Role:** Architect | **Date:** 2026-05-29 | **Status:** Proposal

## 0. Executive summary

Rounds 11–13 completed the display surface (`Surface`, BGRA32 blit), GPU path (wgpu staging buffers, WIT `vyoma:gpu/graphics`), and typography (`FontProvider`, `GlyphAtlas`, `FontWorker`). The missing peer subsystem is the **image and icon pipeline**: decoding raster and vector files, multi-resolution icon bundles, a decode worker with adversarial-input isolation, an LRU image cache, and the WIT + VYOMA_DRAW_V2 protocol extension that makes all of it accessible to WASM apps.

The macOS analogue is **ImageIO + CoreImage**: ImageIO handles format decoding, color space conversion, and metadata; CoreImage provides GPU-accelerated image processing. VyomaOS targets the same capability surface minus the shader-based filter graph; filter composition is deferred to a future round.

Headline constraints this round must satisfy:

| Constraint | Value |
|---|---|
| Supervisor source file limit | ≤ 500 LOC per `.rs` file |
| Maximum image dimension | 8 192 px (desktop), 2 048 px (mobile), 1 024 px (iot/robotics) |
| Per-PID decode budget | configurable via `ImageConfig`; enforced before alloc |
| Adversarial input defense | every decoder call wrapped in `catch_unwind(AssertUnwindSafe(...))` |
| Decode worker panic recovery | watchdog respawns worker; callers receive `ImageError::WorkerPanic` |
| Display pixel formats | BGRA32 (desktop/server), RGB565 (iot/robotics), Mono1bpp (mcu-minimal) |
| Formats enabled on mcu-minimal | none (image pipeline feature-gated out entirely) |

---

## 1. Core data model (`supervisor/src/image/mod.rs`)

Every type defined here is `pub` and re-exported from the crate root via `pub mod image`.

```rust
//! supervisor/src/image/mod.rs
//! Public types for the VyomaOS image & icon pipeline.
//! ≤ 500 LOC — keep this file to type definitions and re-exports only.

pub mod cache;
pub mod config;
pub mod decode_bmp;
pub mod decode_jpeg;
pub mod decode_png;
pub mod decode_qoi;
#[cfg(feature = "svg")]
pub mod decode_svg;
pub mod icon;
pub mod manifest;
pub mod resize;
pub mod surface_ext;
pub mod wit_handlers;
pub mod worker;

use std::sync::Arc;

// ── pixel format ──────────────────────────────────────────────────────────────

/// Pixel format tag for `ImageData` and `Surface` compatibility checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 4 bytes/pixel: B G R A (native display format on desktop/server).
    Bgra32,
    /// 2 bytes/pixel: RGB packed 5-6-5 little-endian (iot-edge, robotics-rt).
    Rgb565,
    /// 1 bit/pixel, MSB-first row-packed (mcu-minimal monochrome display).
    Mono1bpp,
    /// 1 byte/pixel, luminance only (intermediate for grayscale JPEG).
    Grayscale8,
    /// 3 bytes/pixel: R G B (intermediate for BMP/JPEG before BGRA conversion).
    Rgb24,
}

impl PixelFormat {
    /// Bytes per pixel for packed formats.  Returns `None` for `Mono1bpp`
    /// because stride depends on row width.
    pub fn bytes_per_pixel(self) -> Option<usize> {
        match self {
            PixelFormat::Bgra32    => Some(4),
            PixelFormat::Rgb565    => Some(2),
            PixelFormat::Grayscale8 => Some(1),
            PixelFormat::Rgb24     => Some(3),
            PixelFormat::Mono1bpp  => None,
        }
    }
}

// ── image format tag ──────────────────────────────────────────────────────────

/// Format detected from magic bytes or supplied as a hint.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Bmp,
    Qoi,
    /// SVG — decoded on CPU via `resvg`; only available when feature `svg` is enabled.
    Svg,
}

impl ImageFormat {
    /// Sniff format from the first 12 bytes of the file.
    pub fn from_magic(bytes: &[u8]) -> Option<ImageFormat> {
        if bytes.len() < 4 { return None; }
        match bytes {
            b if b.starts_with(b"\x89PNG")                  => Some(ImageFormat::Png),
            b if b.starts_with(b"\xFF\xD8\xFF")             => Some(ImageFormat::Jpeg),
            b if b.starts_with(b"BM")                       => Some(ImageFormat::Bmp),
            b if b.starts_with(b"qoif")                     => Some(ImageFormat::Qoi),
            b if b.starts_with(b"<?xml") || b.starts_with(b"<svg") => Some(ImageFormat::Svg),
            _ => None,
        }
    }

    /// Format identifier string as it appears in vyoma.toml capability lists.
    pub fn capability_name(self) -> &'static str {
        match self {
            ImageFormat::Png  => "png",
            ImageFormat::Jpeg => "jpeg",
            ImageFormat::Bmp  => "bmp",
            ImageFormat::Qoi  => "qoi",
            ImageFormat::Svg  => "svg",
        }
    }
}

// ── image data ─────────────────────────────────────────────────────────────────

/// Decoded image held in CPU memory.
///
/// `pixels` is **always** in `format` layout with `stride` bytes per row.
/// All decoders normalize to `Bgra32` on desktop-full/mobile/server;
/// on iot-edge/robotics-rt they normalize to `Rgb565`.
#[derive(Clone, Debug)]
pub struct ImageData {
    /// Raw pixel bytes, row-major, length = `height * stride`.
    pub pixels: Vec<u8>,
    pub width:  u32,
    pub height: u32,
    /// Bytes per row (may be > width * bytes_per_pixel for alignment).
    pub stride: u32,
    pub format: PixelFormat,
    /// Format the image was decoded from; retained for cache key and diagnostics.
    pub source_format: ImageFormat,
}

impl ImageData {
    /// Total heap bytes consumed by pixel buffer.
    pub fn byte_size(&self) -> usize {
        self.pixels.len()
    }

    /// Validate internal consistency.
    pub fn is_valid(&self) -> bool {
        if self.width == 0 || self.height == 0 { return false; }
        let expected = self.stride as usize * self.height as usize;
        self.pixels.len() == expected
    }
}

// ── resize filter ─────────────────────────────────────────────────────────────

/// Sampling filter used when scaling `ImageData` to a different resolution.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeFilter {
    /// Zero-latency, aliased.  Best for power-of-2 icon downscales.
    Nearest,
    /// 2×2 bilinear tap.  Good for thumbnails; moderate quality.
    Bilinear,
    /// 6-tap Lanczos (a=3).  Highest quality; use for large display images.
    Lanczos3,
}

// ── icon set ──────────────────────────────────────────────────────────────────

/// Standard logical sizes (device-independent pixels) the icon system
/// recognises.  Apps may supply a subset; missing sizes are synthesised by
/// downscaling the next-larger variant.
pub const ICON_STANDARD_SIZES: &[u32] = &[16, 32, 48, 64, 128, 256, 512];

/// One multi-resolution icon bundle, analogous to a macOS `.icns` file.
///
/// `variants` maps `(logical_px, scale_numerator)` → `Arc<ImageData>`.
/// Scale denominators are always 1; scale 2 means "@2x" HiDPI variant.
#[derive(Clone, Debug)]
pub struct IconSet {
    pub app_name: String,
    /// All decoded variants.  Key: `(logical_px, scale)` where scale ∈ {1, 2, 3}.
    pub variants: std::collections::HashMap<(u32, u8), Arc<ImageData>>,
    /// Source directory path (for invalidation).
    pub bundle_path: String,
}

impl IconSet {
    /// Select the best-fit `ImageData` for a given logical size and HiDPI scale.
    ///
    /// Strategy:
    /// 1. Return exact match `(logical_px, scale_floor)`.
    /// 2. Fallback to next-larger size at same scale, downsample on first use.
    /// 3. Fallback to @1x of same logical size, upsample.
    /// 4. Return largest available variant and let the compositor scale.
    pub fn icon_for_size(
        &self,
        logical_px: u32,
        scale: f32,
    ) -> Option<Arc<ImageData>> {
        let scale_floor = scale.floor() as u8;
        // exact match
        if let Some(img) = self.variants.get(&(logical_px, scale_floor)) {
            return Some(Arc::clone(img));
        }
        // next-larger at same scale
        for &sz in ICON_STANDARD_SIZES.iter().filter(|&&s| s > logical_px) {
            if let Some(img) = self.variants.get(&(sz, scale_floor)) {
                return Some(Arc::clone(img));
            }
        }
        // @1x fallback
        if let Some(img) = self.variants.get(&(logical_px, 1)) {
            return Some(Arc::clone(img));
        }
        // largest available
        for &sz in ICON_STANDARD_SIZES.iter().rev() {
            for &sc in &[3u8, 2, 1] {
                if let Some(img) = self.variants.get(&(sz, sc)) {
                    return Some(Arc::clone(img));
                }
            }
        }
        None
    }
}

// ── error ─────────────────────────────────────────────────────────────────────

/// All error conditions in the image pipeline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageError {
    /// Decoded image would exceed per-app or per-platform byte budget.
    TooLarge { bytes_needed: usize, budget_remaining: usize },
    /// Format-specific decode failure (corrupt data, truncated file).
    DecodeFailed(String),
    /// Decoder panicked; caught by `catch_unwind`.
    DecodePanic,
    /// Worker did not respond within the 500 ms timeout.
    WorkerTimeout,
    /// Worker thread panicked and was restarted; request lost.
    WorkerPanic,
    /// Requested format not enabled in `ImageConfig` for this platform.
    UnsupportedFormat(ImageFormat),
    /// Width or height is zero, or exceeds `max_image_dimension`.
    InvalidDimensions { width: u32, height: u32 },
    /// Format not enabled in app's manifest `capabilities.images.decode`.
    FormatNotEnabled(String),
    /// Path is outside permitted directories from manifest.
    PathTraversal(String),
    /// GPU upload failed (device lost, OOM on VRAM).
    GpuUploadFailed(String),
    /// Icon bundle path does not exist or contains no valid images.
    IconBundleNotFound(String),
    /// Handle supplied to a host-call is unknown or already freed.
    InvalidHandle(u32),
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl std::error::Error for ImageError {}
```

---

## 2. Platform configuration (`supervisor/src/image/config.rs`)

```rust
//! supervisor/src/image/config.rs
//! Per-platform image capability matrix, populated from the platform profile
//! loaded in Round 12 and exposed to the image worker at startup.

use crate::image::{ImageFormat, PixelFormat};
use std::collections::HashSet;

/// Immutable platform-wide image configuration.
/// Constructed once at supervisor startup from the active platform profile.
#[derive(Clone, Debug)]
pub struct ImageConfig {
    /// Formats the platform may decode at all (compile-time + runtime gate).
    pub formats_enabled: HashSet<ImageFormat>,
    /// Native pixel format for this platform.
    pub native_pixel_format: PixelFormat,
    /// Maximum byte budget per PID across all resident decoded images.
    /// Enforced before every allocation inside decoders.
    pub max_decode_bytes: usize,
    /// Hard cap on image width and height (checked before alloc).
    pub max_image_dimension: u32,
    /// SVG rasterisation available (requires feature `svg` AND platform allows).
    pub svg_enabled: bool,
    /// GPU upload path available (desktop-full/mobile only).
    pub gpu_upload_enabled: bool,
    /// Icon system available (not on mcu-minimal or server-headless).
    pub icon_system_enabled: bool,
    /// High-water mark for LRU eviction (bytes); 0 disables caching.
    pub cache_high_water_bytes: usize,
    /// Low-water mark; eviction stops here.
    pub cache_low_water_bytes: usize,
    /// Maximum pixels in a single GPU texture upload.
    pub gpu_max_texture_pixels: u32,
}

impl ImageConfig {
    pub fn for_platform(name: &str) -> ImageConfig {
        match name {
            "mcu-minimal" => ImageConfig {
                formats_enabled:        HashSet::new(),
                native_pixel_format:    PixelFormat::Mono1bpp,
                max_decode_bytes:       0,
                max_image_dimension:    0,
                svg_enabled:            false,
                gpu_upload_enabled:     false,
                icon_system_enabled:    false,
                cache_high_water_bytes: 0,
                cache_low_water_bytes:  0,
                gpu_max_texture_pixels: 0,
            },
            "iot-edge" => ImageConfig {
                formats_enabled:        [ImageFormat::Png, ImageFormat::Jpeg,
                                         ImageFormat::Qoi, ImageFormat::Bmp]
                                            .into_iter().collect(),
                native_pixel_format:    PixelFormat::Rgb565,
                max_decode_bytes:       4 * 1024 * 1024,
                max_image_dimension:    1024,
                svg_enabled:            false,
                gpu_upload_enabled:     false,
                icon_system_enabled:    true,
                cache_high_water_bytes: 2 * 1024 * 1024,
                cache_low_water_bytes:  1 * 1024 * 1024,
                gpu_max_texture_pixels: 0,
            },
            "robotics-rt" => ImageConfig {
                formats_enabled:        [ImageFormat::Png, ImageFormat::Jpeg,
                                         ImageFormat::Qoi, ImageFormat::Bmp]
                                            .into_iter().collect(),
                native_pixel_format:    PixelFormat::Rgb565,
                max_decode_bytes:       8 * 1024 * 1024,
                max_image_dimension:    1024,
                svg_enabled:            false,
                gpu_upload_enabled:     false,
                icon_system_enabled:    true,
                cache_high_water_bytes: 4 * 1024 * 1024,
                cache_low_water_bytes:  2 * 1024 * 1024,
                gpu_max_texture_pixels: 0,
            },
            "mobile" => ImageConfig {
                formats_enabled:        [ImageFormat::Png, ImageFormat::Jpeg,
                                         ImageFormat::Qoi, ImageFormat::Bmp,
                                         ImageFormat::Svg]
                                            .into_iter().collect(),
                native_pixel_format:    PixelFormat::Bgra32,
                max_decode_bytes:       32 * 1024 * 1024,
                max_image_dimension:    2048,
                svg_enabled:            true,
                gpu_upload_enabled:     true,
                icon_system_enabled:    true,
                cache_high_water_bytes: 48 * 1024 * 1024,
                cache_low_water_bytes:  24 * 1024 * 1024,
                gpu_max_texture_pixels: 2048 * 2048,
            },
            "server-headless" => ImageConfig {
                formats_enabled:        [ImageFormat::Png, ImageFormat::Jpeg,
                                         ImageFormat::Qoi, ImageFormat::Bmp]
                                            .into_iter().collect(),
                native_pixel_format:    PixelFormat::Bgra32,
                max_decode_bytes:       128 * 1024 * 1024,
                max_image_dimension:    8192,
                svg_enabled:            false,
                gpu_upload_enabled:     false,
                icon_system_enabled:    false,
                cache_high_water_bytes: 256 * 1024 * 1024,
                cache_low_water_bytes:  128 * 1024 * 1024,
                gpu_max_texture_pixels: 0,
            },
            _ /* desktop-full, default */ => ImageConfig {
                formats_enabled:        [ImageFormat::Png, ImageFormat::Jpeg,
                                         ImageFormat::Qoi, ImageFormat::Bmp,
                                         ImageFormat::Svg]
                                            .into_iter().collect(),
                native_pixel_format:    PixelFormat::Bgra32,
                max_decode_bytes:       256 * 1024 * 1024,
                max_image_dimension:    8192,
                svg_enabled:            true,
                gpu_upload_enabled:     true,
                icon_system_enabled:    true,
                cache_high_water_bytes: 512 * 1024 * 1024,
                cache_low_water_bytes:  256 * 1024 * 1024,
                gpu_max_texture_pixels: 8192 * 8192,
            },
        }
    }

    /// Return `true` when `format` is permitted both by platform config and
    /// by the app's manifest `capabilities.images.decode` list.
    pub fn format_allowed(
        &self,
        format: ImageFormat,
        app_formats: &HashSet<String>,
    ) -> bool {
        self.formats_enabled.contains(&format)
            && app_formats.contains(format.capability_name())
    }

    /// Guard: returns error if `bytes_needed` would exceed per-PID budget.
    pub fn check_budget(
        &self,
        bytes_needed: usize,
        bytes_used: usize,
    ) -> Result<(), crate::image::ImageError> {
        let remaining = self.max_decode_bytes.saturating_sub(bytes_used);
        if bytes_needed > remaining {
            Err(crate::image::ImageError::TooLarge { bytes_needed, budget_remaining: remaining })
        } else {
            Ok(())
        }
    }

    /// Guard: returns error if width or height exceeds platform maximum.
    pub fn check_dimensions(
        &self,
        width: u32,
        height: u32,
    ) -> Result<(), crate::image::ImageError> {
        if width == 0 || height == 0 || width > self.max_image_dimension || height > self.max_image_dimension {
            Err(crate::image::ImageError::InvalidDimensions { width, height })
        } else {
            Ok(())
        }
    }
}
```

---

## 3. PNG decoder (`supervisor/src/image/decode_png.rs`)

```rust
//! supervisor/src/image/decode_png.rs
//! PNG decode wrapper around the `png` crate (pure Rust, no C deps).
//! Every decode is wrapped in catch_unwind for adversarial input safety.

use std::panic::{catch_unwind, AssertUnwindSafe};
use crate::image::{ImageData, ImageError, ImageFormat, PixelFormat};

/// Decode PNG bytes into BGRA32 `ImageData`.
///
/// # Errors
/// Returns `ImageError::DecodePanic` if `png` crate panics on malformed input.
/// Returns `ImageError::DecodeFailed` for all recoverable decode errors.
/// Caller must pre-check dimensions and budget via `ImageConfig` guards.
pub fn decode_png(data: &[u8]) -> Result<ImageData, ImageError> {
    let result = catch_unwind(AssertUnwindSafe(|| decode_png_inner(data)));
    match result {
        Ok(inner) => inner,
        Err(_)    => Err(ImageError::DecodePanic),
    }
}

fn decode_png_inner(data: &[u8]) -> Result<ImageData, ImageError> {
    use png::ColorType;

    let decoder = png::Decoder::new(std::io::Cursor::new(data));
    let mut reader = decoder.read_info().map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
    let mut buf = vec![0u8; reader.output_buffer_size()];
    let info   = reader.next_frame(&mut buf).map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
    let width  = info.width;
    let height = info.height;
    let raw    = &buf[..info.buffer_size()];

    let bgra = match info.color_type {
        ColorType::Rgba => rgba_to_bgra(raw),
        ColorType::Rgb  => rgb_to_bgra(raw),
        ColorType::GrayscaleAlpha => ga_to_bgra(raw),
        ColorType::Grayscale      => gray_to_bgra(raw),
        other => return Err(ImageError::DecodeFailed(format!("unsupported color type: {other:?}"))),
    };

    let stride = width * 4;
    Ok(ImageData {
        pixels: bgra,
        width,
        height,
        stride,
        format: PixelFormat::Bgra32,
        source_format: ImageFormat::Png,
    })
}

#[inline]
fn rgba_to_bgra(src: &[u8]) -> Vec<u8> {
    let mut out = src.to_vec();
    for px in out.chunks_exact_mut(4) { px.swap(0, 2); } // R↔B
    out
}
#[inline]
fn rgb_to_bgra(src: &[u8]) -> Vec<u8> {
    let n = src.len() / 3;
    let mut out = Vec::with_capacity(n * 4);
    for c in src.chunks_exact(3) { out.extend_from_slice(&[c[2], c[1], c[0], 255]); }
    out
}
#[inline]
fn ga_to_bgra(src: &[u8]) -> Vec<u8> {
    let n = src.len() / 2;
    let mut out = Vec::with_capacity(n * 4);
    for c in src.chunks_exact(2) { let g = c[0]; out.extend_from_slice(&[g, g, g, c[1]]); }
    out
}
#[inline]
fn gray_to_bgra(src: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 4);
    for &g in src { out.extend_from_slice(&[g, g, g, 255]); }
    out
}
```

---

## 4. JPEG decoder (`supervisor/src/image/decode_jpeg.rs`)

```rust
//! supervisor/src/image/decode_jpeg.rs
//! JPEG decode via `jpeg-decoder` (pure Rust).  Converts YCbCr → RGB → BGRA.

use std::panic::{catch_unwind, AssertUnwindSafe};
use crate::image::{ImageData, ImageError, ImageFormat, PixelFormat};

pub fn decode_jpeg(data: &[u8]) -> Result<ImageData, ImageError> {
    let result = catch_unwind(AssertUnwindSafe(|| decode_jpeg_inner(data)));
    match result {
        Ok(inner) => inner,
        Err(_)    => Err(ImageError::DecodePanic),
    }
}

fn decode_jpeg_inner(data: &[u8]) -> Result<ImageData, ImageError> {
    use jpeg_decoder::PixelFormat as JpegFmt;

    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(data));
    let raw   = decoder.decode().map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
    let meta  = decoder.info().ok_or_else(|| ImageError::DecodeFailed("no metadata".into()))?;
    let width  = meta.width  as u32;
    let height = meta.height as u32;

    let bgra = match meta.pixel_format {
        JpegFmt::RGB24 => {
            let n = raw.len() / 3;
            let mut out = Vec::with_capacity(n * 4);
            for c in raw.chunks_exact(3) {
                out.extend_from_slice(&[c[2], c[1], c[0], 255]);
            }
            out
        }
        JpegFmt::L8 => {
            let mut out = Vec::with_capacity(raw.len() * 4);
            for &g in &raw { out.extend_from_slice(&[g, g, g, 255]); }
            out
        }
        other => return Err(ImageError::DecodeFailed(format!("unsupported jpeg pixel format: {other:?}"))),
    };

    let stride = width * 4;
    Ok(ImageData {
        pixels: bgra,
        width,
        height,
        stride,
        format: PixelFormat::Bgra32,
        source_format: ImageFormat::Jpeg,
    })
}
```

---

## 5. BMP decoder (`supervisor/src/image/decode_bmp.rs`)

Hand-rolled; covers the common DIB (BITMAPINFOHEADER, 24 bpp and 32 bpp) subset. No dependency, ~200 LOC.

```rust
//! supervisor/src/image/decode_bmp.rs
//! Minimal BMP decoder: supports BITMAPINFOHEADER, 24bpp and 32bpp DIB only.
//! No external crates.  Wrapped in catch_unwind for adversarial safety.

use std::panic::{catch_unwind, AssertUnwindSafe};
use crate::image::{ImageData, ImageError, ImageFormat, PixelFormat};

pub fn decode_bmp(data: &[u8]) -> Result<ImageData, ImageError> {
    let result = catch_unwind(AssertUnwindSafe(|| decode_bmp_inner(data)));
    match result {
        Ok(inner) => inner,
        Err(_)    => Err(ImageError::DecodePanic),
    }
}

fn read_u16_le(b: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([b[off], b[off+1]])
}
fn read_u32_le(b: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([b[off], b[off+1], b[off+2], b[off+3]])
}
fn read_i32_le(b: &[u8], off: usize) -> i32 {
    i32::from_le_bytes([b[off], b[off+1], b[off+2], b[off+3]])
}

fn decode_bmp_inner(data: &[u8]) -> Result<ImageData, ImageError> {
    if data.len() < 54 {
        return Err(ImageError::DecodeFailed("file too small for BMP header".into()));
    }
    if &data[0..2] != b"BM" {
        return Err(ImageError::DecodeFailed("not a BMP file".into()));
    }
    let pixel_offset = read_u32_le(data, 10) as usize;
    let dib_size     = read_u32_le(data, 14);
    if dib_size < 40 {
        return Err(ImageError::DecodeFailed(format!("unsupported DIB header size {dib_size}")));
    }
    let width_signed  = read_i32_le(data, 18);
    let height_signed = read_i32_le(data, 22);
    let bit_count     = read_u16_le(data, 28);
    let compression   = read_u32_le(data, 30);

    if compression != 0 && compression != 3 {
        return Err(ImageError::DecodeFailed(format!("unsupported BMP compression {compression}")));
    }
    if bit_count != 24 && bit_count != 32 {
        return Err(ImageError::DecodeFailed(format!("unsupported BMP bit depth {bit_count}")));
    }

    let width  = width_signed.unsigned_abs();
    let height = height_signed.unsigned_abs();
    let top_down = height_signed < 0;
    let bpp  = (bit_count / 8) as usize;
    // BMP rows are padded to 4-byte boundaries
    let src_stride = (width as usize * bpp + 3) & !3;

    let needed = pixel_offset + src_stride * height as usize;
    if data.len() < needed {
        return Err(ImageError::DecodeFailed("pixel data truncated".into()));
    }

    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row_idx in 0..height as usize {
        // BMP is bottom-up by default unless height is negative
        let src_row = if top_down { row_idx } else { height as usize - 1 - row_idx };
        let row_start = pixel_offset + src_row * src_stride;
        let row = &data[row_start .. row_start + width as usize * bpp];
        if bpp == 3 {
            for px in row.chunks_exact(3) {
                // BMP 24bpp is already BGR in memory → add A=255 for BGRA
                pixels.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
        } else {
            // 32bpp: may be BGRA or BGRX
            for px in row.chunks_exact(4) {
                pixels.extend_from_slice(&[px[0], px[1], px[2], px[3]]);
            }
        }
    }

    Ok(ImageData {
        pixels,
        width,
        height,
        stride: width * 4,
        format: PixelFormat::Bgra32,
        source_format: ImageFormat::Bmp,
    })
}
```

---

## 6. QOI decoder (`supervisor/src/image/decode_qoi.rs`)

```rust
//! supervisor/src/image/decode_qoi.rs
//! QOI decode via the `qoi` crate.  QOI is the preferred lossless format
//! for icon/screenshot storage: ~2× faster encode than PNG, same quality.

use std::panic::{catch_unwind, AssertUnwindSafe};
use crate::image::{ImageData, ImageError, ImageFormat, PixelFormat};

pub fn decode_qoi(data: &[u8]) -> Result<ImageData, ImageError> {
    let result = catch_unwind(AssertUnwindSafe(|| decode_qoi_inner(data)));
    match result {
        Ok(inner) => inner,
        Err(_)    => Err(ImageError::DecodePanic),
    }
}

fn decode_qoi_inner(data: &[u8]) -> Result<ImageData, ImageError> {
    let (header, raw) = qoi::decode_to_vec(data)
        .map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
    let width  = header.width;
    let height = header.height;

    // QOI decodes to RGBA; convert to BGRA
    let bgra: Vec<u8> = if header.channels == qoi::Channels::Rgba {
        let mut v = raw;
        for px in v.chunks_exact_mut(4) { px.swap(0, 2); }
        v
    } else {
        // RGB — add alpha
        let n = raw.len() / 3;
        let mut v = Vec::with_capacity(n * 4);
        for c in raw.chunks_exact(3) {
            v.extend_from_slice(&[c[2], c[1], c[0], 255]);
        }
        v
    };

    Ok(ImageData {
        pixels: bgra,
        width,
        height,
        stride: width * 4,
        format: PixelFormat::Bgra32,
        source_format: ImageFormat::Qoi,
    })
}

/// Encode an `ImageData` (BGRA32) to QOI bytes.  Used by the compositor for
/// window thumbnail snapshots (Mission Control, R25).
pub fn encode_qoi(img: &ImageData) -> Result<Vec<u8>, ImageError> {
    assert_eq!(img.format, PixelFormat::Bgra32, "encode_qoi requires Bgra32");
    // Convert BGRA → RGBA for QOI encoder
    let mut rgba = img.pixels.clone();
    for px in rgba.chunks_exact_mut(4) { px.swap(0, 2); }
    qoi::encode_to_vec(&rgba, img.width, img.height)
        .map_err(|e| ImageError::DecodeFailed(e.to_string()))
}
```

---

## 7. SVG rasterizer (`supervisor/src/image/decode_svg.rs`)

Feature-gated; compiled only when `cfg(feature = "svg")`. Available on `desktop-full` and `mobile` platform profiles.

```rust
//! supervisor/src/image/decode_svg.rs  (feature = "svg")
//! SVG → BGRA32 rasterisation via `resvg` + `usvg`.
//! CPU-only.  Enabled on desktop-full and mobile profiles only.

#![cfg(feature = "svg")]

use std::panic::{catch_unwind, AssertUnwindSafe};
use crate::image::{ImageData, ImageError, ImageFormat, PixelFormat};

/// Rasterize an SVG document to BGRA32 at the requested pixel dimensions.
///
/// `target_width`/`target_height` set the output resolution.  Pass `None`
/// to use the SVG's intrinsic viewBox size (capped at `max_dimension`).
pub fn decode_svg(
    data: &[u8],
    target_width:  Option<u32>,
    target_height: Option<u32>,
    max_dimension: u32,
) -> Result<ImageData, ImageError> {
    let result = catch_unwind(AssertUnwindSafe(||
        decode_svg_inner(data, target_width, target_height, max_dimension)
    ));
    match result {
        Ok(inner) => inner,
        Err(_)    => Err(ImageError::DecodePanic),
    }
}

fn decode_svg_inner(
    data: &[u8],
    target_width:  Option<u32>,
    target_height: Option<u32>,
    max_dimension: u32,
) -> Result<ImageData, ImageError> {
    let text = std::str::from_utf8(data)
        .map_err(|e| ImageError::DecodeFailed(format!("svg not UTF-8: {e}")))?;

    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_str(text, &opt)
        .map_err(|e| ImageError::DecodeFailed(e.to_string()))?;

    let size = tree.size();
    let w = target_width .unwrap_or(size.width()  as u32).min(max_dimension);
    let h = target_height.unwrap_or(size.height() as u32).min(max_dimension);
    if w == 0 || h == 0 {
        return Err(ImageError::InvalidDimensions { width: w, height: h });
    }

    let mut pixmap = tiny_skia::Pixmap::new(w, h)
        .ok_or_else(|| ImageError::DecodeFailed("pixmap alloc failed".into()))?;

    let sx = w as f32 / size.width() as f32;
    let sy = h as f32 / size.height() as f32;
    resvg::render(&tree, tiny_skia::Transform::from_scale(sx, sy), &mut pixmap.as_mut());

    // tiny-skia produces pre-multiplied RGBA; convert to straight BGRA
    let mut pixels = pixmap.take();
    for px in pixels.chunks_exact_mut(4) {
        let a = px[3];
        if a > 0 {
            // un-premultiply
            let inv = 255.0 / a as f32;
            let r = ((px[0] as f32 * inv) as u8).min(255);
            let g = ((px[1] as f32 * inv) as u8).min(255);
            let b = ((px[2] as f32 * inv) as u8).min(255);
            px[0] = b; px[1] = g; px[2] = r; // R→B swap → BGRA
        }
    }

    Ok(ImageData {
        pixels,
        width:  w,
        height: h,
        stride: w * 4,
        format: PixelFormat::Bgra32,
        source_format: ImageFormat::Svg,
    })
}
```

---

## 8. Resize engine (`supervisor/src/image/resize.rs`)

```rust
//! supervisor/src/image/resize.rs
//! Three resize algorithms: nearest-neighbour, bilinear, Lanczos3.
//! All operate on BGRA32 ImageData.  Bilinear and Lanczos3 perform
//! gamma-correct resize: sRGB → linear → resample → sRGB.

use crate::image::{ImageData, ImageError, PixelFormat, ResizeFilter};

/// Resize `src` to `(dst_w, dst_h)` using the specified filter.
/// Returns a new `ImageData`; `src` is not modified.
pub fn resize(
    src: &ImageData,
    dst_w: u32,
    dst_h: u32,
    filter: ResizeFilter,
) -> Result<ImageData, ImageError> {
    if dst_w == 0 || dst_h == 0 {
        return Err(ImageError::InvalidDimensions { width: dst_w, height: dst_h });
    }
    assert_eq!(src.format, PixelFormat::Bgra32, "resize requires Bgra32 input");
    let pixels = match filter {
        ResizeFilter::Nearest  => resize_nearest(src, dst_w, dst_h),
        ResizeFilter::Bilinear => resize_bilinear(src, dst_w, dst_h),
        ResizeFilter::Lanczos3 => resize_lanczos3(src, dst_w, dst_h),
    };
    Ok(ImageData {
        pixels,
        width:  dst_w,
        height: dst_h,
        stride: dst_w * 4,
        format: PixelFormat::Bgra32,
        source_format: src.source_format,
    })
}

// ── sRGB ↔ linear LUTs ────────────────────────────────────────────────────────

fn build_srgb_to_linear_lut() -> [f32; 256] {
    let mut lut = [0f32; 256];
    for (i, v) in lut.iter_mut().enumerate() {
        let c = i as f32 / 255.0;
        *v = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055_f32).powf(2.4) };
    }
    lut
}
fn linear_to_srgb(c: f32) -> u8 {
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0/2.4) - 0.055 };
    (s.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

// ── nearest neighbour ─────────────────────────────────────────────────────────

fn resize_nearest(src: &ImageData, dw: u32, dh: u32) -> Vec<u8> {
    let mut out = vec![0u8; dw as usize * dh as usize * 4];
    let sw = src.width as f32;
    let sh = src.height as f32;
    for dy in 0..dh {
        let sy = ((dy as f32 + 0.5) * sh / dh as f32) as u32;
        let sy = sy.min(src.height - 1);
        for dx in 0..dw {
            let sx = ((dx as f32 + 0.5) * sw / dw as f32) as u32;
            let sx = sx.min(src.width - 1);
            let si = (sy * src.stride + sx * 4) as usize;
            let di = (dy * dw + dx) as usize * 4;
            out[di..di+4].copy_from_slice(&src.pixels[si..si+4]);
        }
    }
    out
}

// ── bilinear (gamma-correct) ──────────────────────────────────────────────────

fn resize_bilinear(src: &ImageData, dw: u32, dh: u32) -> Vec<u8> {
    let lut = build_srgb_to_linear_lut();
    let mut out = vec![0u8; dw as usize * dh as usize * 4];
    let x_ratio = src.width  as f32 / dw as f32;
    let y_ratio = src.height as f32 / dh as f32;

    for dy in 0..dh {
        let fy = (dy as f32 + 0.5) * y_ratio - 0.5;
        let y0 = (fy as i32).max(0) as u32;
        let y1 = (y0 + 1).min(src.height - 1);
        let ty = fy.fract().max(0.0);

        for dx in 0..dw {
            let fx = (dx as f32 + 0.5) * x_ratio - 0.5;
            let x0 = (fx as i32).max(0) as u32;
            let x1 = (x0 + 1).min(src.width - 1);
            let tx = fx.fract().max(0.0);

            let s00 = px4(src, x0, y0);
            let s10 = px4(src, x1, y0);
            let s01 = px4(src, x0, y1);
            let s11 = px4(src, x1, y1);

            let di = (dy * dw + dx) as usize * 4;
            for ch in 0..3usize {
                // channel 3 = alpha: bilinear without gamma
                let v = if ch == 3 {
                    lerp2d(s00[ch] as f32/255.0, s10[ch] as f32/255.0,
                           s01[ch] as f32/255.0, s11[ch] as f32/255.0, tx, ty)
                } else {
                    let l00 = lut[s00[ch] as usize];
                    let l10 = lut[s10[ch] as usize];
                    let l01 = lut[s01[ch] as usize];
                    let l11 = lut[s11[ch] as usize];
                    lerp2d(l00, l10, l01, l11, tx, ty)
                };
                if ch == 3 {
                    out[di+ch] = (v.clamp(0.0,1.0)*255.0+0.5) as u8;
                } else {
                    out[di+ch] = linear_to_srgb(v);
                }
            }
        }
    }
    out
}

#[inline]
fn px4(src: &ImageData, x: u32, y: u32) -> [u8; 4] {
    let off = (y * src.stride + x * 4) as usize;
    [src.pixels[off], src.pixels[off+1], src.pixels[off+2], src.pixels[off+3]]
}
#[inline]
fn lerp2d(v00: f32, v10: f32, v01: f32, v11: f32, tx: f32, ty: f32) -> f32 {
    let top = v00 + (v10 - v00) * tx;
    let bot = v01 + (v11 - v01) * tx;
    top + (bot - top) * ty
}

// ── Lanczos3 ──────────────────────────────────────────────────────────────────

fn lanczos3_kernel(x: f32) -> f32 {
    const A: f32 = 3.0;
    if x.abs() < 1e-6  { return 1.0; }
    if x.abs() >= A    { return 0.0; }
    let px = std::f32::consts::PI * x;
    let pa = std::f32::consts::PI * x / A;
    (px.sin() / px) * (pa.sin() / pa)
}

fn resize_lanczos3(src: &ImageData, dw: u32, dh: u32) -> Vec<u8> {
    // Two-pass: horizontal then vertical, both in linear light.
    let lut = build_srgb_to_linear_lut();
    let x_ratio = src.width  as f32 / dw as f32;
    let y_ratio = src.height as f32 / dh as f32;

    // Pass 1: horizontal resize → intermediate (dw × src.height)
    let inter_h = src.height as usize;
    let inter_w = dw as usize;
    let mut inter = vec![[0f32; 4]; inter_w * inter_h];
    for iy in 0..inter_h {
        for ix in 0..inter_w {
            let cx = (ix as f32 + 0.5) * x_ratio - 0.5;
            let x0 = (cx as i32 - 2).max(0) as u32;
            let x1 = ((cx as i32 + 3) as u32).min(src.width - 1);
            let mut acc = [0f32; 4]; let mut wsum = 0f32;
            for sx in x0..=x1 {
                let w = lanczos3_kernel(cx - sx as f32);
                let p = px4(src, sx, iy as u32);
                for ch in 0..4 {
                    let v = if ch == 3 { p[ch] as f32/255.0 } else { lut[p[ch] as usize] };
                    acc[ch] += v * w;
                }
                wsum += w;
            }
            let inv = if wsum.abs() > 1e-9 { 1.0/wsum } else { 1.0 };
            for ch in 0..4 { acc[ch] *= inv; }
            inter[iy * inter_w + ix] = acc;
        }
    }

    // Pass 2: vertical resize → output
    let mut out = vec![0u8; dw as usize * dh as usize * 4];
    for dy in 0..dh {
        let cy = (dy as f32 + 0.5) * y_ratio - 0.5;
        let y0 = (cy as i32 - 2).max(0) as usize;
        let y1 = ((cy as i32 + 3) as usize).min(inter_h - 1);
        for dx in 0..dw as usize {
            let mut acc = [0f32; 4]; let mut wsum = 0f32;
            for sy in y0..=y1 {
                let w = lanczos3_kernel(cy - sy as f32);
                let p = inter[sy * inter_w + dx];
                for ch in 0..4 { acc[ch] += p[ch] * w; }
                wsum += w;
            }
            let inv = if wsum.abs() > 1e-9 { 1.0/wsum } else { 1.0 };
            let di = (dy as usize * dw as usize + dx) * 4;
            for ch in 0..3 { out[di+ch] = linear_to_srgb(acc[ch] * inv); }
            out[di+3] = ((acc[3] * inv).clamp(0.0,1.0)*255.0+0.5) as u8;
        }
    }
    out
}
```

---

## 9. Image & GPU cache (`supervisor/src/image/cache.rs`)

```rust
//! supervisor/src/image/cache.rs
//! Two-tier LRU cache: ImageCache (CPU, decoded BGRA) + GpuImageCache (GPU texture handles).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::image::{ImageData, ImageError, PixelFormat};

// ── CPU image cache ───────────────────────────────────────────────────────────

/// Cache key: content-addresses decoded images regardless of decode path.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageCacheKey {
    pub path_hash: u64,
    pub width:     u32,
    pub height:    u32,
    pub format:    PixelFormat,
}

struct CacheEntry {
    data:       Arc<ImageData>,
    byte_size:  usize,
    /// Monotonic generation counter for LRU ordering.
    last_used:  u64,
}

pub struct ImageCache {
    entries:          HashMap<ImageCacheKey, CacheEntry>,
    total_bytes:      usize,
    generation:       u64,
    high_water_bytes: usize,
    low_water_bytes:  usize,
}

impl ImageCache {
    pub fn new(high_water_bytes: usize, low_water_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            total_bytes: 0,
            generation: 0,
            high_water_bytes,
            low_water_bytes,
        }
    }

    /// Look up; bumps LRU generation on hit.
    pub fn get(&mut self, key: &ImageCacheKey) -> Option<Arc<ImageData>> {
        if let Some(e) = self.entries.get_mut(key) {
            self.generation += 1;
            e.last_used = self.generation;
            Some(Arc::clone(&e.data))
        } else {
            None
        }
    }

    /// Insert decoded image.  May trigger LRU eviction.
    pub fn insert(&mut self, key: ImageCacheKey, data: Arc<ImageData>) {
        if self.high_water_bytes == 0 { return; }
        let sz = data.byte_size();
        if sz > self.high_water_bytes { return; } // single image too large to cache
        self.generation += 1;
        self.entries.insert(key, CacheEntry {
            data, byte_size: sz, last_used: self.generation,
        });
        self.total_bytes += sz;
        self.maybe_evict();
    }

    /// Invalidate all entries for a given path hash (file changed on disk).
    pub fn invalidate_path(&mut self, path_hash: u64) {
        self.entries.retain(|k, e| {
            if k.path_hash == path_hash {
                self.total_bytes = self.total_bytes.saturating_sub(e.byte_size);
                false
            } else {
                true
            }
        });
    }

    fn maybe_evict(&mut self) {
        while self.total_bytes > self.high_water_bytes {
            if let Some(lru_key) = self.lru_key() {
                if let Some(e) = self.entries.remove(&lru_key) {
                    self.total_bytes = self.total_bytes.saturating_sub(e.byte_size);
                }
            } else {
                break;
            }
            if self.total_bytes <= self.low_water_bytes { break; }
        }
    }

    fn lru_key(&self) -> Option<ImageCacheKey> {
        self.entries.iter()
            .min_by_key(|(_, e)| e.last_used)
            .map(|(k, _)| k.clone())
    }

    pub fn total_bytes(&self) -> usize { self.total_bytes }
}

// ── GPU texture cache ─────────────────────────────────────────────────────────

/// Handle type returned by R12's `vyoma:gpu/graphics.create_texture`.
pub type TextureHandle = u32;

/// Maps decoded `ImageData` keys to GPU texture handles.
/// Eviction is demand-driven: call `evict_lru()` when VRAM pressure is signalled.
pub struct GpuImageCache {
    inner: Arc<Mutex<GpuCacheInner>>,
}

struct GpuCacheInner {
    map:        HashMap<ImageCacheKey, (TextureHandle, u64)>,
    generation: u64,
}

impl GpuImageCache {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(GpuCacheInner { map: HashMap::new(), generation: 0 })) }
    }

    pub fn get(&self, key: &ImageCacheKey) -> Option<TextureHandle> {
        let mut g = self.inner.lock().unwrap();
        g.generation += 1;
        if let Some((handle, ts)) = g.map.get_mut(key) {
            *ts = g.generation;
            Some(*handle)
        } else {
            None
        }
    }

    pub fn insert(&self, key: ImageCacheKey, handle: TextureHandle) {
        let mut g = self.inner.lock().unwrap();
        g.generation += 1;
        g.map.insert(key, (handle, g.generation));
    }

    /// Evict the `n` least-recently-used textures.
    /// Caller is responsible for calling the GPU destroy-texture hostcall.
    pub fn evict_lru(&self, n: usize) -> Vec<TextureHandle> {
        let mut g = self.inner.lock().unwrap();
        let mut entries: Vec<_> = g.map.iter().map(|(k,(h,ts))| (k.clone(), *h, *ts)).collect();
        entries.sort_by_key(|(_,_,ts)| *ts);
        let to_evict: Vec<_> = entries.into_iter().take(n).collect();
        let handles: Vec<_> = to_evict.iter().map(|(_,h,_)| *h).collect();
        for (k,_,_) in &to_evict { g.map.remove(k); }
        handles
    }

    pub fn invalidate_path(&self, path_hash: u64) -> Vec<TextureHandle> {
        let mut g = self.inner.lock().unwrap();
        let mut removed = Vec::new();
        g.map.retain(|k, (h,_)| {
            if k.path_hash == path_hash { removed.push(*h); false } else { true }
        });
        removed
    }
}

/// Compute a fast non-cryptographic hash of a file path string.
pub fn hash_path(path: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    h.finish()
}
```

---

## 10. Icon system (`supervisor/src/image/icon.rs`)

```rust
//! supervisor/src/image/icon.rs
//! IconSet loading, multi-resolution selection, and the supervisor-global
//! icon LRU cache shared across all apps.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use crate::image::{
    IconSet, ImageData, ImageError, ImageFormat, ICON_STANDARD_SIZES,
    ResizeFilter,
    worker::ImageWorkerHandle,
};

/// LRU entry in the system icon cache.
struct IconCacheEntry {
    img:       Arc<ImageData>,
    last_used: u64,
}

/// Supervisor-global icon cache shared across all apps.
/// Key: `(path_hash, logical_px, scale_numerator)`.
pub struct SystemIconCache {
    inner: Mutex<IconCacheInner>,
}

struct IconCacheInner {
    map:             HashMap<(u64, u32, u8), IconCacheEntry>,
    total_bytes:     usize,
    generation:      u64,
    high_water_bytes: usize,
    low_water_bytes:  usize,
}

impl SystemIconCache {
    pub fn new(high_water: usize, low_water: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(IconCacheInner {
                map:              HashMap::new(),
                total_bytes:      0,
                generation:       0,
                high_water_bytes: high_water,
                low_water_bytes:  low_water,
            }),
        })
    }

    pub fn get(
        &self,
        path_hash:  u64,
        logical_px: u32,
        scale:      u8,
    ) -> Option<Arc<ImageData>> {
        let mut g = self.inner.lock().unwrap();
        let key = (path_hash, logical_px, scale);
        if let Some(e) = g.map.get_mut(&key) {
            g.generation += 1;
            e.last_used = g.generation;
            Some(Arc::clone(&e.img))
        } else {
            None
        }
    }

    pub fn insert(
        &self,
        path_hash:  u64,
        logical_px: u32,
        scale:      u8,
        img:        Arc<ImageData>,
    ) {
        let mut g = self.inner.lock().unwrap();
        g.generation += 1;
        let sz = img.byte_size();
        g.map.insert((path_hash, logical_px, scale), IconCacheEntry { img, last_used: g.generation });
        g.total_bytes += sz;
        while g.total_bytes > g.high_water_bytes {
            if let Some(k) = g.lru_key() {
                if let Some(e) = g.map.remove(&k) {
                    g.total_bytes = g.total_bytes.saturating_sub(e.img.byte_size());
                }
            } else {
                break;
            }
            if g.total_bytes <= g.low_water_bytes { break; }
        }
    }
}

impl IconCacheInner {
    fn lru_key(&self) -> Option<(u64, u32, u8)> {
        self.map.iter().min_by_key(|(_, e)| e.last_used).map(|(k,_)| *k)
    }
}

// ── IconSet loader ─────────────────────────────────────────────────────────────

/// Scan `bundle_dir` for PNG files named `<size>.png` or `<size>@2x.png`,
/// decode each via the image worker, and return a fully-populated `IconSet`.
///
/// Accepted filenames: `16.png`, `32@2x.png`, `icon_256.png`, `icon_256@2x.png`.
pub fn load_icon_set(
    app_name:   &str,
    bundle_dir: &Path,
    worker:     &ImageWorkerHandle,
) -> Result<IconSet, ImageError> {
    if !bundle_dir.exists() {
        return Err(ImageError::IconBundleNotFound(bundle_dir.display().to_string()));
    }

    let mut variants: HashMap<(u32, u8), Arc<ImageData>> = HashMap::new();

    let entries = std::fs::read_dir(bundle_dir)
        .map_err(|e| ImageError::IconBundleNotFound(e.to_string()))?;

    for entry in entries.flatten() {
        let path = entry.path();
        let fname = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
        if path.extension().map(|e| e != "png").unwrap_or(true) { continue; }

        if let Some((size, scale)) = parse_icon_filename(&fname) {
            let data = worker.decode_path(path.to_string_lossy().as_ref(), Some(ImageFormat::Png))?;
            variants.insert((size, scale), Arc::new(data));
        }
    }

    if variants.is_empty() {
        return Err(ImageError::IconBundleNotFound(
            format!("no valid icon PNGs in {}", bundle_dir.display())
        ));
    }

    // Synthesize missing standard sizes by downscaling the next larger variant.
    for &sz in ICON_STANDARD_SIZES {
        for &sc in &[1u8, 2] {
            if variants.contains_key(&(sz, sc)) { continue; }
            if let Some(src) = find_nearest_larger(&variants, sz, sc) {
                let target = (sz * sc as u32).max(1);
                let resized = crate::image::resize::resize(&src, target, target, ResizeFilter::Lanczos3)?;
                variants.insert((sz, sc), Arc::new(resized));
            }
        }
    }

    Ok(IconSet {
        app_name:    app_name.to_string(),
        variants,
        bundle_path: bundle_dir.display().to_string(),
    })
}

fn parse_icon_filename(stem: &str) -> Option<(u32, u8)> {
    // Accepts: "16", "32@2x", "icon_256", "icon_256@2x"
    let s = stem.trim_start_matches("icon_");
    let (size_part, scale) = if let Some(idx) = s.find('@') {
        let sc = if s[idx+1..].starts_with('2') { 2u8 } else { 3u8 };
        (&s[..idx], sc)
    } else {
        (s, 1u8)
    };
    size_part.parse::<u32>().ok().map(|sz| (sz, scale))
}

fn find_nearest_larger(
    variants: &HashMap<(u32, u8), Arc<ImageData>>,
    target_sz: u32,
    scale: u8,
) -> Option<Arc<ImageData>> {
    ICON_STANDARD_SIZES.iter()
        .filter(|&&s| s > target_sz)
        .find_map(|&s| variants.get(&(s, scale)).cloned())
        .or_else(|| {
            ICON_STANDARD_SIZES.iter().rev()
                .find_map(|&s| variants.get(&(s, 1)).cloned())
        })
}
```

---

## 11. Image worker (`supervisor/src/image/worker.rs`)

Pattern mirrors `FontWorker` from R13: bounded channel, watchdog thread, `catch_unwind` per operation.

```rust
//! supervisor/src/image/worker.rs
//! Dedicated decode worker + watchdog monitor — same pattern as R13 FontWorker.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;
use crate::image::{
    ImageData, ImageError, ImageFormat, ResizeFilter,
    config::ImageConfig,
    decode_png, decode_jpeg, decode_bmp, decode_qoi,
};
#[cfg(feature = "svg")]
use crate::image::decode_svg;

const CHANNEL_CAPACITY: usize  = 32;
const RECV_TIMEOUT_MS:  u64    = 500;

// ── request / reply ───────────────────────────────────────────────────────────

pub type Reply = mpsc::SyncSender<Result<ImageData, ImageError>>;

pub enum ImageReq {
    Decode {
        path:        String,
        format_hint: Option<ImageFormat>,
        reply:       Reply,
    },
    DecodeBytes {
        data:        Vec<u8>,
        format_hint: Option<ImageFormat>,
        reply:       Reply,
    },
    Resize {
        src:    ImageData,
        filter: ResizeFilter,
        width:  u32,
        height: u32,
        reply:  Reply,
    },
    Shutdown,
}

// ── worker state ──────────────────────────────────────────────────────────────

pub struct ImageWorker {
    config: Arc<ImageConfig>,
}

impl ImageWorker {
    pub fn new(config: Arc<ImageConfig>) -> Self { Self { config } }

    fn run(self, rx: mpsc::Receiver<ImageReq>) {
        for req in rx {
            match req {
                ImageReq::Shutdown => break,
                ImageReq::Decode { path, format_hint, reply } => {
                    let res = self.handle_decode_path(&path, format_hint);
                    let _ = reply.send(res);
                }
                ImageReq::DecodeBytes { data, format_hint, reply } => {
                    let res = self.handle_decode_bytes(&data, format_hint);
                    let _ = reply.send(res);
                }
                ImageReq::Resize { src, filter, width, height, reply } => {
                    let res = catch_unwind(AssertUnwindSafe(||
                        crate::image::resize::resize(&src, width, height, filter)
                    )).unwrap_or(Err(ImageError::DecodePanic));
                    let _ = reply.send(res);
                }
            }
        }
    }

    fn handle_decode_path(
        &self,
        path: &str,
        hint: Option<ImageFormat>,
    ) -> Result<ImageData, ImageError> {
        let data = std::fs::read(path)
            .map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
        self.handle_decode_bytes(&data, hint)
    }

    fn handle_decode_bytes(
        &self,
        data: &[u8],
        hint: Option<ImageFormat>,
    ) -> Result<ImageData, ImageError> {
        let fmt = hint
            .or_else(|| ImageFormat::from_magic(data))
            .ok_or(ImageError::DecodeFailed("unknown format".into()))?;

        if !self.config.formats_enabled.contains(&fmt) {
            return Err(ImageError::UnsupportedFormat(fmt));
        }

        // Pre-alloc size estimate for budget check
        let approx_bytes = data.len() * 6; // worst-case expansion
        self.config.check_budget(approx_bytes, 0)?;

        match fmt {
            ImageFormat::Png  => decode_png::decode_png(data),
            ImageFormat::Jpeg => decode_jpeg::decode_jpeg(data),
            ImageFormat::Bmp  => decode_bmp::decode_bmp(data),
            ImageFormat::Qoi  => decode_qoi::decode_qoi(data),
            #[cfg(feature = "svg")]
            ImageFormat::Svg  => decode_svg::decode_svg(data, None, None, self.config.max_image_dimension),
            #[cfg(not(feature = "svg"))]
            ImageFormat::Svg  => Err(ImageError::UnsupportedFormat(ImageFormat::Svg)),
        }
    }
}

// ── handle (caller side) ──────────────────────────────────────────────────────

/// Clone-able handle given to each app host-call context.
#[derive(Clone)]
pub struct ImageWorkerHandle {
    tx: mpsc::SyncSender<ImageReq>,
}

impl ImageWorkerHandle {
    pub fn decode_path(
        &self,
        path: &str,
        fmt:  Option<ImageFormat>,
    ) -> Result<ImageData, ImageError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx.send(ImageReq::Decode {
            path: path.to_string(), format_hint: fmt, reply: reply_tx,
        }).map_err(|_| ImageError::WorkerPanic)?;
        reply_rx.recv_timeout(Duration::from_millis(RECV_TIMEOUT_MS))
            .map_err(|_| ImageError::WorkerTimeout)
            .and_then(|r| r)
    }

    pub fn decode_bytes(
        &self,
        data: Vec<u8>,
        fmt:  Option<ImageFormat>,
    ) -> Result<ImageData, ImageError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx.send(ImageReq::DecodeBytes {
            data, format_hint: fmt, reply: reply_tx,
        }).map_err(|_| ImageError::WorkerPanic)?;
        reply_rx.recv_timeout(Duration::from_millis(RECV_TIMEOUT_MS))
            .map_err(|_| ImageError::WorkerTimeout)
            .and_then(|r| r)
    }

    pub fn resize(
        &self,
        src:    ImageData,
        filter: ResizeFilter,
        width:  u32,
        height: u32,
    ) -> Result<ImageData, ImageError> {
        let (reply_tx, reply_rx) = mpsc::sync_channel(1);
        self.tx.send(ImageReq::Resize { src, filter, width, height, reply: reply_tx })
            .map_err(|_| ImageError::WorkerPanic)?;
        reply_rx.recv_timeout(Duration::from_millis(RECV_TIMEOUT_MS))
            .map_err(|_| ImageError::WorkerTimeout)
            .and_then(|r| r)
    }
}

// ── monitor / watchdog ────────────────────────────────────────────────────────

/// Spawn the image worker + watchdog.  Returns a handle for callers.
pub fn spawn_image_worker(config: Arc<ImageConfig>) -> ImageWorkerHandle {
    let (tx, rx) = mpsc::sync_channel::<ImageReq>(CHANNEL_CAPACITY);
    let handle = ImageWorkerHandle { tx: tx.clone() };
    let cfg = Arc::clone(&config);

    std::thread::Builder::new()
        .name("img-monitor".into())
        .spawn(move || {
            loop {
                let (wtx, wrx) = mpsc::sync_channel::<ImageReq>(CHANNEL_CAPACITY);
                // Drain tx into wtx while worker is live (forwarding proxy)
                let cfg2 = Arc::clone(&cfg);
                let worker_thread = std::thread::Builder::new()
                    .name("img-worker".into())
                    .spawn(move || {
                        ImageWorker::new(cfg2).run(wrx);
                    })
                    .expect("failed to spawn image worker");

                // Forward requests from public tx to internal wtx
                // until worker dies or Shutdown is received.
                let mut shutdown = false;
                loop {
                    match rx.recv() {
                        Err(_) | Ok(ImageReq::Shutdown) => { shutdown = true; break; }
                        Ok(req) => {
                            if wtx.send(req).is_err() {
                                // Worker channel closed — it panicked; break to respawn
                                break;
                            }
                        }
                    }
                }
                let _ = wtx.send(ImageReq::Shutdown);
                let _ = worker_thread.join();
                if shutdown { break; }
                eprintln!("[image] worker panicked — respawning");
            }
        })
        .expect("failed to spawn image monitor");

    handle
}
```

---

## 12. Surface extensions (`supervisor/src/image/surface_ext.rs`)

```rust
//! supervisor/src/image/surface_ext.rs
//! Extends `Surface` (R11) with ImageData blit and construction paths.

use crate::display::Surface;
use crate::image::{ImageData, ImageError, PixelFormat};

impl Surface {
    /// Blit an `ImageData` (BGRA32) onto this surface at `(dst_x, dst_y)`.
    /// Clips to surface bounds.  Alpha channel is composited (straight-over).
    pub fn blit_rgba(
        &mut self,
        src: &ImageData,
        dst_x: i32,
        dst_y: i32,
    ) -> Result<(), ImageError> {
        if src.format != PixelFormat::Bgra32 {
            return Err(ImageError::DecodeFailed(
                format!("blit_rgba requires Bgra32, got {:?}", src.format)
            ));
        }
        let surf_w = self.width() as i32;
        let surf_h = self.height() as i32;
        let img_w  = src.width as i32;
        let img_h  = src.height as i32;

        for row in 0..img_h {
            let dy = dst_y + row;
            if dy < 0 || dy >= surf_h { continue; }
            for col in 0..img_w {
                let dx = dst_x + col;
                if dx < 0 || dx >= surf_w { continue; }

                let si = (row * src.stride as i32 + col * 4) as usize;
                let sb = src.pixels[si];
                let sg = src.pixels[si+1];
                let sr = src.pixels[si+2];
                let sa = src.pixels[si+3];

                if sa == 0 { continue; }
                if sa == 255 {
                    self.put_pixel_bgra(dx as u32, dy as u32, sb, sg, sr, sa);
                } else {
                    let (db, dg, dr, _da) = self.get_pixel_bgra(dx as u32, dy as u32);
                    let inv = 255 - sa;
                    let out_b = ((sb as u32 * sa as u32 + db as u32 * inv as u32) / 255) as u8;
                    let out_g = ((sg as u32 * sa as u32 + dg as u32 * inv as u32) / 255) as u8;
                    let out_r = ((sr as u32 * sa as u32 + dr as u32 * inv as u32) / 255) as u8;
                    self.put_pixel_bgra(dx as u32, dy as u32, out_b, out_g, out_r, 255);
                }
            }
        }
        Ok(())
    }

    /// Construct a `Surface` with its pixel buffer copied from `ImageData`.
    pub fn from_image_data(data: &ImageData) -> Result<Surface, ImageError> {
        if data.format != PixelFormat::Bgra32 {
            return Err(ImageError::DecodeFailed(
                format!("from_image_data requires Bgra32, got {:?}", data.format)
            ));
        }
        Ok(Surface::from_raw_bgra(data.width, data.height, data.pixels.clone()))
    }

    /// Snapshot this surface to an `ImageData` for Mission Control thumbnail (R25).
    pub fn snapshot(&self) -> ImageData {
        ImageData {
            pixels: self.raw_bgra().to_vec(),
            width:  self.width(),
            height: self.height(),
            stride: self.width() * 4,
            format: PixelFormat::Bgra32,
            source_format: crate::image::ImageFormat::Qoi, // will be QOI-compressed
        }
    }
}
```

---

## 13. Manifest capability (`supervisor/src/image/manifest.rs`)

```rust
//! supervisor/src/image/manifest.rs
//! `[capabilities.images]` TOML parsing and path validation.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use serde::Deserialize;
use crate::image::ImageError;

/// Parsed from `[capabilities.images]` in an app's `vyoma.toml`.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct ImagesCapability {
    /// Allowed decode format names: "png", "jpeg", "qoi", "bmp", "svg".
    #[serde(default)]
    pub decode: Vec<String>,
    /// Total decode budget in MiB.  Overrides platform default if lower.
    #[serde(default)]
    pub max_decode_mb: Option<u64>,
    /// Icon bundle directories (relative to app root).
    #[serde(default)]
    pub icons: Vec<String>,
}

impl ImagesCapability {
    /// Return the set of allowed format names as a `HashSet` for O(1) lookup.
    pub fn allowed_formats(&self) -> HashSet<String> {
        self.decode.iter().cloned().collect()
    }

    /// Effective decode budget in bytes.
    /// Takes the minimum of the manifest limit and platform config limit.
    pub fn effective_budget_bytes(&self, platform_max: usize) -> usize {
        if let Some(mb) = self.max_decode_mb {
            let cap = mb as usize * 1024 * 1024;
            cap.min(platform_max)
        } else {
            platform_max
        }
    }

    /// Validate that `path` is within one of the allowed `base_dirs`.
    /// Returns `Err(ImageError::PathTraversal)` if path escapes.
    pub fn validate_path(
        &self,
        path: &str,
        app_root: &Path,
    ) -> Result<PathBuf, ImageError> {
        let canonical_root = app_root.canonicalize()
            .map_err(|_| ImageError::PathTraversal(app_root.display().to_string()))?;

        // Also allow `/data` (persistent filesystem mount)
        let data_root = PathBuf::from("/data");

        let full = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            app_root.join(path)
        };

        let canonical_full = full.canonicalize()
            .map_err(|_| ImageError::PathTraversal(path.to_string()))?;

        if canonical_full.starts_with(&canonical_root)
            || canonical_full.starts_with(&data_root)
        {
            Ok(canonical_full)
        } else {
            Err(ImageError::PathTraversal(path.to_string()))
        }
    }

    /// Validate icon bundle paths and return canonical absolute paths.
    pub fn canonical_icon_dirs(&self, app_root: &Path) -> Vec<PathBuf> {
        self.icons.iter().filter_map(|rel| {
            let p = app_root.join(rel);
            p.canonicalize().ok().filter(|c| c.starts_with(app_root))
        }).collect()
    }
}

// ── TOML integration test ──────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_images_capability() {
        let toml_str = r#"
            decode = ["png", "jpeg", "qoi"]
            max_decode_mb = 64
            icons = ["icons/"]
        "#;
        let cap: ImagesCapability = toml::from_str(toml_str).unwrap();
        assert_eq!(cap.decode.len(), 3);
        assert_eq!(cap.max_decode_mb, Some(64));
        assert_eq!(cap.icons, vec!["icons/"]);
    }

    #[test]
    fn empty_capability_is_valid() {
        let cap: ImagesCapability = toml::from_str("").unwrap();
        assert!(cap.decode.is_empty());
        assert!(cap.max_decode_mb.is_none());
    }
}
```

---

## 14. WIT interface (`wit/vyoma-images.wit`)

```wit
// wit/vyoma-images.wit
// VyomaOS Image & Icon Pipeline — WIT definition for WASM app host-calls.
// Part of the vyoma:images@1.0.0 package.

package vyoma:images@1.0.0;

/// Image I/O: decode, blit, GPU upload.
interface io {
    /// Decode an image from a filesystem path.
    /// Returns an opaque handle (u32) valid for the lifetime of the call context.
    /// Errors: path-traversal, unsupported-format, decode-failed, too-large, worker-timeout.
    load-image: func(path: string) -> result<u32, string>;

    /// Decode an image from an in-memory byte buffer.
    /// format-hint: 0 = auto-detect, 1 = PNG, 2 = JPEG, 3 = BMP, 4 = QOI, 5 = SVG.
    decode-image: func(data: list<u8>, format-hint: u8) -> result<u32, string>;

    /// Invalidate and free the image handle.  Noop for unknown handles.
    unload-image: func(handle: u32);

    /// Return (width, height) in pixels of the decoded image.
    image-size: func(handle: u32) -> result<tuple<u32, u32>, string>;

    /// Blit the image onto the app's current window surface.
    /// dst-w and dst-h: if both are 0, blit at 1:1 pixel scale.
    /// If non-zero, scale to the requested dimensions using the active resize filter.
    blit-image: func(handle: u32, dst-x: i32, dst-y: i32, dst-w: u32, dst-h: u32) -> result<_, string>;

    /// Upload image pixels to a GPU texture via the R12 graphics device.
    /// Returns a TextureHandle for use with vyoma:gpu/graphics.
    /// Only available on desktop-full and mobile profiles; returns error otherwise.
    upload-texture: func(handle: u32) -> result<u32, string>;

    /// Set the active resize filter for subsequent blit-image calls.
    /// filter: 0 = Nearest, 1 = Bilinear, 2 = Lanczos3.
    set-resize-filter: func(filter: u8) -> result<_, string>;
}

/// Icon system: multi-resolution icon bundle access.
interface icons {
    /// Load an icon set from a bundle directory path.
    /// The directory must be listed in capabilities.images.icons in vyoma.toml.
    load-icon-set: func(bundle-path: string) -> result<u32, string>;

    /// Get the best-fit image handle for a logical size and HiDPI scale.
    /// Returns an image handle usable with the io interface.
    icon-for-size: func(set: u32, logical-px: u32, scale: f32) -> result<u32, string>;

    /// Free an icon set handle.  Does not evict the underlying image cache.
    unload-icon-set: func(set: u32);

    /// Blit the best-fit icon directly onto the window surface.
    /// Convenience wrapper: icon-for-size + blit-image at (x, y) with icon size.
    blit-icon: func(set: u32, logical-px: u32, scale: f32, x: i32, y: i32) -> result<_, string>;
}

/// World imported by every WASM app that requests image capabilities.
world vyoma-app {
    import io;
    import icons;
}
```

---

## 15. WIT host-call handlers (`supervisor/src/image/wit_handlers.rs`)

```rust
//! supervisor/src/image/wit_handlers.rs
//! Wasmtime linker bindings for vyoma:images/io and vyoma:images/icons.
//! One function per WIT export; all are registered via `wasmtime::Linker::func_wrap`.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use wasmtime::Linker;
use crate::image::{
    ImageData, ImageError, ImageFormat, ResizeFilter,
    cache::{GpuImageCache, ImageCache, hash_path},
    config::ImageConfig,
    icon::{SystemIconCache, load_icon_set},
    manifest::ImagesCapability,
    worker::ImageWorkerHandle,
};

/// Per-app image host-call state.
pub struct AppImageState {
    pub worker:     ImageWorkerHandle,
    pub config:     Arc<ImageConfig>,
    pub capability: ImagesCapability,
    pub app_root:   std::path::PathBuf,
    pub cpu_cache:  ImageCache,
    pub gpu_cache:  GpuImageCache,
    pub icon_cache: Arc<SystemIconCache>,
    pub handles:    HashMap<u32, Arc<ImageData>>,
    pub icon_sets:  HashMap<u32, crate::image::IconSet>,
    pub next_handle: u32,
    pub bytes_used:  usize,
    pub resize_filter: ResizeFilter,
}

impl AppImageState {
    pub fn alloc_handle(&mut self, data: ImageData) -> u32 {
        let h = self.next_handle;
        self.next_handle += 1;
        self.bytes_used += data.byte_size();
        self.handles.insert(h, Arc::new(data));
        h
    }

    pub fn alloc_icon_set(&mut self, set: crate::image::IconSet) -> u32 {
        let h = self.next_handle;
        self.next_handle += 1;
        self.icon_sets.insert(h, set);
        h
    }
}

type AppState = Arc<Mutex<AppImageState>>;

/// Register all `vyoma:images` host-calls on `linker`.
pub fn register_image_hostcalls(
    linker: &mut Linker<AppState>,
) -> anyhow::Result<()> {
    // ── io::load-image ──────────────────────────────────────────────────────
    linker.func_wrap(
        "vyoma:images/io",
        "load-image",
        |mut caller: wasmtime::Caller<'_, AppState>, ptr: u32, len: u32| -> (u32, u32) {
            let state = caller.data().clone();
            let path  = read_wasm_string(&mut caller, ptr, len)?;
            let mut s = state.lock().unwrap();

            let canonical = s.capability.validate_path(&path, &s.app_root)
                .map_err(|e| e.to_string())?;
            let key_path  = canonical.to_string_lossy().to_string();
            let path_hash = hash_path(&key_path);

            let img = s.worker.decode_path(&key_path, None)
                .map_err(|e| e.to_string())?;
            s.config.check_dimensions(img.width, img.height)
                .map_err(|e| e.to_string())?;
            let h = s.alloc_handle(img);
            Ok((h, 0)) // (handle, err_ptr=0)
        }
    )?;

    // ── io::decode-image ────────────────────────────────────────────────────
    linker.func_wrap(
        "vyoma:images/io",
        "decode-image",
        |mut caller: wasmtime::Caller<'_, AppState>, ptr: u32, len: u32, fmt_hint: u32| -> (u32, u32) {
            let state = caller.data().clone();
            let data  = read_wasm_bytes(&mut caller, ptr, len)?;
            let fmt   = match fmt_hint {
                1 => Some(ImageFormat::Png),
                2 => Some(ImageFormat::Jpeg),
                3 => Some(ImageFormat::Bmp),
                4 => Some(ImageFormat::Qoi),
                5 => Some(ImageFormat::Svg),
                _ => None,
            };
            let mut s = state.lock().unwrap();
            let budget = s.capability.effective_budget_bytes(s.config.max_decode_bytes);
            s.config.check_budget(data.len() * 6, s.bytes_used)
                .map_err(|e| e.to_string())?;
            let img = s.worker.decode_bytes(data, fmt).map_err(|e| e.to_string())?;
            s.config.check_dimensions(img.width, img.height).map_err(|e| e.to_string())?;
            let h = s.alloc_handle(img);
            Ok((h, 0))
        }
    )?;

    // ── io::unload-image ────────────────────────────────────────────────────
    linker.func_wrap("vyoma:images/io", "unload-image",
        |caller: wasmtime::Caller<'_, AppState>, handle: u32| {
            let mut s = caller.data().lock().unwrap();
            if let Some(img) = s.handles.remove(&handle) {
                s.bytes_used = s.bytes_used.saturating_sub(img.byte_size());
            }
        }
    )?;

    // ── io::image-size ──────────────────────────────────────────────────────
    linker.func_wrap("vyoma:images/io", "image-size",
        |caller: wasmtime::Caller<'_, AppState>, handle: u32| -> (u32, u32, u32) {
            let s = caller.data().lock().unwrap();
            if let Some(img) = s.handles.get(&handle) {
                (img.width, img.height, 0)
            } else {
                (0, 0, 1) // err flag
            }
        }
    )?;

    // ── io::set-resize-filter ───────────────────────────────────────────────
    linker.func_wrap("vyoma:images/io", "set-resize-filter",
        |caller: wasmtime::Caller<'_, AppState>, filter: u32| -> u32 {
            let mut s = caller.data().lock().unwrap();
            s.resize_filter = match filter {
                0 => ResizeFilter::Nearest,
                1 => ResizeFilter::Bilinear,
                2 => ResizeFilter::Lanczos3,
                _ => return 1, // error: unknown filter
            };
            0
        }
    )?;

    // ── icons::load-icon-set ────────────────────────────────────────────────
    linker.func_wrap("vyoma:images/icons", "load-icon-set",
        |mut caller: wasmtime::Caller<'_, AppState>, ptr: u32, len: u32| -> (u32, u32) {
            let state      = caller.data().clone();
            let bundle_str = read_wasm_string(&mut caller, ptr, len)?;
            let mut s      = state.lock().unwrap();
            if !s.config.icon_system_enabled {
                return Err("icon system not enabled on this platform".to_string());
            }
            let canonical  = s.capability.validate_path(&bundle_str, &s.app_root)
                .map_err(|e| e.to_string())?;
            let icon_set = load_icon_set(
                &s.app_root.file_name().unwrap_or_default().to_string_lossy(),
                &canonical,
                &s.worker,
            ).map_err(|e| e.to_string())?;
            let h = s.alloc_icon_set(icon_set);
            Ok((h, 0))
        }
    )?;

    // ── icons::icon-for-size ────────────────────────────────────────────────
    linker.func_wrap("vyoma:images/icons", "icon-for-size",
        |caller: wasmtime::Caller<'_, AppState>, set: u32, logical_px: u32, scale: f32| -> (u32, u32) {
            let mut s = caller.data().lock().unwrap();
            let icon_set = s.icon_sets.get(&set)
                .ok_or_else(|| ImageError::InvalidHandle(set).to_string())?;
            let img = icon_set.icon_for_size(logical_px, scale)
                .ok_or_else(|| "no icon variant found".to_string())?;
            let data = (*img).clone();
            s.bytes_used += data.byte_size();
            let h = s.next_handle;
            s.next_handle += 1;
            s.handles.insert(h, Arc::clone(&img));
            Ok((h, 0))
        }
    )?;

    // ── icons::unload-icon-set ──────────────────────────────────────────────
    linker.func_wrap("vyoma:images/icons", "unload-icon-set",
        |caller: wasmtime::Caller<'_, AppState>, set: u32| {
            caller.data().lock().unwrap().icon_sets.remove(&set);
        }
    )?;

    Ok(())
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn read_wasm_string(
    caller: &mut wasmtime::Caller<'_, AppState>,
    ptr: u32, len: u32,
) -> Result<String, String> {
    let mem = caller.get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or("no memory export")?;
    let bytes = mem.data(caller)
        .get(ptr as usize .. ptr as usize + len as usize)
        .ok_or("string out of bounds")?;
    String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())
}

fn read_wasm_bytes(
    caller: &mut wasmtime::Caller<'_, AppState>,
    ptr: u32, len: u32,
) -> Result<Vec<u8>, String> {
    let mem = caller.get_export("memory")
        .and_then(|e| e.into_memory())
        .ok_or("no memory export")?;
    Ok(mem.data(caller)
        .get(ptr as usize .. ptr as usize + len as usize)
        .ok_or("bytes out of bounds")?
        .to_vec())
}
```

---

## 16. VYOMA_DRAW_V2 image commands (stdout protocol extension)

The existing VYOMA_DRAW protocol (R11) is extended by a `V2` prefix for image commands. The supervisor's stdout parser detects `VYOMA_DRAW_V2:` and routes to the image pipeline. Commands remain line-oriented and stateless from the app's perspective.

```
VYOMA_DRAW_V2:load_image:<path>
    → Supervisor decodes the image (via worker), assigns a handle, writes
      "VYOMA_DRAW_REPLY:load_image:<handle>\n" to the app's stdin.
      Path must be within manifest-permitted directories.

VYOMA_DRAW_V2:blit_image:<handle>,<x>,<y>,<w>,<h>
    → Scale image to (w × h) using the active resize filter and blit onto
      the app's window Surface at (x, y).
      If w=0 and h=0, blit at 1:1 pixel scale.

VYOMA_DRAW_V2:blit_image_raw:<handle>,<x>,<y>
    → Blit at 1:1 without scaling (same as blit_image with w=0, h=0).

VYOMA_DRAW_V2:set_icon:<path>
    → Load icons from the directory at <path> and register as this app's
      system icon.  Supervisor updates the icon cache entry for this PID.

VYOMA_DRAW_V2:unload_image:<handle>
    → Free the image handle and decrement per-PID byte accounting.

VYOMA_DRAW_V2:image_size:<handle>
    → Reply: "VYOMA_DRAW_REPLY:image_size:<handle>,<w>,<h>\n"
```

Dispatch in supervisor (`supervisor/src/draw_cmd.rs` extension):

```rust
// Fragment added to the existing draw command parser in draw_cmd.rs:

pub enum DrawCmdV2 {
    LoadImage     { path: String },
    BlitImage     { handle: u32, x: i32, y: i32, w: u32, h: u32 },
    BlitImageRaw  { handle: u32, x: i32, y: i32 },
    SetIcon       { path: String },
    UnloadImage   { handle: u32 },
    ImageSize     { handle: u32 },
}

/// Parse a `VYOMA_DRAW_V2:` line (the prefix already stripped).
pub fn parse_draw_v2(line: &str) -> Option<DrawCmdV2> {
    let (cmd, args) = line.split_once(':')?;
    match cmd {
        "load_image"    => Some(DrawCmdV2::LoadImage { path: args.to_string() }),
        "unload_image"  => args.parse().ok().map(|h| DrawCmdV2::UnloadImage { handle: h }),
        "image_size"    => args.parse().ok().map(|h| DrawCmdV2::ImageSize   { handle: h }),
        "set_icon"      => Some(DrawCmdV2::SetIcon { path: args.to_string() }),
        "blit_image_raw" => {
            let p: Vec<&str> = args.splitn(3, ',').collect();
            if p.len() < 3 { return None; }
            Some(DrawCmdV2::BlitImageRaw {
                handle: p[0].parse().ok()?,
                x:      p[1].parse().ok()?,
                y:      p[2].parse().ok()?,
            })
        }
        "blit_image" => {
            let p: Vec<&str> = args.splitn(5, ',').collect();
            if p.len() < 5 { return None; }
            Some(DrawCmdV2::BlitImage {
                handle: p[0].parse().ok()?,
                x:      p[1].parse().ok()?,
                y:      p[2].parse().ok()?,
                w:      p[3].parse().ok()?,
                h:      p[4].parse().ok()?,
            })
        }
        _ => None,
    }
}
```

---

## 17. Integration with R11, R12, R13

### R11 (Surface, blit_surface)

`Surface::blit_rgba(src: &ImageData, dst_x, dst_y)` defined in `surface_ext.rs` performs straight-over alpha compositing pixel-by-pixel. `Surface::from_image_data` initialises a surface from a decoded image (used by wallpaper renderer). `Surface::snapshot` captures the current surface as an `ImageData` for QOI thumbnail storage.

### R12 (GPU, wgpu)

`upload-texture` WIT call invokes `vyoma:gpu/graphics.create_texture(data_ptr, byte_len, width, height, format_tag)`. The supervisor copies `ImageData.pixels` into a wgpu staging buffer then calls `write_texture` on the GPU device, reusing the same staging buffer allocation pattern as `R13::GpuGlyphAtlas`. The returned `TextureHandle` (u32) is stored in `GpuImageCache` for reuse across frames.

On eviction, `GpuImageCache::evict_lru` returns a list of `TextureHandle` values; the compositor calls `vyoma:gpu/graphics.destroy_texture(handle)` for each before dropping the cache entry.

### R13 (Font, GlyphAtlas)

The font subsystem's `GpuGlyphAtlas` uploads alpha maps via the same staging buffer path. Both systems share the `check_budget` / `check_dimensions` guard pattern from `ImageConfig`. When an `ImageData` is used as a glyph atlas page (custom bitmap font support, future), it flows through `Surface::blit_rgba` during rasterization.

### Compositor thumbnail pipeline (R25 preview)

```
flush pass (compositor)
  → for each visible window:
      surface.snapshot()              → ImageData (BGRA32, uncompressed)
      encode_qoi(&snapshot)           → Vec<u8> (QOI compressed)
      store in MissionControlThumbCache keyed by (window_id, timestamp)
  → Mission Control overlay decodes QOI thumbs on demand:
      decode_qoi(&qoi_bytes)          → ImageData
      surface.blit_rgba(&img, x, y)  → composited into overview surface
```

---

## 18. App-side usage example (WASM, Rust)

```rust
// apps/photo-viewer/src/main.rs  (wasm32-wasip2)
// Demonstrates loading, blitting, and icon registration.

fn main() {
    // Register app icon — supervisor picks up set_icon and updates icon cache.
    println!("VYOMA_DRAW_V2:set_icon:icons/");
    println!("VYOMA_DRAW:flush");

    // Load a JPEG photo from persistent storage.
    println!("VYOMA_DRAW_V2:load_image:/data/photos/landscape.jpg");
    let handle = read_reply_handle();   // reads "VYOMA_DRAW_REPLY:load_image:0"

    // Clear background.
    println!("VYOMA_DRAW:fill_rect:0,0,960,700,{}", 0x1E1E2EFF_u32);

    // Scale-blit to fit window (960 × 680 destination).
    println!("VYOMA_DRAW_V2:blit_image:{handle},0,20,960,680");
    println!("VYOMA_DRAW:flush");

    // Free handle when done.
    println!("VYOMA_DRAW_V2:unload_image:{handle}");

    // WIT path (for apps using the hostcall ABI directly):
    // let handle = vyoma_images::io::load_image("/data/photos/landscape.jpg").unwrap();
    // let (w, h) = vyoma_images::io::image_size(handle).unwrap();
    // vyoma_images::io::blit_image(handle, 0, 20, 960, 680).unwrap();
    // let tex = vyoma_images::io::upload_texture(handle).unwrap(); // R12 GPU path
}

fn read_reply_handle() -> u32 {
    use std::io::BufRead;
    let stdin = std::io::stdin();
    for line in stdin.lock().lines().flatten() {
        if let Some(rest) = line.strip_prefix("VYOMA_DRAW_REPLY:load_image:") {
            return rest.trim().parse().unwrap_or(0);
        }
    }
    0
}
```

App manifest:

```toml
# apps/photo-viewer/vyoma.toml
[app]
name    = "photo-viewer"
version = "1.0.0"
wasm    = "photo-viewer.wasm"

[capabilities]
stdio   = true
display = true

[capabilities.images]
decode       = ["jpeg", "png", "qoi"]
max_decode_mb = 64
icons         = ["icons/"]
```

---

## 19. Platform capability matrix

| Feature | mcu-minimal | iot-edge | robotics-rt | mobile | desktop-full | server-headless |
|---|---|---|---|---|---|---|
| PNG | no | yes | yes | yes | yes | yes |
| JPEG | no | yes | yes | yes | yes | yes |
| QOI | no | yes | yes | yes | yes | yes |
| BMP | no | yes | yes | yes | yes | yes |
| SVG (resvg) | no | no | no | yes | yes | no |
| GPU upload | no | no | no | yes | yes | no |
| Max image dimension | n/a | 1024 px | 1024 px | 2048 px | 8192 px | 8192 px |
| Max decode per PID | 0 | 4 MiB | 8 MiB | 32 MiB | 256 MiB | 128 MiB |
| Icon system | no | basic | basic | yes | yes | no |
| CPU image cache HWM | 0 | 2 MiB | 4 MiB | 48 MiB | 512 MiB | 256 MiB |
| Native pixel format | Mono1bpp | RGB565 | RGB565 | BGRA32 | BGRA32 | BGRA32 |
| `catch_unwind` guards | n/a | yes | yes | yes | yes | yes |

---

## 20. Security model

**Adversarial input**: Every decoder (`decode_png`, `decode_jpeg`, `decode_bmp`, `decode_qoi`, `decode_svg`) wraps the inner call in `catch_unwind(AssertUnwindSafe(...))`. A panic in any third-party crate returns `ImageError::DecodePanic` to the caller; the supervisor never crashes.

**Budget enforcement**: `ImageConfig::check_budget(bytes_needed, bytes_used)` is called before every allocation using a worst-case expansion estimate (`raw_bytes × 6`). Exceeding the per-PID budget returns `ImageError::TooLarge` without allocating.

**Dimension gate**: `ImageConfig::check_dimensions(w, h)` rejects zero or oversized dimensions before passing to any decoder. This prevents multi-gigabyte allocs from malformed IHDR headers.

**Path traversal**: `ImagesCapability::validate_path` canonicalises both the app root and the requested path and asserts `starts_with(app_root) || starts_with("/data")`. Symlink traversal is blocked by `canonicalize`.

**Worker watchdog**: If the image worker thread panics (uncaught Rust panic not from a decoder, e.g., OOM in the worker itself), the monitor thread detects channel closure and respawns a fresh worker. In-flight requests receive `ImageError::WorkerPanic`. The public `ImageWorkerHandle::tx` sender is reused via the forwarding-proxy design in `worker.rs`.

**Format allowlist**: `ImageConfig::format_allowed` requires both the platform `formats_enabled` set AND the app manifest `capabilities.images.decode` list to contain the format. An app cannot decode SVG on a platform where it is disabled regardless of its manifest declaration.

---

## 21. Cargo dependencies

```toml
# supervisor/Cargo.toml additions for the image pipeline:
[dependencies]
png            = { version = "0.17",  optional = true }          # PNG decode
jpeg-decoder   = { version = "0.3",   optional = true }          # JPEG decode
qoi            = { version = "0.4",   optional = true }          # QOI decode/encode
resvg          = { version = "0.42",  optional = true }          # SVG rasterise
usvg           = { version = "0.42",  optional = true }          # SVG parse
tiny-skia      = { version = "0.11",  optional = true }          # pixel buffer for resvg

[features]
# Activated per platform in build.rs / profile TOML:
image-png  = ["png"]
image-jpeg = ["jpeg-decoder"]
image-qoi  = ["qoi"]
svg        = ["resvg", "usvg", "tiny-skia"]
# Convenience: all raster formats (iot-edge through server-headless)
image-raster = ["image-png", "image-jpeg", "image-qoi"]
# Full desktop + mobile image stack
image-full = ["image-raster", "svg"]
```

Feature selection in `build.rs`:

```rust
// supervisor/build.rs (relevant fragment)
fn main() {
    let platform = std::env::var("VYOMA_PLATFORM").unwrap_or_else(|_| "desktop-full".into());
    match platform.as_str() {
        "mcu-minimal"    => { /* no image features */ }
        "iot-edge" | "robotics-rt" | "server-headless"
                         => println!("cargo:rustc-cfg=feature=\"image-raster\""),
        "mobile"         => println!("cargo:rustc-cfg=feature=\"image-full\""),
        _                => println!("cargo:rustc-cfg=feature=\"image-full\""), // desktop-full
    }
}
```

---

## 22. Implementation file map

| File | Purpose | LOC budget |
|---|---|---|
| `supervisor/src/image/mod.rs` | `ImageData`, `ImageFormat`, `PixelFormat`, `IconSet`, `ImageError`, `ResizeFilter`; module re-exports | ≤ 500 |
| `supervisor/src/image/config.rs` | `ImageConfig` per-platform defaults, `check_budget`, `check_dimensions` | ≤ 300 |
| `supervisor/src/image/decode_png.rs` | `decode_png(data)` → `ImageData`; RGBA/RGB/Grayscale → BGRA32 | ≤ 200 |
| `supervisor/src/image/decode_jpeg.rs` | `decode_jpeg(data)` → `ImageData`; YCbCr → RGB → BGRA32 | ≤ 150 |
| `supervisor/src/image/decode_bmp.rs` | `decode_bmp(data)` — hand-rolled 24/32 bpp DIB | ≤ 250 |
| `supervisor/src/image/decode_qoi.rs` | `decode_qoi(data)` + `encode_qoi(img)` for thumbnail snapshots | ≤ 120 |
| `supervisor/src/image/decode_svg.rs` | `decode_svg(data, w, h, max)` via resvg; `#[cfg(feature="svg")]` | ≤ 150 |
| `supervisor/src/image/resize.rs` | `resize(src, w, h, filter)` — Nearest, Bilinear, Lanczos3; gamma-correct | ≤ 500 |
| `supervisor/src/image/cache.rs` | `ImageCache` (CPU LRU) + `GpuImageCache` (texture handle LRU) + `hash_path` | ≤ 350 |
| `supervisor/src/image/icon.rs` | `SystemIconCache` + `load_icon_set` + filename parser + size synthesis | ≤ 400 |
| `supervisor/src/image/worker.rs` | `ImageWorker`, `ImageReq`, `ImageWorkerHandle`, `spawn_image_worker` watchdog | ≤ 450 |
| `supervisor/src/image/surface_ext.rs` | `Surface::blit_rgba`, `Surface::from_image_data`, `Surface::snapshot` | ≤ 200 |
| `supervisor/src/image/wit_handlers.rs` | Wasmtime linker for `vyoma:images/io` + `icons`; handle table management | ≤ 450 |
| `supervisor/src/image/manifest.rs` | `ImagesCapability` deserialise, `validate_path`, `canonical_icon_dirs` | ≤ 200 |
| `wit/vyoma-images.wit` | WIT interface definition | — |

Total: 14 Rust source files, all within the 500-line budget.
