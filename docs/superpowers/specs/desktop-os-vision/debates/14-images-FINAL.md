# VyomaOS Subsystem 14: Image & Icon Pipeline — FINAL

**Status:** FINAL — all 10 blocking issues resolved  
**Date:** 2026-05-29  
**Depends on:** R11 (Surface/blit_surface), R12 (GPU WIT/wgpu), R13 (FontProvider/GlyphAtlas)

---

## §1 Overview

The Image & Icon Pipeline (macOS equivalent: ImageIO + CoreImage lite) provides VyomaOS with a secure, multi-format image decode/encode/resize/blit subsystem integrated into the supervisor's display path and exposed to WASM apps via WIT hostcalls and the `VYOMA_DRAW_V2:` protocol.

### v1 Deliverables

- **Decode:** PNG, JPEG (with EXIF orientation), BMP (BI_RGB only), QOI, SVG (rasterized, no external resources)
- **Encode:** QOI (compositor thumbnails), raw BGRA dump
- **Resize:** Nearest, Bilinear (alpha-correct), Lanczos3 (sRGB gamma-correct two-pass)
- **Cache:** LRU image cache (full-path key, no hash collisions), GPU texture cache coordinated with R12
- **Icons:** `IconSet` directory scan, size synthesis, `SystemIconCache` with scale-factor awareness
- **Surface integration:** `blit_rgba` with straight-over alpha, `from_image_data`, `snapshot_qoi`
- **WIT interface:** `vyoma:images@1.0.0` worlds `io` and `icons` with source crop in `blit-image`
- **Security:** 6-layer hardening (SVG lockdown, dimension-before-alloc, per-PID budget, full-path keys, catch_unwind, worker watchdog)
- **Worker pool:** platform-sized decode worker pool with per-worker monitor/respawn

### Deferred to v2+

- Animated formats: GIF, APNG, WebP animation (frame sequencing, timing loop)
- Full ICC color profile pipeline (owned by R15: Color Management)
- Image effects / filter graph (CoreImage equivalent) — v3
- Thumbnail daemon (background pre-raster cache warmer)
- Image metadata query API (EXIF/XMP arbitrary tag read)
- HEIF/AVIF decode
- Progressive JPEG streaming decode

---

## §2 Core Types

```rust
// supervisor/src/image/types.rs  (~120 lines)

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// Decoded image data in a specific pixel format.
/// `stride` is bytes per row (may be padded, always >= width * bpp).
#[derive(Debug, Clone)]
pub struct ImageData {
    pub pixels: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Bytes per row. Always `width * 4` for Bgra32; may differ for Rgb565/Grayscale8.
    pub stride: u32,
    pub format: PixelFormat,
    pub original_format: ImageFormat,
}

impl ImageData {
    /// Returns a sub-region view as a new ImageData (copies pixels).
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Result<ImageData, ImageError> {
        if x + w > self.width || y + h > self.height {
            return Err(ImageError::InvalidDimensions);
        }
        let bpp = self.format.bytes_per_pixel() as u32;
        let mut out = Vec::with_capacity((w * h * bpp) as usize);
        for row in y..(y + h) {
            let start = (row * self.stride + x * bpp) as usize;
            let end = start + (w * bpp) as usize;
            out.extend_from_slice(&self.pixels[start..end]);
        }
        Ok(ImageData {
            pixels: out,
            width: w,
            height: h,
            stride: w * bpp,
            format: self.format,
            original_format: self.original_format,
        })
    }

    pub fn decoded_bytes(&self) -> usize {
        self.pixels.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    Png,
    Jpeg,
    Bmp,
    Qoi,
    Svg,
}

impl ImageFormat {
    pub fn extension(&self) -> &'static str {
        match self {
            Self::Png  => "png",
            Self::Jpeg => "jpg",
            Self::Bmp  => "bmp",
            Self::Qoi  => "qoi",
            Self::Svg  => "svg",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// 4 bytes/pixel, byte order B G R A. Default output format for all decoders.
    Bgra32,
    /// 2 bytes/pixel packed: RRRRRGGGGGGBBBBB (little-endian). Used on mcu-minimal.
    Rgb565,
    /// 1 byte/pixel luminance. Used for grayscale JPEG on iot-edge.
    Grayscale8,
    /// 1 bit/pixel packed MSB-first. Reserved for e-ink displays.
    Mono1bpp,
}

impl PixelFormat {
    pub fn bytes_per_pixel(&self) -> usize {
        match self {
            Self::Bgra32    => 4,
            Self::Rgb565    => 2,
            Self::Grayscale8 => 1,
            Self::Mono1bpp  => 1, // stride is ceil(width/8) bytes but bpp reported as 1 for simplicity
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeFilter {
    /// No interpolation. Fast, pixelated. Used for icons on mcu-minimal/iot-edge.
    Nearest,
    /// Bilinear interpolation, all 4 BGRA channels. ~15ms for 4 MP → 1 MP.
    Bilinear,
    /// Two-pass Lanczos3 in linear light (sRGB gamma expansion/compression).
    /// ~40ms for 4 MP → 1 MP. Default for display-quality downscale.
    Lanczos3,
}

/// Cache key using the full canonical path — no hash collision risk (B2).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct ImageCacheKey {
    /// Full canonical PathBuf — equality is byte-for-byte path comparison.
    pub path: Arc<PathBuf>,
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
}

impl ImageCacheKey {
    pub fn new(path: Arc<PathBuf>, width: u32, height: u32, format: PixelFormat) -> Self {
        Self { path, width, height, format }
    }
}

/// A set of pre-rasterized icon sizes for one logical icon.
#[derive(Debug, Clone)]
pub struct IconSet {
    pub name: String,
    /// Keyed by pixel size (16, 32, 48, 64, 128, 256, 512). Always Bgra32.
    pub sizes: BTreeMap<u32, ImageData>,
}

impl IconSet {
    pub fn canonical_sizes() -> &'static [u32] {
        &[16, 32, 48, 64, 128, 256, 512]
    }
}

/// All error variants for the image subsystem.
#[derive(Debug, Clone)]
pub enum ImageError {
    /// Compressed/raw data exceeds `max_raw_bytes`.
    TooLarge(usize),
    /// Decode library returned an error.
    DecodeFailed(String),
    /// Decoder panicked; caught by `catch_unwind`.
    DecodePanic,
    /// Worker did not respond within timeout.
    WorkerTimeout,
    /// Worker thread panicked; will be respawned by monitor.
    WorkerPanic,
    /// Format variant is recognised but not supported in v1 (e.g., BMP BI_BITFIELDS).
    UnsupportedFormat(&'static str),
    /// Width or height exceeds `max_image_dimension`, or crop is out of bounds.
    InvalidDimensions,
    /// Feature flag for this format is not compiled in (`#[cfg(feature = "...")]`).
    FormatNotEnabled,
    /// App's per-PID decoded-bytes budget would be exceeded.
    BudgetExceeded { pid: u32, requested: usize, limit: usize },
    /// Path escapes the declared capability directory (canonicalize + starts_with check).
    PathTraversal,
    /// Encode failed.
    EncodeFailed(String),
    /// Requested icon size not available and synthesis failed.
    IconNotFound { name: String, size: u32 },
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(n)        => write!(f, "image too large: {n} bytes"),
            Self::DecodeFailed(s)    => write!(f, "decode failed: {s}"),
            Self::DecodePanic        => write!(f, "decoder panicked"),
            Self::WorkerTimeout      => write!(f, "image worker timeout"),
            Self::WorkerPanic        => write!(f, "image worker panicked"),
            Self::UnsupportedFormat(s) => write!(f, "unsupported format: {s}"),
            Self::InvalidDimensions  => write!(f, "invalid dimensions"),
            Self::FormatNotEnabled   => write!(f, "format not compiled in"),
            Self::BudgetExceeded{pid,requested,limit} =>
                write!(f, "budget exceeded for pid {pid}: requested {requested}, limit {limit}"),
            Self::PathTraversal      => write!(f, "path traversal attempt"),
            Self::EncodeFailed(s)    => write!(f, "encode failed: {s}"),
            Self::IconNotFound{name,size} => write!(f, "icon '{name}' not found at size {size}"),
        }
    }
}

impl std::error::Error for ImageError {}
```

---

## §3 ImageConfig

```rust
// supervisor/src/image/config.rs  (~110 lines)

/// Per-platform image subsystem configuration.
/// All memory limits are for decoded BGRA32 pixels, not compressed bytes.
#[derive(Debug, Clone)]
pub struct ImageConfig {
    /// Maximum compressed input size in bytes. Reject before any decode.
    pub max_raw_bytes: usize,
    /// Maximum decoded image dimension (width or height).
    pub max_image_dimension: u32,
    /// Maximum decoded bytes per PID across all live images (B6).
    pub max_decoded_bytes_per_pid: usize,
    /// Number of decode worker threads (B5). 0 = synchronous fallback on caller thread.
    pub decode_worker_threads: usize,
    /// Maximum capacity of the LRU image cache (number of entries).
    pub image_cache_capacity: usize,
    /// Maximum capacity of the LRU GPU texture cache (number of entries).
    pub gpu_cache_capacity: usize,
    /// Enable SVG decode (requires `decode_svg` feature).
    pub svg_enabled: bool,
    /// Enable GPU texture upload (requires `image_gpu` feature + R12).
    pub gpu_enabled: bool,
    /// Enable EXIF orientation correction (requires `kamadak-exif` feature).
    pub exif_enabled: bool,
    /// Default resize filter for icon synthesis.
    pub icon_resize_filter: ResizeFilter,
    /// Canonical icon sizes to pre-rasterize from SVG source icons.
    pub icon_sizes: &'static [u32],
    /// Worker recv_timeout in milliseconds.
    pub worker_timeout_ms: u64,
}

impl ImageConfig {
    pub fn for_platform(platform: &str) -> Self {
        match platform {
            "mcu-minimal" => Self {
                max_raw_bytes:               64 * 1024,          // 64 KB
                max_image_dimension:         256,
                max_decoded_bytes_per_pid:   128 * 1024,         // 128 KB
                decode_worker_threads:       0,                  // synchronous only
                image_cache_capacity:        8,
                gpu_cache_capacity:          0,
                svg_enabled:                 false,
                gpu_enabled:                 false,
                exif_enabled:                false,
                icon_resize_filter:          ResizeFilter::Nearest,
                icon_sizes:                  &[16, 32],
                worker_timeout_ms:           500,
            },
            "iot-edge" => Self {
                max_raw_bytes:               4 * 1024 * 1024,    // 4 MB
                max_image_dimension:         2048,
                max_decoded_bytes_per_pid:   8 * 1024 * 1024,    // 8 MB
                decode_worker_threads:       1,
                image_cache_capacity:        32,
                gpu_cache_capacity:          0,
                svg_enabled:                 false,
                gpu_enabled:                 false,
                exif_enabled:                true,
                icon_resize_filter:          ResizeFilter::Nearest,
                icon_sizes:                  &[16, 32, 48, 64],
                worker_timeout_ms:           1000,
            },
            "robotics-rt" => Self {
                max_raw_bytes:               8 * 1024 * 1024,    // 8 MB
                max_image_dimension:         4096,
                max_decoded_bytes_per_pid:   16 * 1024 * 1024,   // 16 MB
                decode_worker_threads:       1,
                image_cache_capacity:        48,
                gpu_cache_capacity:          0,
                svg_enabled:                 false,
                gpu_enabled:                 false,
                exif_enabled:                true,
                icon_resize_filter:          ResizeFilter::Bilinear,
                icon_sizes:                  &[16, 32, 48, 64, 128],
                worker_timeout_ms:           500,
            },
            "mobile" => Self {
                max_raw_bytes:               32 * 1024 * 1024,   // 32 MB
                max_image_dimension:         8192,
                max_decoded_bytes_per_pid:   64 * 1024 * 1024,   // 64 MB
                decode_worker_threads:       2,
                image_cache_capacity:        128,
                gpu_cache_capacity:          64,
                svg_enabled:                 true,
                gpu_enabled:                 true,
                exif_enabled:                true,
                icon_resize_filter:          ResizeFilter::Lanczos3,
                icon_sizes:                  &[16, 32, 48, 64, 128, 256],
                worker_timeout_ms:           500,
            },
            "desktop-full" => Self {
                max_raw_bytes:               128 * 1024 * 1024,  // 128 MB
                max_image_dimension:         16384,
                max_decoded_bytes_per_pid:   256 * 1024 * 1024,  // 256 MB
                decode_worker_threads:       3,
                image_cache_capacity:        512,
                gpu_cache_capacity:          256,
                svg_enabled:                 true,
                gpu_enabled:                 true,
                exif_enabled:                true,
                icon_resize_filter:          ResizeFilter::Lanczos3,
                icon_sizes:                  &[16, 32, 48, 64, 128, 256, 512],
                worker_timeout_ms:           500,
            },
            "server-headless" => Self {
                max_raw_bytes:               256 * 1024 * 1024,  // 256 MB
                max_image_dimension:         32768,
                max_decoded_bytes_per_pid:   512 * 1024 * 1024,  // 512 MB
                decode_worker_threads:       3,
                image_cache_capacity:        256,
                gpu_cache_capacity:          0,
                svg_enabled:                 true,
                gpu_enabled:                 false,              // headless: no display GPU
                exif_enabled:                true,
                icon_resize_filter:          ResizeFilter::Nearest,
                icon_sizes:                  &[16, 32, 48, 64, 128, 256, 512],
                worker_timeout_ms:           2000,
            },
            _ => Self::for_platform("desktop-full"), // safe default
        }
    }

    /// Check that (w, h) fit within configured limits.
    pub fn check_dimensions(&self, w: u32, h: u32) -> Result<(), ImageError> {
        if w == 0 || h == 0 || w > self.max_image_dimension || h > self.max_image_dimension {
            Err(ImageError::InvalidDimensions)
        } else {
            Ok(())
        }
    }
}
```

---

## §4 Decoders

```rust
// supervisor/src/image/detect.rs  (~60 lines)

/// Detect image format from magic bytes. Returns None for unknown/ambiguous data.
/// This runs BEFORE any allocation; data slice may be as short as 12 bytes.
pub fn detect_format(data: &[u8]) -> Option<ImageFormat> {
    if data.len() < 4 { return None; }
    // PNG: 8-byte signature \x89PNG\r\n\x1a\n
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some(ImageFormat::Png);
    }
    // JPEG: FF D8 FF
    if data.starts_with(b"\xFF\xD8\xFF") {
        return Some(ImageFormat::Jpeg);
    }
    // BMP: 'BM'
    if data.starts_with(b"BM") {
        return Some(ImageFormat::Bmp);
    }
    // QOI: 'qoif'
    if data.starts_with(b"qoif") {
        return Some(ImageFormat::Qoi);
    }
    // SVG: XML scan for '<svg' within first 512 bytes (handles <?xml ...> preamble)
    let scan_len = data.len().min(512);
    let head = &data[..scan_len];
    if head.windows(4).any(|w| w == b"<svg") {
        return Some(ImageFormat::Svg);
    }
    None
}
```

```rust
// supervisor/src/image/decode_png.rs  (~90 lines)

use crate::image::types::{ImageData, ImageError, ImageFormat, PixelFormat};

/// Parse PNG header only — no pixel allocation (B4).
pub fn png_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if data.len() < 24 {
            return Err(ImageError::DecodeFailed("PNG data too short".into()));
        }
        // PNG IHDR: bytes 16-23 are width(4) + height(4) big-endian
        if !data.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(ImageError::DecodeFailed("not a PNG".into()));
        }
        let w = u32::from_be_bytes(data[16..20].try_into().unwrap());
        let h = u32::from_be_bytes(data[20..24].try_into().unwrap());
        Ok((w, h))
    })).map_err(|_| ImageError::DecodePanic)?
}

/// Decode PNG to Bgra32. Requires `decode_png` feature.
#[cfg(feature = "decode_png")]
pub fn decode_png(data: &[u8]) -> Result<ImageData, ImageError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let decoder = png::Decoder::new(std::io::Cursor::new(data));
        let mut reader = decoder.read_info()
            .map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
        let mut buf = vec![0u8; reader.output_buffer_size()];
        let frame = reader.next_frame(&mut buf)
            .map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
        let info = reader.info();
        let w = info.width;
        let h = info.height;
        // Convert to Bgra32 from whatever output color type we got
        let bgra = png_to_bgra32(&buf[..frame.buffer_size()], w, h, frame.color_type, frame.bit_depth)?;
        Ok(ImageData {
            stride: w * 4,
            pixels: bgra,
            width: w,
            height: h,
            format: PixelFormat::Bgra32,
            original_format: ImageFormat::Png,
        })
    })).map_err(|_| ImageError::DecodePanic)?
}

#[cfg(feature = "decode_png")]
fn png_to_bgra32(buf: &[u8], w: u32, h: u32,
                 ct: png::ColorType, bd: png::BitDepth) -> Result<Vec<u8>, ImageError> {
    use png::{ColorType::*, BitDepth::*};
    let n = (w * h) as usize;
    let mut out = vec![0u8; n * 4];
    match (ct, bd) {
        (Rgba, Eight) => {
            for (i, px) in buf.chunks_exact(4).enumerate() {
                out[i*4]   = px[2]; // B
                out[i*4+1] = px[1]; // G
                out[i*4+2] = px[0]; // R
                out[i*4+3] = px[3]; // A
            }
        },
        (Rgb, Eight) => {
            for (i, px) in buf.chunks_exact(3).enumerate() {
                out[i*4]   = px[2]; // B
                out[i*4+1] = px[1]; // G
                out[i*4+2] = px[0]; // R
                out[i*4+3] = 0xFF;  // A = opaque
            }
        },
        (Grayscale, Eight) => {
            for (i, &g) in buf.iter().enumerate() {
                out[i*4]   = g;
                out[i*4+1] = g;
                out[i*4+2] = g;
                out[i*4+3] = 0xFF;
            }
        },
        (GrayscaleAlpha, Eight) => {
            for (i, px) in buf.chunks_exact(2).enumerate() {
                out[i*4]   = px[0];
                out[i*4+1] = px[0];
                out[i*4+2] = px[0];
                out[i*4+3] = px[1];
            }
        },
        _ => return Err(ImageError::UnsupportedFormat("PNG: unsupported bit depth / color type")),
    }
    Ok(out)
}

#[cfg(not(feature = "decode_png"))]
pub fn decode_png(_data: &[u8]) -> Result<ImageData, ImageError> {
    Err(ImageError::FormatNotEnabled)
}
```

```rust
// supervisor/src/image/decode_jpeg.rs  (~130 lines)

use crate::image::types::{ImageData, ImageError, ImageFormat, PixelFormat};

/// Parse JPEG header only (B4).
pub fn jpeg_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        #[cfg(feature = "decode_jpeg")] {
            let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(data));
            decoder.decode_metadata()
                .map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
            let info = decoder.info()
                .ok_or_else(|| ImageError::DecodeFailed("no JPEG info".into()))?;
            Ok((info.width as u32, info.height as u32))
        }
        #[cfg(not(feature = "decode_jpeg"))]
        Err(ImageError::FormatNotEnabled)
    })).map_err(|_| ImageError::DecodePanic)?
}

/// Read EXIF Orientation tag (B10). Returns None if no EXIF or no orientation tag.
#[cfg(all(feature = "decode_jpeg", feature = "exif"))]
fn read_jpeg_exif_orientation(data: &[u8]) -> Option<u32> {
    use kamadak_exif as exif;
    let mut cursor = std::io::Cursor::new(data);
    let exif_reader = exif::Reader::new();
    let exif_data = exif_reader.read_from_container(&mut cursor).ok()?;
    let field = exif_data.get_field(exif::Tag::Orientation, exif::In::PRIMARY)?;
    match &field.value {
        exif::Value::Short(v) => v.first().map(|&x| x as u32),
        _ => None,
    }
}

#[cfg(not(all(feature = "decode_jpeg", feature = "exif")))]
fn read_jpeg_exif_orientation(_data: &[u8]) -> Option<u32> { None }

/// Apply EXIF orientation transform to decoded ImageData (B10).
fn apply_exif_orientation(img: ImageData, orientation: u32) -> ImageData {
    match orientation {
        1 => img,
        2 => flip_horizontal(img),
        3 => rotate_180(img),
        4 => flip_vertical(img),
        5 => rotate_90_cw(flip_horizontal(img)),
        6 => rotate_90_cw(img),
        7 => rotate_90_ccw(flip_horizontal(img)),
        8 => rotate_90_ccw(img),
        _ => img,
    }
}

fn rotate_90_cw(img: ImageData) -> ImageData {
    let (sw, sh) = (img.width as usize, img.height as usize);
    let mut out = vec![0u8; sw * sh * 4];
    for y in 0..sh {
        for x in 0..sw {
            let src = (y * sw + x) * 4;
            let dst = (x * sh + (sh - 1 - y)) * 4;
            out[dst..dst+4].copy_from_slice(&img.pixels[src..src+4]);
        }
    }
    ImageData { pixels: out, width: sh as u32, height: sw as u32,
                stride: sh as u32 * 4, format: img.format, original_format: img.original_format }
}

fn rotate_90_ccw(img: ImageData) -> ImageData {
    let (sw, sh) = (img.width as usize, img.height as usize);
    let mut out = vec![0u8; sw * sh * 4];
    for y in 0..sh {
        for x in 0..sw {
            let src = (y * sw + x) * 4;
            let dst = ((sw - 1 - x) * sh + y) * 4;
            out[dst..dst+4].copy_from_slice(&img.pixels[src..src+4]);
        }
    }
    ImageData { pixels: out, width: sh as u32, height: sw as u32,
                stride: sh as u32 * 4, format: img.format, original_format: img.original_format }
}

fn rotate_180(img: ImageData) -> ImageData {
    let mut out = img.pixels.clone();
    let n = (img.width * img.height) as usize;
    for i in 0..n/2 {
        let j = n - 1 - i;
        let (a, b) = (i * 4, j * 4);
        for k in 0..4 { out.swap(a + k, b + k); }
    }
    ImageData { pixels: out, ..img }
}

fn flip_horizontal(img: ImageData) -> ImageData {
    let mut out = img.pixels.clone();
    let w = img.width as usize;
    let h = img.height as usize;
    for y in 0..h {
        for x in 0..w/2 {
            let a = (y * w + x) * 4;
            let b = (y * w + (w - 1 - x)) * 4;
            for k in 0..4 { out.swap(a + k, b + k); }
        }
    }
    ImageData { pixels: out, ..img }
}

fn flip_vertical(img: ImageData) -> ImageData {
    let mut out = img.pixels.clone();
    let w = img.width as usize;
    let h = img.height as usize;
    for y in 0..h/2 {
        let top = y * w * 4;
        let bot = (h - 1 - y) * w * 4;
        for x in 0..w*4 {
            out.swap(top + x, bot + x);
        }
    }
    ImageData { pixels: out, ..img }
}

/// Decode JPEG to Bgra32 with EXIF orientation applied (B10).
#[cfg(feature = "decode_jpeg")]
pub fn decode_jpeg(data: &[u8]) -> Result<ImageData, ImageError> {
    let orientation = read_jpeg_exif_orientation(data);
    let img = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_jpeg_pixels(data)
    })).map_err(|_| ImageError::DecodePanic)??;
    Ok(if let Some(o) = orientation { apply_exif_orientation(img, o) } else { img })
}

#[cfg(feature = "decode_jpeg")]
fn decode_jpeg_pixels(data: &[u8]) -> Result<ImageData, ImageError> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(data));
    let pixels_raw = decoder.decode()
        .map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
    let info = decoder.info().ok_or_else(|| ImageError::DecodeFailed("no JPEG info".into()))?;
    let w = info.width as u32;
    let h = info.height as u32;
    let bgra = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => {
            let mut out = vec![0u8; (w * h * 4) as usize];
            for (i, px) in pixels_raw.chunks_exact(3).enumerate() {
                out[i*4] = px[2]; out[i*4+1] = px[1]; out[i*4+2] = px[0]; out[i*4+3] = 0xFF;
            }
            out
        },
        jpeg_decoder::PixelFormat::L8 => {
            let mut out = vec![0u8; (w * h * 4) as usize];
            for (i, &g) in pixels_raw.iter().enumerate() {
                out[i*4] = g; out[i*4+1] = g; out[i*4+2] = g; out[i*4+3] = 0xFF;
            }
            out
        },
        _ => return Err(ImageError::UnsupportedFormat("JPEG: unsupported pixel format")),
    };
    Ok(ImageData { pixels: bgra, width: w, height: h, stride: w * 4,
                   format: PixelFormat::Bgra32, original_format: ImageFormat::Jpeg })
}

#[cfg(not(feature = "decode_jpeg"))]
pub fn decode_jpeg(_data: &[u8]) -> Result<ImageData, ImageError> { Err(ImageError::FormatNotEnabled) }
```

```rust
// supervisor/src/image/decode_bmp.rs  (~100 lines)

use crate::image::types::{ImageData, ImageError, ImageFormat, PixelFormat};

/// Parse BMP header dimensions only (B4).
pub fn bmp_header_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    if data.len() < 26 { return Err(ImageError::DecodeFailed("BMP too short".into())); }
    if &data[0..2] != b"BM" { return Err(ImageError::DecodeFailed("not a BMP".into())); }
    let w = u32::from_le_bytes(data[18..22].try_into().unwrap());
    let h_raw = i32::from_le_bytes(data[22..26].try_into().unwrap());
    let h = h_raw.unsigned_abs();
    Ok((w, h))
}

/// Decode BMP to Bgra32. Only BI_RGB (uncompressed) is supported in v1 (B3).
pub fn decode_bmp(data: &[u8]) -> Result<ImageData, ImageError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_bmp_inner(data)
    })).map_err(|_| ImageError::DecodePanic)?
}

fn decode_bmp_inner(data: &[u8]) -> Result<ImageData, ImageError> {
    if data.len() < 54 { return Err(ImageError::DecodeFailed("BMP header too short".into())); }
    if &data[0..2] != b"BM" { return Err(ImageError::DecodeFailed("not a BMP".into())); }

    let pixel_offset = u32::from_le_bytes(data[10..14].try_into().unwrap()) as usize;
    let dib_size    = u32::from_le_bytes(data[14..18].try_into().unwrap());
    let width       = u32::from_le_bytes(data[18..22].try_into().unwrap());
    let h_raw       = i32::from_le_bytes(data[22..26].try_into().unwrap());
    let height      = h_raw.unsigned_abs();
    let top_down    = h_raw < 0;
    let bpp         = u16::from_le_bytes(data[28..30].try_into().unwrap());
    let compression = u32::from_le_bytes(data[30..34].try_into().unwrap());

    // B3: reject anything but BI_RGB
    match compression {
        0 => {},   // BI_RGB: supported
        1 => return Err(ImageError::UnsupportedFormat("BMP BI_RLE8 not supported in v1")),
        2 => return Err(ImageError::UnsupportedFormat("BMP BI_RLE4 not supported in v1")),
        3 => return Err(ImageError::UnsupportedFormat("BMP BI_BITFIELDS not supported in v1")),
        _ => return Err(ImageError::UnsupportedFormat("BMP unknown compression")),
    }
    if width == 0 || height == 0 { return Err(ImageError::InvalidDimensions); }
    if bpp != 24 && bpp != 32 { return Err(ImageError::UnsupportedFormat("BMP: only 24bpp/32bpp supported")); }

    let bpp_bytes = (bpp / 8) as usize;
    // Padded row stride: aligned to 4 bytes
    let row_stride = ((width as usize * bpp_bytes + 3) / 4) * 4;
    let expected_size = pixel_offset + row_stride.checked_mul(height as usize)
        .ok_or(ImageError::InvalidDimensions)?;
    if data.len() < expected_size {
        return Err(ImageError::DecodeFailed("BMP pixel data truncated".into()));
    }

    let mut bgra = vec![0u8; (width * height * 4) as usize];
    for y in 0..height as usize {
        let src_y = if top_down { y } else { height as usize - 1 - y };
        let row_start = pixel_offset + src_y * row_stride;
        for x in 0..width as usize {
            let src_i = row_start + x * bpp_bytes;
            let dst_i = (y * width as usize + x) * 4;
            // BMP stores BGR[A] — map to BGRA32
            bgra[dst_i]   = data[src_i];     // B
            bgra[dst_i+1] = data[src_i+1];   // G
            bgra[dst_i+2] = data[src_i+2];   // R
            bgra[dst_i+3] = if bpp == 32 { data[src_i+3] } else { 0xFF };
        }
    }

    Ok(ImageData {
        pixels: bgra, width, height: height as u32,
        stride: width * 4,
        format: PixelFormat::Bgra32,
        original_format: ImageFormat::Bmp,
    })
}
```

```rust
// supervisor/src/image/decode_qoi.rs  (~55 lines)

use crate::image::types::{ImageData, ImageError, ImageFormat, PixelFormat};

/// Parse QOI header dimensions only (B4).
pub fn qoi_dimensions(data: &[u8]) -> Result<(u32, u32), ImageError> {
    if data.len() < 12 { return Err(ImageError::DecodeFailed("QOI too short".into())); }
    if &data[0..4] != b"qoif" { return Err(ImageError::DecodeFailed("not a QOI".into())); }
    let w = u32::from_be_bytes(data[4..8].try_into().unwrap());
    let h = u32::from_be_bytes(data[8..12].try_into().unwrap());
    Ok((w, h))
}

#[cfg(feature = "decode_qoi")]
pub fn decode_qoi(data: &[u8]) -> Result<ImageData, ImageError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let (header, pixels) = qoi::decode_to_vec(data)
            .map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
        let w = header.width;
        let h = header.height;
        let bgra = match header.channels {
            qoi::Channels::Rgba => {
                let mut out = vec![0u8; pixels.len()];
                for (i, px) in pixels.chunks_exact(4).enumerate() {
                    out[i*4] = px[2]; out[i*4+1] = px[1]; out[i*4+2] = px[0]; out[i*4+3] = px[3];
                }
                out
            },
            qoi::Channels::Rgb => {
                let n = (w * h) as usize;
                let mut out = vec![0u8; n * 4];
                for (i, px) in pixels.chunks_exact(3).enumerate() {
                    out[i*4] = px[2]; out[i*4+1] = px[1]; out[i*4+2] = px[0]; out[i*4+3] = 0xFF;
                }
                out
            },
        };
        Ok(ImageData { pixels: bgra, width: w, height: h, stride: w * 4,
                       format: PixelFormat::Bgra32, original_format: ImageFormat::Qoi })
    })).map_err(|_| ImageError::DecodePanic)?
}

/// Encode ImageData (Bgra32) to QOI bytes.
#[cfg(feature = "decode_qoi")]
pub fn encode_qoi(img: &ImageData) -> Result<Vec<u8>, ImageError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Convert BGRA32 → RGBA for qoi encoder
        let rgba: Vec<u8> = img.pixels.chunks_exact(4).flat_map(|px| {
            [px[2], px[1], px[0], px[3]]
        }).collect();
        qoi::encode_to_vec(&rgba, img.width, img.height)
            .map_err(|e| ImageError::EncodeFailed(e.to_string()))
    })).map_err(|_| ImageError::DecodePanic)?
}

#[cfg(not(feature = "decode_qoi"))]
pub fn decode_qoi(_data: &[u8]) -> Result<ImageData, ImageError> { Err(ImageError::FormatNotEnabled) }
#[cfg(not(feature = "decode_qoi"))]
pub fn encode_qoi(_img: &ImageData) -> Result<Vec<u8>, ImageError> { Err(ImageError::FormatNotEnabled) }
```

```rust
// supervisor/src/image/decode_svg.rs  (~90 lines)

use crate::image::types::{ImageData, ImageError, ImageFormat, PixelFormat};

/// Build usvg options with ALL external resources disabled (B1).
/// Version constraint: usvg 0.37+ is required. usvg < 0.37 enables fs access by default.
#[cfg(feature = "decode_svg")]
fn make_usvg_opts() -> usvg::Options {
    let mut opts = usvg::Options::default();
    // No resources_dir means no relative-path filesystem resolution
    opts.resources_dir = None;
    opts.font_family = "VyomaSans".into();
    opts.dpi = 96.0;
    // The default ImageHrefResolver in usvg 0.37+ does NOT load from disk
    // unless `resources_dir` is set. Explicit documentation: usvg changelog 0.37.
    // We do NOT call opts.image_href_resolver = ... because the default
    // already rejects external hrefs when resources_dir is None.
    opts
}

/// Rasterize SVG to Bgra32 at the given dimensions.
/// External resources (images, stylesheets, fonts via href) are fully disabled (B1).
#[cfg(feature = "decode_svg")]
pub fn decode_svg(data: &[u8], width: u32, height: u32) -> Result<ImageData, ImageError> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        decode_svg_inner(data, width, height)
    })).map_err(|_| ImageError::DecodePanic)?
}

#[cfg(feature = "decode_svg")]
fn decode_svg_inner(data: &[u8], width: u32, height: u32) -> Result<ImageData, ImageError> {
    let opts = make_usvg_opts();
    let tree = usvg::Tree::from_data(data, &opts)
        .map_err(|e| ImageError::DecodeFailed(format!("svg parse: {e}")))?;
    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .ok_or(ImageError::InvalidDimensions)?;
    resvg::render(
        &tree,
        resvg::FitTo::Width(width),
        tiny_skia::Transform::default(),
        pixmap.as_mut(),
    ).ok_or_else(|| ImageError::DecodeFailed("resvg render returned None".into()))?;
    // tiny-skia outputs pre-multiplied RGBA; convert to straight-alpha BGRA32
    let bgra = premul_rgba_to_straight_bgra(pixmap.data());
    Ok(ImageData {
        pixels: bgra, width, height,
        stride: width * 4,
        format: PixelFormat::Bgra32,
        original_format: ImageFormat::Svg,
    })
}

#[cfg(feature = "decode_svg")]
fn premul_rgba_to_straight_bgra(premul: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; premul.len()];
    for (i, px) in premul.chunks_exact(4).enumerate() {
        let (r, g, b, a) = (px[0] as u32, px[1] as u32, px[2] as u32, px[3] as u32);
        let (r, g, b) = if a == 0 {
            (0, 0, 0)
        } else {
            ((r * 255 + a / 2) / a, (g * 255 + a / 2) / a, (b * 255 + a / 2) / a)
        };
        out[i*4]   = b as u8;  // B
        out[i*4+1] = g as u8;  // G
        out[i*4+2] = r as u8;  // R
        out[i*4+3] = a as u8;  // A
    }
    out
}

/// SVG does not have a fixed pixel dimension in the header; return target size directly.
pub fn svg_dimensions(cfg_target_w: u32, cfg_target_h: u32) -> (u32, u32) {
    (cfg_target_w, cfg_target_h)
}

#[cfg(not(feature = "decode_svg"))]
pub fn decode_svg(_data: &[u8], _w: u32, _h: u32) -> Result<ImageData, ImageError> {
    Err(ImageError::FormatNotEnabled)
}
```

```rust
// supervisor/src/image/decode.rs  (~80 lines)
// Top-level dispatcher: detect format, check dimensions BEFORE allocation (B4),
// check decoded budget (B6), dispatch to per-format decoder.

use std::sync::Arc;
use dashmap::DashMap;
use crate::image::{
    config::ImageConfig,
    detect::detect_format,
    types::{ImageData, ImageError, ImageFormat, PixelFormat},
    decode_png, decode_jpeg, decode_bmp, decode_qoi, decode_svg,
};

pub struct DecodeRequest<'a> {
    pub data: &'a [u8],
    pub format_hint: Option<ImageFormat>,
    /// Target dimensions for SVG rasterization. Ignored for raster formats.
    pub svg_target_width: u32,
    pub svg_target_height: u32,
    pub pid: u32,
}

pub fn decode_image(
    req: &DecodeRequest<'_>,
    cfg: &ImageConfig,
    per_app_usage: &DashMap<u32, usize>,
) -> Result<ImageData, ImageError> {
    // 1. Raw size check — before any parsing
    if req.data.len() > cfg.max_raw_bytes {
        return Err(ImageError::TooLarge(req.data.len()));
    }
    // 2. Detect format (magic bytes override hint for security)
    let detected = detect_format(req.data);
    let fmt = detected.or(req.format_hint)
        .ok_or_else(|| ImageError::DecodeFailed("unknown format".into()))?;
    // 3. Parse header → dimensions BEFORE pixel allocation (B4)
    let (w, h) = match fmt {
        ImageFormat::Png  => decode_png::png_dimensions(req.data)?,
        ImageFormat::Jpeg => decode_jpeg::jpeg_dimensions(req.data)?,
        ImageFormat::Bmp  => decode_bmp::bmp_header_dimensions(req.data)?,
        ImageFormat::Qoi  => decode_qoi::qoi_dimensions(req.data)?,
        ImageFormat::Svg  => (req.svg_target_width, req.svg_target_height),
    };
    cfg.check_dimensions(w, h)?;
    // 4. Decoded-bytes budget check BEFORE allocation (B6)
    let decoded_bytes = (w as usize) * (h as usize) * 4;
    check_decoded_budget(req.pid, decoded_bytes, cfg, per_app_usage)?;
    // 5. Decode pixels
    let img = match fmt {
        ImageFormat::Png  => decode_png::decode_png(req.data)?,
        ImageFormat::Jpeg => decode_jpeg::decode_jpeg(req.data)?,
        ImageFormat::Bmp  => decode_bmp::decode_bmp(req.data)?,
        ImageFormat::Qoi  => decode_qoi::decode_qoi(req.data)?,
        ImageFormat::Svg  => decode_svg::decode_svg(req.data, w, h)?,
    };
    Ok(img)
}

/// Charge `decoded_bytes` against PID's budget. Returns BudgetExceeded if over limit (B6).
pub fn check_decoded_budget(
    pid: u32,
    decoded_bytes: usize,
    cfg: &ImageConfig,
    per_app_usage: &DashMap<u32, usize>,
) -> Result<(), ImageError> {
    let current = per_app_usage.get(&pid).map(|v| *v).unwrap_or(0);
    if current + decoded_bytes > cfg.max_decoded_bytes_per_pid {
        return Err(ImageError::BudgetExceeded {
            pid,
            requested: decoded_bytes,
            limit: cfg.max_decoded_bytes_per_pid,
        });
    }
    *per_app_usage.entry(pid).or_default() += decoded_bytes;
    Ok(())
}

/// Release all budget for a PID (called on app exit).
pub fn release_budget(pid: u32, per_app_usage: &DashMap<u32, usize>) {
    per_app_usage.remove(&pid);
}
```

---

## §5 Resize

```rust
// supervisor/src/image/resize.rs  (~160 lines)

use crate::image::types::{ImageData, ImageError, ImageFormat, PixelFormat, ResizeFilter};

pub fn resize(
    src: &ImageData,
    filter: ResizeFilter,
    dst_w: u32,
    dst_h: u32,
) -> Result<ImageData, ImageError> {
    if dst_w == 0 || dst_h == 0 { return Err(ImageError::InvalidDimensions); }
    if src.format != PixelFormat::Bgra32 {
        return Err(ImageError::UnsupportedFormat("resize: only Bgra32 input supported"));
    }
    match filter {
        ResizeFilter::Nearest  => Ok(resize_nearest(src, dst_w, dst_h)),
        ResizeFilter::Bilinear => Ok(resize_bilinear(src, dst_w, dst_h)),
        ResizeFilter::Lanczos3 => resize_lanczos3(src, dst_w, dst_h),
    }
}

fn resize_nearest(src: &ImageData, dst_w: u32, dst_h: u32) -> ImageData {
    let (sw, sh) = (src.width, src.height);
    let mut out = vec![0u8; (dst_w * dst_h * 4) as usize];
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let sx = (dx * sw / dst_w) as usize;
            let sy = (dy * sh / dst_h) as usize;
            let src_i = (sy * sw as usize + sx) * 4;
            let dst_i = (dy as usize * dst_w as usize + dx as usize) * 4;
            out[dst_i..dst_i+4].copy_from_slice(&src.pixels[src_i..src_i+4]);
        }
    }
    ImageData { pixels: out, width: dst_w, height: dst_h, stride: dst_w * 4,
                format: PixelFormat::Bgra32, original_format: src.original_format }
}

fn resize_bilinear(src: &ImageData, dst_w: u32, dst_h: u32) -> ImageData {
    let mut out = vec![0u8; (dst_w * dst_h * 4) as usize];
    resize_bilinear_inner(&src.pixels, src.width, src.height, &mut out, dst_w, dst_h);
    ImageData { pixels: out, width: dst_w, height: dst_h, stride: dst_w * 4,
                format: PixelFormat::Bgra32, original_format: src.original_format }
}

/// B9 fix: loop 0..4 (not 0..3) to include alpha channel.
fn resize_bilinear_inner(
    src: &[u8], src_w: u32, src_h: u32,
    dst: &mut [u8], dst_w: u32, dst_h: u32,
) {
    let scale_x = src_w as f32 / dst_w as f32;
    let scale_y = src_h as f32 / dst_h as f32;

    #[inline]
    fn src_pixel(src: &[u8], src_w: u32, x: u32, y: u32, ch: usize) -> f32 {
        src[(y as usize * src_w as usize + x as usize) * 4 + ch] as f32
    }

    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let fx_raw = (dx as f32 + 0.5) * scale_x - 0.5;
            let fy_raw = (dy as f32 + 0.5) * scale_y - 0.5;
            let x0 = (fx_raw.floor() as i32).max(0) as u32;
            let y0 = (fy_raw.floor() as i32).max(0) as u32;
            let x1 = (x0 + 1).min(src_w - 1);
            let y1 = (y0 + 1).min(src_h - 1);
            let fx = fx_raw - fx_raw.floor();
            let fy = fy_raw - fy_raw.floor();
            let dst_idx = (dy as usize * dst_w as usize + dx as usize) * 4;
            // B9: iterate all 4 channels including alpha (ch in 0..4)
            for ch in 0..4usize {
                let tl = src_pixel(src, src_w, x0, y0, ch);
                let tr = src_pixel(src, src_w, x1, y0, ch);
                let bl = src_pixel(src, src_w, x0, y1, ch);
                let br = src_pixel(src, src_w, x1, y1, ch);
                let top = tl + (tr - tl) * fx;
                let bot = bl + (br - bl) * fx;
                dst[dst_idx + ch] = (top + (bot - top) * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// Lanczos3 two-pass (horizontal then vertical) in linear light.
/// sRGB gamma is expanded before filtering and compressed after.
fn resize_lanczos3(src: &ImageData, dst_w: u32, dst_h: u32) -> Result<ImageData, ImageError> {
    // Expand sRGB → linear f32
    let linear = srgb_to_linear(&src.pixels);
    // Horizontal pass: src_w × src_h → dst_w × src_h
    let h_pass = lanczos3_pass_h(&linear, src.width, src.height, dst_w);
    // Vertical pass: dst_w × src_h → dst_w × dst_h
    let v_pass = lanczos3_pass_v(&h_pass, dst_w, src.height, dst_h);
    // Compress linear → sRGB u8
    let srgb = linear_to_srgb(&v_pass);
    Ok(ImageData { pixels: srgb, width: dst_w, height: dst_h, stride: dst_w * 4,
                   format: PixelFormat::Bgra32, original_format: src.original_format })
}

fn lanczos3_kernel(x: f32) -> f32 {
    if x.abs() < 1e-6 { return 1.0; }
    if x.abs() >= 3.0 { return 0.0; }
    let px = std::f32::consts::PI * x;
    let px3 = std::f32::consts::PI * x / 3.0;
    (px.sin() / px) * (px3.sin() / px3)
}

fn lanczos3_pass_h(src: &[f32], src_w: u32, src_h: u32, dst_w: u32) -> Vec<f32> {
    let scale = src_w as f32 / dst_w as f32;
    let filter_radius = if scale > 1.0 { 3.0 * scale } else { 3.0 };
    let mut out = vec![0.0f32; (dst_w * src_h * 4) as usize];
    for y in 0..src_h as usize {
        for dx in 0..dst_w as usize {
            let center = (dx as f32 + 0.5) * scale - 0.5;
            let x_start = (center - filter_radius).ceil() as i32;
            let x_end   = (center + filter_radius).floor() as i32;
            for ch in 0..4usize {
                let mut sum = 0.0f32;
                let mut weight = 0.0f32;
                for sx in x_start..=x_end {
                    let sx_c = sx.clamp(0, src_w as i32 - 1) as usize;
                    let k = lanczos3_kernel((sx as f32 - center) / scale.max(1.0));
                    sum += src[(y * src_w as usize + sx_c) * 4 + ch] * k;
                    weight += k;
                }
                out[(y * dst_w as usize + dx) * 4 + ch] = if weight != 0.0 { sum / weight } else { 0.0 };
            }
        }
    }
    out
}

fn lanczos3_pass_v(src: &[f32], src_w: u32, src_h: u32, dst_h: u32) -> Vec<f32> {
    let scale = src_h as f32 / dst_h as f32;
    let filter_radius = if scale > 1.0 { 3.0 * scale } else { 3.0 };
    let mut out = vec![0.0f32; (src_w * dst_h * 4) as usize];
    for x in 0..src_w as usize {
        for dy in 0..dst_h as usize {
            let center = (dy as f32 + 0.5) * scale - 0.5;
            let y_start = (center - filter_radius).ceil() as i32;
            let y_end   = (center + filter_radius).floor() as i32;
            for ch in 0..4usize {
                let mut sum = 0.0f32;
                let mut weight = 0.0f32;
                for sy in y_start..=y_end {
                    let sy_c = sy.clamp(0, src_h as i32 - 1) as usize;
                    let k = lanczos3_kernel((sy as f32 - center) / scale.max(1.0));
                    sum += src[(sy_c * src_w as usize + x) * 4 + ch] * k;
                    weight += k;
                }
                out[(dy * src_w as usize + x) * 4 + ch] = if weight != 0.0 { sum / weight } else { 0.0 };
            }
        }
    }
    out
}

fn srgb_to_linear(px: &[u8]) -> Vec<f32> {
    px.iter().enumerate().map(|(i, &v)| {
        if i % 4 == 3 { v as f32 / 255.0 } // alpha: linear
        else {
            let s = v as f32 / 255.0;
            if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
        }
    }).collect()
}

fn linear_to_srgb(px: &[f32]) -> Vec<u8> {
    px.iter().enumerate().map(|(i, &v)| {
        if i % 4 == 3 { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } // alpha: linear
        else {
            let s = v.clamp(0.0, 1.0);
            let g = if s <= 0.0031308 { s * 12.92 } else { 1.055 * s.powf(1.0/2.4) - 0.055 };
            (g * 255.0).round() as u8
        }
    }).collect()
}
```

---

## §6 Image Worker

```rust
// supervisor/src/image/worker.rs  (~180 lines)

use std::sync::Arc;
use std::time::Duration;
use crossbeam::channel::{self, Sender, Receiver, RecvTimeoutError};
use crate::image::{
    config::ImageConfig,
    decode::{decode_image, DecodeRequest, release_budget},
    encode_qoi,
    resize::resize,
    registry::ImageRegistry,
    types::{ImageData, ImageError, ImageFormat, ImageCacheKey, PixelFormat, ResizeFilter},
};
use dashmap::DashMap;

/// Request types dispatched to image worker threads.
pub enum ImageReq {
    /// Decode from a filesystem path (supervisor-side path validation must precede this).
    Decode {
        path: Arc<std::path::PathBuf>,
        format_hint: Option<ImageFormat>,
        pid: u32,
        reply: oneshot::Sender<Result<u32, ImageError>>, // returns image handle
    },
    /// Decode from raw bytes already loaded by the caller.
    DecodeBytes {
        data: Vec<u8>,
        format_hint: Option<ImageFormat>,
        /// SVG rasterization target; ignored for raster formats.
        svg_w: u32,
        svg_h: u32,
        pid: u32,
        reply: oneshot::Sender<Result<u32, ImageError>>,
    },
    /// Resize a cached image handle to new dimensions, returning new handle.
    Resize {
        handle: u32,
        filter: ResizeFilter,
        dst_w: u32,
        dst_h: u32,
        pid: u32,
        reply: oneshot::Sender<Result<u32, ImageError>>,
    },
    /// Encode an image handle to QOI bytes (for compositor thumbnails).
    EncodeQoi {
        handle: u32,
        reply: oneshot::Sender<Result<Vec<u8>, ImageError>>,
    },
    /// Internal: worker alive ping for monitor thread.
    Ping {
        reply: oneshot::Sender<()>,
    },
    /// Shutdown signal.
    Shutdown,
}

pub struct ImageWorker {
    tx: Sender<ImageReq>,
}

impl ImageWorker {
    /// Spawn worker pool sized by cfg.decode_worker_threads (B5).
    /// Each worker gets a monitor thread that respawns it on panic (R13 pattern).
    pub fn spawn(registry: Arc<ImageRegistry>, cfg: Arc<ImageConfig>) -> Self {
        let pool_size = cfg.decode_worker_threads.max(1);
        let (tx, rx) = channel::bounded::<ImageReq>(pool_size * 16);
        for i in 0..pool_size {
            let rx2 = rx.clone();
            let reg = registry.clone();
            let cfg2 = cfg.clone();
            spawn_worker_with_monitor(i, rx2, reg, cfg2);
        }
        Self { tx }
    }

    /// Send a request with a timeout. Returns WorkerTimeout if channel is full.
    pub fn send(&self, req: ImageReq) -> Result<(), ImageError> {
        self.tx.send_timeout(req, Duration::from_millis(200))
            .map_err(|_| ImageError::WorkerTimeout)
    }
}

fn spawn_worker_with_monitor(
    idx: usize,
    rx: Receiver<ImageReq>,
    registry: Arc<ImageRegistry>,
    cfg: Arc<ImageConfig>,
) {
    std::thread::Builder::new()
        .name(format!("vyoma-img-monitor-{idx}"))
        .spawn(move || {
            loop {
                let rx2 = rx.clone();
                let reg2 = registry.clone();
                let cfg3 = cfg.clone();
                let result = std::thread::Builder::new()
                    .name(format!("vyoma-img-worker-{idx}"))
                    .spawn(move || image_worker_loop(rx2, reg2, cfg3))
                    .expect("spawn image worker")
                    .join();
                match result {
                    Ok(_) => break, // clean shutdown via Shutdown message
                    Err(_) => {
                        log::error!("[image] worker-{idx} panicked, respawning");
                        std::thread::sleep(Duration::from_millis(100));
                        // Continue loop → respawn
                    }
                }
            }
        })
        .expect("spawn image monitor");
}

fn image_worker_loop(
    rx: Receiver<ImageReq>,
    registry: Arc<ImageRegistry>,
    cfg: Arc<ImageConfig>,
) {
    let per_app_usage: Arc<DashMap<u32, usize>> = registry.per_app_usage();
    let timeout = Duration::from_millis(cfg.worker_timeout_ms);

    loop {
        let req = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(r) => r,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        match req {
            ImageReq::Decode { path, format_hint, pid, reply } => {
                let result = handle_decode_path(&path, format_hint, pid, &cfg, &per_app_usage, &registry);
                let _ = reply.send(result);
            },
            ImageReq::DecodeBytes { data, format_hint, svg_w, svg_h, pid, reply } => {
                let dreq = DecodeRequest {
                    data: &data, format_hint, svg_target_width: svg_w,
                    svg_target_height: svg_h, pid,
                };
                let result = decode_image(&dreq, &cfg, &per_app_usage)
                    .and_then(|img| registry.store(img, pid));
                let _ = reply.send(result);
            },
            ImageReq::Resize { handle, filter, dst_w, dst_h, pid, reply } => {
                let result = registry.get(handle)
                    .ok_or(ImageError::DecodeFailed("handle not found".into()))
                    .and_then(|img| resize(&img, filter, dst_w, dst_h))
                    .and_then(|img| registry.store(img, pid));
                let _ = reply.send(result);
            },
            ImageReq::EncodeQoi { handle, reply } => {
                let result = registry.get(handle)
                    .ok_or(ImageError::DecodeFailed("handle not found".into()))
                    .and_then(|img| encode_qoi::encode_qoi(&img));
                let _ = reply.send(result);
            },
            ImageReq::Ping { reply } => { let _ = reply.send(()); },
            ImageReq::Shutdown => break,
        }
    }
}

fn handle_decode_path(
    path: &std::path::PathBuf,
    format_hint: Option<ImageFormat>,
    pid: u32,
    cfg: &ImageConfig,
    per_app_usage: &DashMap<u32, usize>,
    registry: &ImageRegistry,
) -> Result<u32, ImageError> {
    // Check cache first
    let key = ImageCacheKey::new(Arc::new(path.clone()), 0, 0, PixelFormat::Bgra32);
    if let Some(handle) = registry.cache_lookup(&key) {
        return Ok(handle);
    }
    let data = std::fs::read(path).map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
    let dreq = DecodeRequest {
        data: &data, format_hint,
        svg_target_width: cfg.max_image_dimension / 4,
        svg_target_height: cfg.max_image_dimension / 4,
        pid,
    };
    let img = decode_image(&dreq, cfg, per_app_usage)?;
    let handle = registry.store_with_key(img, pid, key)?;
    Ok(handle)
}
```

---

## §7 Image Cache

```rust
// supervisor/src/image/registry.rs  (~170 lines)

use std::sync::{Arc, Mutex, RwLock};
use std::path::PathBuf;
use lru::LruCache;
use dashmap::DashMap;
use ahash::AHashMap;
use crate::image::types::{ImageCacheKey, ImageData, ImageError, PixelFormat};

/// Slot-based store for decoded images. Handles are u32 indices.
/// Separate LRU cache for path-keyed entries.
pub struct ImageRegistry {
    /// Slot store: handle → ImageData. Protected by RwLock.
    slots: RwLock<Vec<Option<Arc<ImageData>>>>,
    /// LRU cache of path-keyed images (full-path key, B2).
    path_cache: Mutex<LruCache<ImageCacheKey, u32>>,
    /// Per-app decoded-bytes usage (B6). Shared with decode pipeline.
    pub(crate) usage: Arc<DashMap<u32, usize>>,
    /// Next slot index (reuses freed slots via free_list).
    next_handle: Mutex<(u32, Vec<u32>)>,
}

impl ImageRegistry {
    pub fn new(cache_capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            slots: RwLock::new(Vec::with_capacity(256)),
            path_cache: Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(cache_capacity).unwrap()
            )),
            usage: Arc::new(DashMap::new()),
            next_handle: Mutex::new((0, Vec::new())),
        })
    }

    pub fn per_app_usage(&self) -> Arc<DashMap<u32, usize>> {
        self.usage.clone()
    }

    /// Allocate a handle slot and store an ImageData.
    pub fn store(&self, img: ImageData, _pid: u32) -> Result<u32, ImageError> {
        let handle = self.alloc_handle();
        let mut slots = self.slots.write().unwrap();
        if handle as usize >= slots.len() {
            slots.resize(handle as usize + 1, None);
        }
        slots[handle as usize] = Some(Arc::new(img));
        Ok(handle)
    }

    /// Store with a path cache key for deduplication (B2).
    pub fn store_with_key(&self, img: ImageData, pid: u32, key: ImageCacheKey) -> Result<u32, ImageError> {
        let handle = self.store(img, pid)?;
        self.path_cache.lock().unwrap().put(key, handle);
        Ok(handle)
    }

    /// Look up by path cache key. Returns handle if cached and slot still valid.
    pub fn cache_lookup(&self, key: &ImageCacheKey) -> Option<u32> {
        let mut cache = self.path_cache.lock().unwrap();
        let &handle = cache.get(key)?;
        let slots = self.slots.read().unwrap();
        if slots.get(handle as usize).and_then(|s| s.as_ref()).is_some() {
            Some(handle)
        } else {
            None
        }
    }

    /// Get ImageData by handle. Returns None if handle is invalid or freed.
    pub fn get(&self, handle: u32) -> Option<Arc<ImageData>> {
        let slots = self.slots.read().unwrap();
        slots.get(handle as usize)?.clone()
    }

    /// Free a handle. Decrements usage budget for the owning PID.
    pub fn free(&self, handle: u32, pid: u32) {
        let mut slots = self.slots.write().unwrap();
        if let Some(slot) = slots.get_mut(handle as usize) {
            if let Some(img) = slot.take() {
                let bytes = img.decoded_bytes();
                let mut usage = self.usage.entry(pid).or_default();
                *usage = usage.saturating_sub(bytes);
            }
        }
        self.next_handle.lock().unwrap().1.push(handle);
    }

    /// Free all handles owned by a PID (called on app exit).
    pub fn free_pid(&self, pid: u32) {
        self.usage.remove(&pid);
        // Handles not individually tracked per-pid in v1; rely on budget accounting.
        // Full per-handle pid tracking deferred to v2.
    }

    fn alloc_handle(&self) -> u32 {
        let mut guard = self.next_handle.lock().unwrap();
        if let Some(h) = guard.1.pop() { h }
        else { let h = guard.0; guard.0 += 1; h }
    }
}

/// GPU texture cache: maps ImageCacheKey → wgpu texture handle (u32 from R12).
/// Only active on platforms with gpu_enabled = true.
pub struct GpuImageCache {
    inner: Mutex<LruCache<ImageCacheKey, u32>>,
    capacity: usize,
}

impl GpuImageCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(capacity.max(1)).unwrap()
            )),
            capacity,
        }
    }

    pub fn get(&self, key: &ImageCacheKey) -> Option<u32> {
        self.inner.lock().unwrap().get(key).copied()
    }

    pub fn put(&self, key: ImageCacheKey, texture_handle: u32) {
        self.inner.lock().unwrap().put(key, texture_handle);
    }

    /// Evict on VRAM pressure: remove oldest N entries, returning their handles for R12 cleanup.
    pub fn evict_lru(&self, count: usize) -> Vec<u32> {
        let mut cache = self.inner.lock().unwrap();
        let mut evicted = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some((_, handle)) = cache.pop_lru() {
                evicted.push(handle);
            }
        }
        evicted
    }
}
```

---

## §8 Icon System

```rust
// supervisor/src/image/icons.rs  (~150 lines)

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use lru::LruCache;
use crate::image::{
    config::ImageConfig,
    decode::{decode_image, check_decoded_budget, DecodeRequest},
    registry::ImageRegistry,
    resize::resize,
    types::{IconSet, ImageData, ImageError, ImageFormat, ImageCacheKey, PixelFormat, ResizeFilter},
};
use dashmap::DashMap;

/// Load an `IconSet` from a directory.
/// Expects files named `icon-{N}.png` for N in canonical_sizes.
/// Missing sizes are synthesized by downscaling the nearest larger size.
pub fn load_icon_set(
    dir: &Path,
    name: &str,
    cfg: &ImageConfig,
    per_app_usage: &DashMap<u32, usize>,
) -> Result<IconSet, ImageError> {
    let canonical = IconSet::canonical_sizes();
    let mut sizes: BTreeMap<u32, ImageData> = BTreeMap::new();

    // 1. Load all present sizes
    for &sz in canonical {
        let path = dir.join(format!("icon-{sz}.png"));
        if path.exists() {
            let data = std::fs::read(&path).map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
            let dreq = DecodeRequest {
                data: &data, format_hint: Some(ImageFormat::Png),
                svg_target_width: sz, svg_target_height: sz, pid: 0,
            };
            let img = decode_image(&dreq, cfg, per_app_usage)?;
            sizes.insert(sz, img);
        }
    }

    // Also try icon.svg as source for all sizes on platforms with svg_enabled
    if sizes.is_empty() && cfg.svg_enabled {
        let svg_path = dir.join("icon.svg");
        if svg_path.exists() {
            let data = std::fs::read(&svg_path).map_err(|e| ImageError::DecodeFailed(e.to_string()))?;
            for &sz in canonical {
                let dreq = DecodeRequest {
                    data: &data, format_hint: Some(ImageFormat::Svg),
                    svg_target_width: sz, svg_target_height: sz, pid: 0,
                };
                if let Ok(img) = decode_image(&dreq, cfg, per_app_usage) {
                    sizes.insert(sz, img);
                }
            }
        }
    }

    if sizes.is_empty() {
        return Err(ImageError::IconNotFound { name: name.to_string(), size: 0 });
    }

    // 2. Synthesize missing sizes by downscaling from nearest larger
    let present: Vec<u32> = sizes.keys().copied().collect();
    for &needed in canonical {
        if sizes.contains_key(&needed) { continue; }
        // Find smallest source >= needed; fallback to largest available
        let source_sz = present.iter().filter(|&&s| s >= needed).copied().min()
            .or_else(|| present.iter().copied().max())
            .unwrap();
        let source = sizes[&source_sz].clone();
        let synthesized = resize(&source, cfg.icon_resize_filter, needed, needed)?;
        sizes.insert(needed, synthesized);
    }

    Ok(IconSet { name: name.to_string(), sizes })
}

/// Pick best icon data for a requested logical pixel size at a display scale factor.
/// `scale` is the display scale factor (e.g., 2.0 for Retina / HiDPI).
/// Returns a clone of the best matching ImageData. Synthesizes via resize if exact match missing.
pub fn icon_for_size(
    set: &IconSet,
    logical_px: u32,
    scale: f32,
    filter: ResizeFilter,
) -> Result<Arc<ImageData>, ImageError> {
    let physical_px = ((logical_px as f32) * scale).round() as u32;
    // Exact match
    if let Some(img) = set.sizes.get(&physical_px) {
        return Ok(Arc::new(img.clone()));
    }
    // Nearest larger (prefer downscale over upscale for quality)
    let best = set.sizes.range(physical_px..).next()
        .or_else(|| set.sizes.range(..physical_px).next_back());
    let (_, source) = best.ok_or_else(|| ImageError::IconNotFound {
        name: set.name.clone(), size: physical_px
    })?;
    let resized = resize(source, filter, physical_px, physical_px)?;
    Ok(Arc::new(resized))
}

/// System-wide icon cache with scale-factor awareness (B2 full-path key).
/// Key: (canonical icon directory path, logical_px, scale_factor_x100)
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct IconCacheKey {
    pub path: Arc<PathBuf>,
    pub logical_px: u32,
    /// scale * 100 as u32 (e.g. 200 for 2x, 150 for 1.5x).
    pub scale_x100: u32,
}

pub struct SystemIconCache {
    inner: RwLock<LruCache<IconCacheKey, Arc<ImageData>>>,
}

impl SystemIconCache {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: RwLock::new(LruCache::new(
                std::num::NonZeroUsize::new(capacity.max(1)).unwrap()
            )),
        })
    }

    pub fn get(&self, key: &IconCacheKey) -> Option<Arc<ImageData>> {
        self.inner.write().unwrap().get(key).cloned()
    }

    pub fn put(&self, key: IconCacheKey, img: Arc<ImageData>) {
        self.inner.write().unwrap().put(key, img);
    }

    /// Look up or load+cache an icon.
    pub fn get_or_load(
        &self,
        dir: Arc<PathBuf>,
        name: &str,
        logical_px: u32,
        scale: f32,
        cfg: &ImageConfig,
        per_app_usage: &DashMap<u32, usize>,
    ) -> Result<Arc<ImageData>, ImageError> {
        let scale_x100 = (scale * 100.0).round() as u32;
        let key = IconCacheKey { path: dir.clone(), logical_px, scale_x100 };
        if let Some(cached) = self.get(&key) {
            return Ok(cached);
        }
        let set = load_icon_set(&dir, name, cfg, per_app_usage)?;
        let img = icon_for_size(&set, logical_px, scale, cfg.icon_resize_filter)?;
        self.put(key, img.clone());
        Ok(img)
    }
}
```

---

## §9 Surface Integration

```rust
// supervisor/src/image/surface_ext.rs  (~100 lines)
// Extension methods for R11 Surface type, adding image-aware blit and snapshot.

use crate::display::surface::{Surface, Rect};
use crate::image::types::{ImageData, ImageError, PixelFormat};

impl Surface {
    /// Blit an ImageData onto this surface using straight-over alpha composite.
    /// `src_rect`: None = full image. `dst_*`: destination rectangle (scaled blit).
    pub fn blit_rgba(
        &mut self,
        src: &ImageData,
        src_rect: Option<Rect>,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) -> Result<(), ImageError> {
        if src.format != PixelFormat::Bgra32 {
            return Err(ImageError::UnsupportedFormat("blit_rgba: only Bgra32 source supported"));
        }
        let (sx, sy, sw, sh) = match src_rect {
            None => (0, 0, src.width, src.height),
            Some(r) => (r.x as u32, r.y as u32, r.w, r.h),
        };
        // If src and dst dimensions match, use direct blit without resize
        if sw == dst_w && sh == dst_h {
            self.blit_bgra32_direct(src, sx, sy, sw, sh, dst_x, dst_y);
        } else {
            // Scale src_rect to dst dimensions using nearest-neighbor inline
            self.blit_bgra32_scaled(src, sx, sy, sw, sh, dst_x, dst_y, dst_w, dst_h);
        }
        Ok(())
    }

    fn blit_bgra32_direct(
        &mut self, src: &ImageData,
        sx: u32, sy: u32, sw: u32, sh: u32,
        dst_x: i32, dst_y: i32,
    ) {
        let (dw, dh) = (self.width() as i32, self.height() as i32);
        for row in 0..sh as i32 {
            let dy = dst_y + row;
            let src_y_abs = (sy as i32 + row) as u32;
            if dy < 0 || dy >= dh { continue; }
            for col in 0..sw as i32 {
                let dx = dst_x + col;
                let src_x_abs = (sx as i32 + col) as u32;
                if dx < 0 || dx >= dw { continue; }
                let si = (src_y_abs * src.stride / 4 + src_x_abs) as usize * 4;
                let sb = src.pixels[si];
                let sg = src.pixels[si+1];
                let sr = src.pixels[si+2];
                let sa = src.pixels[si+3];
                self.blend_pixel(dx as u32, dy as u32, sb, sg, sr, sa);
            }
        }
    }

    fn blit_bgra32_scaled(
        &mut self, src: &ImageData,
        sx: u32, sy: u32, sw: u32, sh: u32,
        dst_x: i32, dst_y: i32, dst_w: u32, dst_h: u32,
    ) {
        let (dw, dh) = (self.width() as i32, self.height() as i32);
        for dy_rel in 0..dst_h as i32 {
            let dy = dst_y + dy_rel;
            if dy < 0 || dy >= dh { continue; }
            let src_row = sy + (dy_rel as u32 * sh / dst_h);
            for dx_rel in 0..dst_w as i32 {
                let dx = dst_x + dx_rel;
                if dx < 0 || dx >= dw { continue; }
                let src_col = sx + (dx_rel as u32 * sw / dst_w);
                let si = (src_row * src.stride / 4 + src_col) as usize * 4;
                let sb = src.pixels[si];
                let sg = src.pixels[si+1];
                let sr = src.pixels[si+2];
                let sa = src.pixels[si+3];
                self.blend_pixel(dx as u32, dy as u32, sb, sg, sr, sa);
            }
        }
    }

    /// Create a Surface pre-filled with an ImageData (takes ownership of pixels via copy).
    pub fn from_image_data(img: &ImageData) -> Result<Surface, ImageError> {
        if img.format != PixelFormat::Bgra32 {
            return Err(ImageError::UnsupportedFormat("from_image_data: only Bgra32 supported"));
        }
        let mut surf = Surface::new(img.width, img.height);
        surf.pixels_mut().copy_from_slice(&img.pixels);
        Ok(surf)
    }

    /// Snapshot current surface pixels as QOI bytes (for Mission Control / compositor thumbnails).
    pub fn snapshot_qoi(&self) -> Result<Vec<u8>, ImageError> {
        let img = ImageData {
            pixels: self.pixels().to_vec(),
            width: self.width(),
            height: self.height(),
            stride: self.width() * 4,
            format: PixelFormat::Bgra32,
            original_format: crate::image::types::ImageFormat::Qoi,
        };
        crate::image::decode_qoi::encode_qoi(&img)
    }
}
```

---

## §10 WIT Interface

```wit
// supervisor/wit/vyoma-images.wit

package vyoma:images@1.0.0;

/// Image I/O world — exposed to all apps with [capabilities.images] declared.
world io {
    import images-io;
    import images-resize;
}

/// Icon query world — exposed to apps with [capabilities.images.icons] declared.
world icons {
    import images-io;
    import images-icons;
}

/// Core image load/blit/free operations.
interface images-io {
    /// Pixel format for decoded images. Only bgra32 is supported in v1.
    enum pixel-format {
        bgra32,
        rgb565,
        grayscale8,
    }

    /// Image format hint for decode-bytes.
    enum image-format {
        png,
        jpeg,
        bmp,
        qoi,
        svg,
    }

    /// Decode image from a path within the app's declared capability directory.
    /// Returns an opaque image handle valid until free-image is called.
    /// Error: path-traversal, too-large, decode-failed, format-not-enabled.
    decode-image: func(
        path: string,
    ) -> result<u32, string>;

    /// Decode image from raw bytes already in WASM linear memory.
    /// `format-hint`: optional format override; magic bytes take precedence.
    /// `svg-target-w`, `svg-target-h`: rasterization size for SVG; ignored for raster.
    decode-bytes: func(
        data: list<u8>,
        format-hint: option<image-format>,
        svg-target-w: u32,
        svg-target-h: u32,
    ) -> result<u32, string>;

    /// Query dimensions of a decoded image handle.
    image-size: func(
        handle: u32,
    ) -> result<tuple<u32, u32>, string>;

    /// Blit a decoded image onto the current window's surface.
    ///
    /// Source crop: `src-w == 0 && src-h == 0` means full image (B7).
    /// Destination rectangle is in window-local pixels.
    /// Alpha compositing: straight-over (src alpha blended onto surface).
    blit-image: func(
        handle: u32,
        src-x: u32,
        src-y: u32,
        src-w: u32,   /// 0 = full width
        src-h: u32,   /// 0 = full height
        dst-x: i32,
        dst-y: i32,
        dst-w: u32,
        dst-h: u32,
    ) -> result<_, string>;

    /// Upload image to GPU as a texture, returning R12 texture handle.
    /// Only valid on platforms with gpu_enabled = true.
    upload-texture: func(
        handle: u32,
    ) -> result<u32, string>;

    /// Release an image handle and reclaim decoded-bytes budget.
    free-image: func(handle: u32);

    /// Encode image handle to QOI bytes (for app-side thumbnail generation).
    encode-qoi: func(
        handle: u32,
    ) -> result<list<u8>, string>;
}

/// Resize operations on image handles.
interface images-resize {
    enum resize-filter {
        nearest,
        bilinear,
        lanczos3,
    }

    /// Resize a decoded image, returning a new handle. Original is unaffected.
    resize-image: func(
        handle: u32,
        filter: resize-filter,
        dst-w: u32,
        dst-h: u32,
    ) -> result<u32, string>;
}

/// Icon system operations.
interface images-icons {
    /// Query system icon for a named icon set at a logical pixel size and scale factor.
    /// scale-x100: display scale * 100 (e.g., 200 for 2x HiDPI).
    /// Returns an image handle valid until free-image is called.
    icon-for-size: func(
        name: string,
        logical-px: u32,
        scale-x100: u32,
    ) -> result<u32, string>;

    /// List available icon set names in the declared icons directory.
    list-icon-sets: func() -> result<list<string>, string>;
}
```

---

## §11 Manifest Capability

```toml
# apps/my-app/vyoma.toml — example images capability declaration

[capabilities]
stdio   = true
display = true

[capabilities.images]
# Directories the app may load images from.
# Paths are relative to the app's declared root (/data/<app-name>/).
# Path traversal (../../) is rejected at canonicalize time.
dirs = ["assets/images", "assets/icons"]

# Enable icon queries from system icon directories.
icons = true

# Maximum raw compressed bytes this app may submit per decode call.
# Overrides platform default if lower. Cannot exceed platform max_raw_bytes.
max_raw_bytes = 4194304   # 4 MB

# Maximum total decoded BGRA pixels live at any time for this app.
# Overrides platform default if lower.
max_decoded_bytes = 67108864  # 64 MB
```

```rust
// supervisor/src/image/capability.rs  (~70 lines)

use std::path::{Path, PathBuf};
use serde::Deserialize;
use crate::image::types::ImageError;

#[derive(Debug, Clone, Deserialize)]
pub struct ImagesCapability {
    /// Directories relative to app root that images may be loaded from.
    #[serde(default)]
    pub dirs: Vec<String>,
    /// Whether icon queries are permitted.
    #[serde(default)]
    pub icons: bool,
    /// Per-app override for max raw compressed bytes (optional).
    pub max_raw_bytes: Option<usize>,
    /// Per-app override for max decoded bytes (optional).
    pub max_decoded_bytes: Option<usize>,
}

impl ImagesCapability {
    /// Validate that `requested_path` stays within one of the declared capability dirs.
    /// Uses canonicalize + starts_with for traversal prevention.
    /// `app_root`: the resolved absolute root for this app (e.g., /data/my-app).
    pub fn validate_path(&self, app_root: &Path, requested_path: &Path) -> Result<PathBuf, ImageError> {
        // Build candidate absolute path
        let candidate = if requested_path.is_absolute() {
            requested_path.to_path_buf()
        } else {
            app_root.join(requested_path)
        };

        // Canonicalize to resolve symlinks and ../ components
        let canonical = candidate.canonicalize().map_err(|_| ImageError::PathTraversal)?;

        // Check against each declared dir
        for dir in &self.dirs {
            let allowed = app_root.join(dir).canonicalize().map_err(|_| ImageError::PathTraversal)?;
            if canonical.starts_with(&allowed) {
                return Ok(canonical);
            }
        }

        // Also allow system icon dirs if icons = true (supervisor-controlled paths)
        Err(ImageError::PathTraversal)
    }

    /// Parse from raw TOML value. Returns error on unknown subkeys.
    pub fn from_toml(value: &toml::Value) -> Result<Self, String> {
        let table = value.as_table().ok_or("images capability must be a table")?;
        let known = ["dirs", "icons", "max_raw_bytes", "max_decoded_bytes"];
        for key in table.keys() {
            if !known.contains(&key.as_str()) {
                return Err(format!("unknown images capability key: '{key}'"));
            }
        }
        value.clone().try_into::<ImagesCapability>().map_err(|e| e.to_string())
    }
}
```

---

## §12 VYOMA_DRAW_V2 Protocol

The `VYOMA_DRAW_V2:` prefix supersedes `VYOMA_DRAW:` for image commands. Both protocols coexist in v1; the supervisor handles both.

```
# Load image from a path within the app's declared capability dirs.
# Returns a numeric handle in the next VYOMA_EVENT line:
#   VYOMA_EVENT:image_loaded:<handle>  (success)
#   VYOMA_EVENT:image_error:<message>  (failure)
VYOMA_DRAW_V2:load_image:<path>

# Blit image with source crop and destination scale.
# src-w/src-h = 0 means full image (B7 backwards-compat).
# Format: handle,sx,sy,sw,sh,dx,dy,dw,dh
VYOMA_DRAW_V2:blit_image:<handle>,<sx>,<sy>,<sw>,<sh>,<dx>,<dy>,<dw>,<dh>

# Blit raw BGRA32 bytes encoded as base64 (for small inline images ≤ 16 KB).
# Format: width,height,<base64-encoded-bgra32>,dx,dy
VYOMA_DRAW_V2:blit_image_raw:<width>,<height>,<base64_bgra>,<dx>,<dy>

# Set the app's window icon from a loaded image handle.
VYOMA_DRAW_V2:set_icon:<handle>

# Free a loaded image handle.
VYOMA_DRAW_V2:unload_image:<handle>

# Query image dimensions; response is VYOMA_EVENT:image_size:<handle>,<w>,<h>
VYOMA_DRAW_V2:image_size:<handle>
```

**Path resolution rules:**
1. Paths starting with `/data/` are validated against `[capabilities.images].dirs`.
2. Relative paths are resolved relative to the app's declared image dirs in order.
3. Any path that canonicalizes outside a declared dir produces `VYOMA_EVENT:image_error:path_traversal`.
4. Absolute paths outside `/data/` are rejected unconditionally.

**Example Rust app using VYOMA_DRAW_V2:**
```rust
// Request load; supervisor dispatches asynchronously
println!("VYOMA_DRAW_V2:load_image:assets/images/splash.png");

// Read response from stdin (supervisor injects VYOMA_EVENT lines)
let mut line = String::new();
std::io::stdin().read_line(&mut line).unwrap();
// line: "VYOMA_EVENT:image_loaded:3\n"
let handle: u32 = line.trim().split(':').last().unwrap().parse().unwrap();

// Blit full image at (0,0) scaled to 960×540
println!("VYOMA_DRAW_V2:blit_image:{handle},0,0,0,0,0,0,960,540");
println!("VYOMA_DRAW:flush");
```

---

## §13 R11 / R12 / R13 Integration

### R11 Surface Integration

`Surface::blit_rgba` (§9) accepts any decoded `ImageData` and performs straight-over alpha compositing. The compositor's per-window `Surface` buffer is the target. `Surface::snapshot_qoi()` produces QOI thumbnails for Mission Control (R25).

```rust
// In compositor flush pass (supervisor/src/compositor.rs):
// After per-app draw commands are processed:
if let Some(thumb_req) = app.needs_thumbnail {
    if let Ok(qoi_bytes) = app.surface.snapshot_qoi() {
        thumbnail_cache.insert(app.pid, qoi_bytes);
    }
}
```

### R12 GPU Texture Upload

When `gpu_enabled = true` and the app calls `upload-texture`:
```rust
// supervisor/src/image/gpu_upload.rs
pub fn upload_image_to_gpu(
    img: &ImageData,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> wgpu::Texture {
    let size = wgpu::Extent3d { width: img.width, height: img.height, depth_or_array_layers: 1 };
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("vyoma_image"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        texture.as_image_copy(),
        &img.pixels,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(img.stride),
            rows_per_image: Some(img.height),
        },
        size,
    );
    texture
}
```

The `GpuImageCache` (§7) maps `ImageCacheKey → u32` texture handles registered in R12's handle table. Eviction from `GpuImageCache` triggers `vyoma:gpu/graphics.destroy_texture(handle)`.

### R13 Font / Image Separation

The GlyphAtlas from R13 and the ImageRegistry from R14 are separate systems. Images are not stored in the font atlas. If a UI framework needs to render icon textures alongside text, it uploads the icon via `upload-texture` to a separate R12 texture, not into the glyph atlas pages.

---

## §14 Security

Six independent hardening layers, each independently enforced:

**Layer 1 — SVG external resource lock-down (B1)**  
`usvg::Options` is constructed with `resources_dir = None`. usvg 0.37+ does not load external `href` resources when `resources_dir` is absent. Version constraint enforced in `Cargo.toml`: `usvg = "0.37"`. No user-supplied font data is passed to the SVG renderer; only the system VyomaSans font name.

**Layer 2 — Header-only dimension check before pixel allocation (B4)**  
Every decoder exposes a `*_dimensions()` function that parses only the compressed header. `decode_image()` calls `*_dimensions()` first, then `cfg.check_dimensions(w, h)?` before any `Vec::with_capacity()`. For SVG, the configured target dimensions are used directly (they are already validated by the caller).

**Layer 3 — Per-PID decoded-bytes budget (B6)**  
`check_decoded_budget(pid, w*h*4, cfg, &per_app_usage)` is called after dimension validation and before pixel allocation. Budget is charged on alloc and released on `free_image` or on app exit via `release_budget(pid)`. Budget is for BGRA32 decoded bytes only, not compressed input.

**Layer 4 — Full-path cache key, no hash (B2)**  
`ImageCacheKey.path` is `Arc<PathBuf>`. All cache lookups use byte-for-byte path equality via the `PathBuf` `PartialEq` implementation. The `ahash` crate provides the `HashMap` hasher for performance; it is keyed on the full path bytes, not a truncated hash.

**Layer 5 — catch_unwind on every decoder**  
All five decoder functions (`decode_png`, `decode_jpeg`, `decode_bmp`, `decode_qoi`, `decode_svg`) wrap their bodies in `std::panic::catch_unwind(AssertUnwindSafe(...))`. A panicking decoder returns `Err(ImageError::DecodePanic)` instead of unwinding the worker thread.

**Layer 6 — Worker watchdog respawn**  
Each worker thread has a corresponding monitor thread (§6) that `join()`s it and respawns on `Err` (panic). The channel remains live across respawns because the `Receiver` is cloned before spawning. A panicking worker does not stall other apps' decode requests because the pool has 1–3 workers.

**Path traversal prevention**  
`ImagesCapability::validate_path()` (§11) calls `std::fs::canonicalize` on the candidate path and verifies it `starts_with` a declared capability dir. Both the candidate and the allowed dir are canonicalized to resolve symlinks. Requests to paths outside declared dirs return `ImageError::PathTraversal`.

---

## §15 Platform Matrix

| Feature | mcu-minimal | iot-edge | robotics-rt | mobile | desktop-full | server-headless |
|---|---|---|---|---|---|---|
| PNG decode | no | yes | yes | yes | yes | yes |
| JPEG decode | no | yes | yes | yes | yes | yes |
| QOI decode | no | yes | yes | yes | yes | yes |
| BMP decode | no | yes | yes | yes | yes | yes |
| SVG decode | no | no | no | yes | yes | yes |
| GPU upload | no | no | no | yes | yes | no |
| EXIF orient | no | yes | yes | yes | yes | yes |
| Icons | 16,32 | 16–64 | 16–128 | 16–256 | 16–512 | 16–512 |
| Worker threads | 0 | 1 | 1 | 2 | 3 | 3 |
| Max decoded MB/app | 0.125 | 8 | 16 | 64 | 256 | 512 |
| Max raw bytes | 64 KB | 4 MB | 8 MB | 32 MB | 128 MB | 256 MB |
| Max dimension px | 256 | 2048 | 4096 | 8192 | 16384 | 32768 |
| Resize filters | Nearest | Nearest | Bilinear | Lanczos3 | Lanczos3 | Nearest |
| LRU cache entries | 8 | 32 | 48 | 128 | 512 | 256 |
| GPU LRU entries | 0 | 0 | 0 | 64 | 256 | 0 |
| Worker timeout ms | 500 | 1000 | 500 | 500 | 500 | 2000 |

---

## §16 Performance Budget

All timings measured on `desktop-full` platform (x86-64, 3 GHz, single core) with decoded output at Bgra32:

| Operation | Input | Time (target) | Notes |
|---|---|---|---|
| PNG decode | 4 MP (2048×2048) | ≤ 50 ms | libpng via `png` crate |
| JPEG decode | 4 MP | ≤ 20 ms | `jpeg-decoder` (pure Rust) |
| QOI decode | 4 MP | ≤ 5 ms | near-memcpy speed |
| QOI encode | 4 MP | ≤ 8 ms | thumbnail path |
| BMP decode | 4 MP (24bpp) | ≤ 10 ms | hand-rolled, no crate |
| SVG raster | 1 MP (1024×1024) | ≤ 100 ms | resvg + tiny-skia |
| SVG raster | 256×256 | ≤ 15 ms | icon rasterization |
| Resize Nearest | 4 MP → 256×256 | ≤ 5 ms | pure integer math |
| Resize Bilinear | 4 MP → 1 MP | ≤ 15 ms | 4-channel fixed |
| Resize Lanczos3 | 4 MP → 1 MP | ≤ 40 ms | two-pass, sRGB-correct |
| GPU upload | 4 MP | ≤ 3 ms | wgpu write_texture |
| Cache lookup | any | ≤ 0.05 ms | LRU + Arc clone |

**Frame-time accounting (desktop-full, 16.67 ms budget at 60 Hz):**
- Decode is always off-frame (worker pool). `blit_image` on a pre-decoded handle: < 1 ms per 1 MP blit.
- SVG icon synthesis at startup: amortized via `SystemIconCache`; not on hot path.
- Worker queue saturation threshold: 3 workers × 16-slot queue = 48 queued requests. At 20 ms/JPEG, saturation occurs at > 150 decode/second. Beyond that, callers receive `WorkerTimeout`.
- Per-60Hz frame: maximum recommended blits = 8 × 1 MP = 8 MP blitted = ~8 ms in blit_bgra32_direct. Allow 6 ms headroom for compositor, font, and IPC.

**iot-edge / robotics-rt (1 worker):**
- Single QOI decode: < 5 ms. PNG: < 50 ms. Worker queue depth: 16.
- Saturates at 1 QOI/5 ms = 200 QOI/sec, 1 PNG/50 ms = 20 PNG/sec.

---

## §17 Cargo Dependencies

```toml
# supervisor/Cargo.toml (image subsystem section)

[features]
default = [
    "decode_png",
    "decode_jpeg",
    "decode_bmp",
    "decode_qoi",
    "exif",
]
# Format features
decode_png   = ["dep:png"]
decode_jpeg  = ["dep:jpeg-decoder"]
decode_bmp   = []                                  # hand-rolled, no crate dep
decode_qoi   = ["dep:qoi"]
decode_svg   = ["dep:resvg", "dep:usvg", "dep:tiny-skia"]   # not default; mobile/desktop/server
# Auxiliary features
exif         = ["dep:kamadak-exif"]                # JPEG EXIF orientation; bundled with decode_jpeg
image_gpu    = ["dep:wgpu"]                        # GPU texture upload via R12; mobile/desktop only

[dependencies]
# Image decode/encode (all optional, feature-gated — B8)
png             = { version = "0.17", optional = true }
jpeg-decoder    = { version = "0.3", optional = true }
qoi             = { version = "0.4", optional = true }
resvg           = { version = "0.37", optional = true }
usvg            = { version = "0.37", optional = true }
tiny-skia       = { version = "0.11", optional = true }
kamadak-exif    = { version = "0.5", optional = true }

# Caching / data structures
lru             = { version = "0.12" }              # LruCache — no unsafe, pure Rust
ahash           = { version = "0.8" }               # AHashMap hasher (B2 — no hash collisions)
dashmap         = { version = "5" }                 # Per-PID budget DashMap (B6)

# Concurrency
crossbeam       = { version = "0.8", features = ["crossbeam-channel"] }
oneshot         = { version = "0.1" }               # reply channels in ImageReq

# GPU (conditional)
wgpu            = { version = "0.19", optional = true }

# Serialization (manifest parsing)
serde           = { version = "1", features = ["derive"] }
toml            = { version = "0.8" }
```

**Platform feature selection** (B8 — no build.rs, standard Cargo):

```toml
# .cargo/config.toml  (workspace root)
# Platform profiles set features via environment or workspace member overrides.
# mcu-minimal: no image features in default set
# iot-edge, robotics-rt: default (png, jpeg, bmp, qoi, exif); no svg, no gpu
# mobile: default + decode_svg + image_gpu
# desktop-full: default + decode_svg + image_gpu
# server-headless: default + decode_svg (no gpu)
```

Workspace `Cargo.toml` per-platform selections:
```toml
# In platform-specific workspace member or via build matrix:
# mobile / desktop-full:
[features]
default = ["decode_png","decode_jpeg","decode_bmp","decode_qoi","exif","decode_svg","image_gpu"]

# server-headless:
[features]
default = ["decode_png","decode_jpeg","decode_bmp","decode_qoi","exif","decode_svg"]

# mcu-minimal:
[features]
default = []
```

`mcu-minimal` compiles with no image crate features; all `decode_image` calls return `ImageError::FormatNotEnabled`. The `ImageConfig` for mcu-minimal uses `decode_worker_threads: 0`, so the worker pool is not spawned.

---

## §18 Implementation Files

All files under `supervisor/src/image/`. Each is ≤ 500 lines. The `mod.rs` re-exports public types only.

| File | Approximate LOC | Responsibility |
|---|---|---|
| `mod.rs` | 30 | Module re-exports; `pub use` for public API |
| `types.rs` | 120 | `ImageData`, `ImageFormat`, `PixelFormat`, `ResizeFilter`, `ImageCacheKey`, `IconSet`, `ImageError` |
| `config.rs` | 110 | `ImageConfig`, `for_platform()`, `check_dimensions()` |
| `detect.rs` | 60 | `detect_format()` magic-byte scanner |
| `decode.rs` | 80 | `decode_image()` dispatcher, `check_decoded_budget()`, `release_budget()` |
| `decode_png.rs` | 90 | `png_dimensions()`, `decode_png()`, `png_to_bgra32()` |
| `decode_jpeg.rs` | 130 | `jpeg_dimensions()`, `decode_jpeg()`, EXIF orientation transforms |
| `decode_bmp.rs` | 100 | `bmp_header_dimensions()`, `decode_bmp()`, BI_BITFIELDS rejection |
| `decode_qoi.rs` | 55 | `qoi_dimensions()`, `decode_qoi()`, `encode_qoi()` |
| `decode_svg.rs` | 90 | `decode_svg()`, `make_usvg_opts()`, `premul_rgba_to_straight_bgra()` |
| `resize.rs` | 160 | `resize()`, Nearest, Bilinear (B9 fix), Lanczos3 two-pass sRGB |
| `registry.rs` | 170 | `ImageRegistry` (slot store + LRU cache), `GpuImageCache` |
| `worker.rs` | 180 | `ImageWorker::spawn()`, `ImageReq`, worker loop, monitor/respawn (B5) |
| `icons.rs` | 150 | `load_icon_set()`, `icon_for_size()`, `SystemIconCache`, `IconCacheKey` |
| `capability.rs` | 70 | `ImagesCapability`, `validate_path()`, `from_toml()` |
| `surface_ext.rs` | 100 | `Surface::blit_rgba()`, `Surface::from_image_data()`, `Surface::snapshot_qoi()` |
| `gpu_upload.rs` | 40 | `upload_image_to_gpu()` wgpu integration |

**Total: 17 files, ~1535 LOC.** All within the 500-line limit. `gpu_upload.rs` is compiled only with `image_gpu` feature.

```rust
// supervisor/src/image/mod.rs
pub mod types;
pub mod config;
pub mod detect;
pub mod decode;
pub mod decode_png;
pub mod decode_jpeg;
pub mod decode_bmp;
pub mod decode_qoi;
pub mod decode_svg;
pub mod resize;
pub mod registry;
pub mod worker;
pub mod icons;
pub mod capability;
pub mod surface_ext;

#[cfg(feature = "image_gpu")]
pub mod gpu_upload;

// Convenience re-exports for supervisor modules
pub use types::{ImageData, ImageFormat, ImageError, PixelFormat, ResizeFilter, ImageCacheKey, IconSet};
pub use config::ImageConfig;
pub use registry::ImageRegistry;
pub use worker::{ImageWorker, ImageReq};
pub use icons::{SystemIconCache, IconCacheKey};
pub use capability::ImagesCapability;
```

---

## §19 Deferred

The following are explicitly out of scope for v1 and will be specified in future rounds:

**Animated formats (v2)**
- GIF: frame loop, disposal method, transparency index, inter-frame delta compositing
- APNG: `fcTL`/`fdAT` chunk parsing, blend operation, dispose operation
- WebP: both lossy and animated WebP via `libwebp` or `image-webp`
- Frame timing loop: `AnimatedImage { frames: Vec<(ImageData, Duration)> }` + a `FrameClock` driven by the compositor tick

**ICC Color Profile Pipeline (R15)**
- Full ICC v2/v4 profile parsing (owned by subsystem R15: Color Management)
- sRGB → display profile transforms using LittleCMS2 or a pure-Rust equivalent
- R14's Lanczos3 path currently converts to linear light for sRGB only; R15 will generalize

**Image Effects / Filter Graph (v3)**
- CoreImage-equivalent filter graph: blur (Gaussian, box), sharpen, color matrix, LUT, blend modes
- GPU-accelerated via R12 compute shaders
- Filter composition DSL (wgpu compute pipeline per filter node)

**Thumbnail Daemon (v2)**
- Background process that pre-rasterizes icons and document thumbnails
- Priority queue: visible icons first, off-screen second
- IPC protocol: `@thumbnail-daemon: preload <path> <size>`

**Image Metadata Query API**
- Arbitrary EXIF/XMP tag read beyond Orientation
- IPTC keyword extraction
- PNG tEXt/iTXt chunk reader
- `query_metadata(handle, tags: &[TagId]) -> Vec<MetadataValue>`

**HEIF / AVIF**
- HEIF: `libheif` Rust bindings; AVIF: `dav1d` via `rav1d`
- Both deferred pending musl static linking validation on all 6 platforms
