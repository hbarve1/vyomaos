# FINAL Spec: Image Processing Pipeline (Round 71)

**macOS Analogue**: CoreImage / vImage / Accelerate  
**Status**: APPROVED for implementation  
**Target Phase**: P19  
**Blocking Issues**: B1–B5 (see §8)

---

## 1. Architecture

The Image Processing Pipeline is a supervisor-side subsystem that processes WASM app image buffers through a composable filter chain. No new daemon threads; filter execution runs on per-app reader threads with optional short-lived spawns (B4).

### Module Structure

```
supervisor/src/
└── imgproc/
    ├── mod.rs          (≤120 lines) — ImageProcState, protocol dispatch, lifecycle
    ├── shm.rs          (≤180 lines) — ImageShm memfd/mmap allocation & buffer lifecycle
    ├── filter.rs       (≤220 lines) — FilterOp enum, FilterChain parsing & validation
    ├── ops_color.rs    (≤200 lines) — brightness, contrast, saturation, colorspace conversions
    ├── ops_resize.rs   (≤160 lines) — bilinear resize for RGBA8 and Greyscale8
    ├── ops_blur.rs     (≤160 lines) — separable Gaussian blur (3×3 through 15×15)
    └── ops_composite.rs (≤100 lines) — alpha blend modes (src-over, dst-over, multiply)
```

### Thread Model

- **Per-app reader thread** executes filter chains inline (no new threads for most operations)
- **Long operations** (e.g., large Gaussian blur, resize) spawn short-lived worker threads via rayon (B4)
- **No polling loops**; callbacks deliver `VYOMA_IMAGE:done|<req_id>|<w>|<h>` to app stdin
- **Memory**: Shared buffers use memfd (inheritable FDs, no MFD_CLOEXEC) for zero-copy WASM ↔ supervisor

---

## 2. VYOMA_IMAGE Protocol

Apps communicate image processing requests via stdout. Supervisor parses, executes, responds via stdin.

### Protocol Commands

```
VYOMA_IMAGE:alloc|<width>|<height>|<format>
  → VYOMA_IMAGE:allocated|<fd_path>|<size_bytes>

VYOMA_IMAGE:submit|<req_id>|<src_w>|<src_h>|<dst_w>|<dst_h>|<ops>
  → VYOMA_IMAGE:done|<req_id>|<dst_w>|<dst_h>

VYOMA_IMAGE:free
  → (no response)
```

### Filter Chain Syntax

`<ops>` is semicolon-separated list of filter operations:
- `resize:bilinear` — bilinear resize to (dst_w, dst_h)
- `blur:<radius>` — Gaussian blur, radius 1–7 (3×3 through 15×15 kernel)
- `brightness:<delta>` — brightness shift, delta ∈ [-255, 255]
- `contrast:<factor>` — contrast multiplier, factor ∈ [0.5, 3.0]
- `saturation:<factor>` — saturation multiplier, factor ∈ [0.0, 2.0]
- `colorspace:<src>:<dst>` — convert between formats (e.g., `rgba8:grey8`, `rgba8:yuv420`)
- `composite:<mode>:<alpha>` — composite with existing buffer, mode ∈ {src-over, dst-over, multiply}, alpha ∈ [0, 255]

Example:
```
VYOMA_IMAGE:submit|req42|640|480|1280|960|resize:bilinear;blur:2;brightness:30;saturation:1.2
```

### Pixel Formats

- `rgba8` — 4 bytes/pixel, little-endian: [R, G, B, A]
- `bgra8` — 4 bytes/pixel, little-endian: [B, G, R, A]
- `grey8` — 1 byte/pixel, greyscale
- `yuv420` — planar YUV, Y full resolution, U/V half resolution (4:2:0)

---

## 3. Core Data Types

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Rgba8,
    Bgra8,
    Grey8,
    Yuv420,
}

impl PixelFormat {
    /// Total bytes needed for frame of given dimensions.
    pub fn frame_bytes(self, w: u32, h: u32) -> usize {
        match self {
            PixelFormat::Rgba8 | PixelFormat::Bgra8 => (w * h * 4) as usize,
            PixelFormat::Grey8 => (w * h) as usize,
            PixelFormat::Yuv420 => ((w * h * 3) / 2) as usize, // Y full + U/V half
        }
    }
}

#[derive(Debug, Clone)]
pub enum FilterOp {
    Resize { mode: ResizeMode },
    Blur { radius: u8 },            // 1–7
    Brightness { delta: i32 },      // [-255, 255]
    Contrast { factor: f32 },       // [0.5, 3.0]
    Saturation { factor: f32 },     // [0.0, 2.0]
    ColorSpace { src: PixelFormat, dst: PixelFormat },
    Composite { mode: BlendMode, alpha: u8 }, // [0, 255]
}

#[derive(Debug, Clone)]
pub enum ResizeMode {
    Bilinear,
}

#[derive(Debug, Clone, Copy)]
pub enum BlendMode {
    SrcOver,
    DstOver,
    Multiply,
}

#[derive(Debug, Clone)]
pub struct FilterChain(pub Vec<FilterOp>);

impl FilterChain {
    pub fn parse(s: &str) -> Result<Self, String> {
        // Parse semicolon-separated ops, validate ranges
    }
}

/// Per-app image processing state
pub struct AppImageState {
    pub shm: Option<ImageShm>,
    pub fmt: PixelFormat,
    pub alloc_w: u32,
    pub alloc_h: u32,
}

/// Shared memory buffer (memfd-backed)
pub struct ImageShm {
    pub fd: i32,
    pub ptr: *mut u8,
    pub total_bytes: usize,
    pub src_bytes: usize, // split boundary
}

impl ImageShm {
    pub unsafe fn new(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32, fmt: PixelFormat) -> io::Result<Self> {
        // alloc src + dst halves, memfd_create, mmap MAP_SHARED, return fd + ptr
    }
    
    pub unsafe fn src_slice(&self) -> &[u8] {
        std::slice::from_raw_parts(self.ptr, self.src_bytes)
    }
    
    pub unsafe fn src_mut(&mut self) -> &mut [u8] {
        std::slice::from_raw_parts_mut(self.ptr, self.src_bytes)
    }
    
    pub unsafe fn dst_slice(&self) -> &[u8] {
        std::slice::from_raw_parts(self.ptr.add(self.src_bytes), self.total_bytes - self.src_bytes)
    }
    
    pub unsafe fn dst_mut(&mut self) -> &mut [u8] {
        std::slice::from_raw_parts_mut(self.ptr.add(self.src_bytes), self.total_bytes - self.src_bytes)
    }
}

impl Drop for ImageShm {
    fn drop(&mut self) {
        // munmap, close fd
    }
}
```

---

## 4. Key Algorithms

### 4a. Bilinear Resize (ops_resize.rs)

```rust
pub fn resize_rgba8_bilinear(src: &[u8], src_w: u32, src_h: u32, 
                              dst: &mut [u8], dst_w: u32, dst_h: u32) -> Result<(), String> {
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return Err("invalid dimensions".into());
    }
    
    let x_ratio: u32 = ((src_w - 1) << 16) / (dst_w - 1).max(1);
    let y_ratio: u32 = ((src_h - 1) << 16) / (dst_h - 1).max(1);
    
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let sx_fp = dx as u32 * x_ratio; // fixed-point
            let sy_fp = dy as u32 * y_ratio;
            
            let sx = (sx_fp >> 16) as usize;
            let sy = (sy_fp >> 16) as usize;
            let fx = (sx_fp & 0xFFFF) as f32 / 65536.0;
            let fy = (sy_fp & 0xFFFF) as f32 / 65536.0;
            
            // Clamp to valid indices
            let x0 = sx.min((src_w - 1) as usize);
            let x1 = (sx + 1).min((src_w - 1) as usize);
            let y0 = sy.min((src_h - 1) as usize);
            let y1 = (sy + 1).min((src_h - 1) as usize);
            
            // Sample 4 corners (RGBA8)
            let p00 = sample_rgba8(src, src_w, x0, y0);
            let p10 = sample_rgba8(src, src_w, x1, y0);
            let p01 = sample_rgba8(src, src_w, x0, y1);
            let p11 = sample_rgba8(src, src_w, x1, y1);
            
            // Bilinear interpolation
            let c0 = lerp_rgba8(p00, p10, fx);
            let c1 = lerp_rgba8(p01, p11, fx);
            let result = lerp_rgba8(c0, c1, fy);
            
            write_rgba8(dst, dst_w, dx as usize, dy as usize, result);
        }
    }
    Ok(())
}

fn sample_rgba8(buf: &[u8], w: u32, x: usize, y: usize) -> [u8; 4] {
    let idx = (y as u32 * w + x as u32) as usize * 4;
    [buf[idx], buf[idx+1], buf[idx+2], buf[idx+3]]
}

fn write_rgba8(buf: &mut [u8], w: u32, x: usize, y: usize, px: [u8; 4]) {
    let idx = (y as u32 * w + x as u32) as usize * 4;
    buf[idx..idx+4].copy_from_slice(&px);
}

fn lerp_rgba8(a: [u8; 4], b: [u8; 4], t: f32) -> [u8; 4] {
    [
        (a[0] as f32 * (1.0 - t) + b[0] as f32 * t).round() as u8,
        (a[1] as f32 * (1.0 - t) + b[1] as f32 * t).round() as u8,
        (a[2] as f32 * (1.0 - t) + b[2] as f32 * t).round() as u8,
        (a[3] as f32 * (1.0 - t) + b[3] as f32 * t).round() as u8,
    ]
}
```

### 4b. Separable Gaussian Blur (ops_blur.rs)

```rust
const KERNELS: &[&[f32]] = &[
    &[0.25, 0.5, 0.25],                        // radius 1 (3×3)
    &[0.06, 0.24, 0.4, 0.24, 0.06],            // radius 2 (5×5)
    &[0.02, 0.11, 0.25, 0.25, 0.25, 0.11, 0.02], // radius 3 (7×7)
    // ... up to radius 7 (15×15)
];

pub fn blur_rgba8_gaussian(src: &[u8], w: u32, h: u32, 
                            tmp: &mut [u8], dst: &mut [u8], 
                            radius: u8) -> Result<(), String> {
    if radius < 1 || radius > 7 {
        return Err("radius must be 1..=7".into());
    }
    
    let kernel = KERNELS[(radius - 1) as usize];
    let k_size = kernel.len();
    
    // Horizontal pass: src → tmp
    blur_pass_h(src, tmp, w, h, kernel, k_size);
    
    // Vertical pass: tmp → dst
    blur_pass_v(tmp, dst, w, h, kernel, k_size);
    
    Ok(())
}

fn blur_pass_h(src: &[u8], dst: &mut [u8], w: u32, h: u32, kernel: &[f32], k_size: usize) {
    let center = k_size / 2;
    for y in 0..h as usize {
        for x in 0..w as usize {
            let mut r = 0.0f32, g = 0.0f32, b = 0.0f32, a = 0.0f32;
            for i in 0..k_size {
                let px = (x as i32 + i as i32 - center as i32).clamp(0, w as i32 - 1) as usize;
                let src_idx = (y as u32 * w + px as u32) as usize * 4;
                r += src[src_idx] as f32 * kernel[i];
                g += src[src_idx + 1] as f32 * kernel[i];
                b += src[src_idx + 2] as f32 * kernel[i];
                a += src[src_idx + 3] as f32 * kernel[i];
            }
            let dst_idx = (y as u32 * w + x as u32) as usize * 4;
            dst[dst_idx] = r.round() as u8;
            dst[dst_idx + 1] = g.round() as u8;
            dst[dst_idx + 2] = b.round() as u8;
            dst[dst_idx + 3] = a.round() as u8;
        }
    }
}

fn blur_pass_v(src: &[u8], dst: &mut [u8], w: u32, h: u32, kernel: &[f32], k_size: usize) {
    // Similar to blur_pass_h but iterates vertically
}
```

### 4c. Brightness / Contrast / Saturation (ops_color.rs)

```rust
pub fn brightness_rgba8(buf: &mut [u8], delta: i32) -> Result<(), String> {
    if delta < -255 || delta > 255 {
        return Err("brightness delta must be in [-255, 255]".into());
    }
    
    for chunk in buf.chunks_exact_mut(4) {
        chunk[0] = (chunk[0] as i32 + delta).clamp(0, 255) as u8; // R
        chunk[1] = (chunk[1] as i32 + delta).clamp(0, 255) as u8; // G
        chunk[2] = (chunk[2] as i32 + delta).clamp(0, 255) as u8; // B
        // A unchanged
    }
    Ok(())
}

pub fn contrast_rgba8(buf: &mut [u8], factor: f32) -> Result<(), String> {
    if factor < 0.5 || factor > 3.0 {
        return Err("contrast factor must be in [0.5, 3.0]".into());
    }
    
    let center = 128.0f32;
    for chunk in buf.chunks_exact_mut(4) {
        chunk[0] = ((chunk[0] as f32 - center) * factor + center).clamp(0.0, 255.0) as u8;
        chunk[1] = ((chunk[1] as f32 - center) * factor + center).clamp(0.0, 255.0) as u8;
        chunk[2] = ((chunk[2] as f32 - center) * factor + center).clamp(0.0, 255.0) as u8;
    }
    Ok(())
}

pub fn saturation_rgba8(buf: &mut [u8], factor: f32) -> Result<(), String> {
    if factor < 0.0 || factor > 2.0 {
        return Err("saturation factor must be in [0.0, 2.0]".into());
    }
    
    for chunk in buf.chunks_exact_mut(4) {
        let r = chunk[0] as f32;
        let g = chunk[1] as f32;
        let b = chunk[2] as f32;
        
        // Convert to HSL, adjust saturation, convert back
        let grey = (r + g + b) / 3.0;
        chunk[0] = (grey + (r - grey) * factor).clamp(0.0, 255.0) as u8;
        chunk[1] = (grey + (g - grey) * factor).clamp(0.0, 255.0) as u8;
        chunk[2] = (grey + (b - grey) * factor).clamp(0.0, 255.0) as u8;
    }
    Ok(())
}
```

### 4d. Color Space Conversion (ops_color.rs)

```rust
// BT.601 luma coefficients
const LUMA_R: f32 = 0.299;
const LUMA_G: f32 = 0.587;
const LUMA_B: f32 = 0.114;

pub fn rgba8_to_grey8(src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    for (i, chunk) in src.chunks_exact(4).enumerate() {
        let grey = (chunk[0] as f32 * LUMA_R + chunk[1] as f32 * LUMA_G + chunk[2] as f32 * LUMA_B) as u8;
        dst[i] = grey;
    }
    Ok(())
}

pub fn rgba8_to_bgra8(src: &[u8], dst: &mut [u8]) -> Result<(), String> {
    for (i, chunk) in src.chunks_exact(4).enumerate() {
        let j = i * 4;
        dst[j] = chunk[2];     // B
        dst[j+1] = chunk[1];   // G
        dst[j+2] = chunk[0];   // R
        dst[j+3] = chunk[3];   // A
    }
    Ok(())
}

pub fn rgba8_to_yuv420(src: &[u8], src_w: u32, src_h: u32, dst: &mut [u8]) -> Result<(), String> {
    // YUV420: Y full resolution, U/V half resolution
    // Requires even dimensions (B5: round up at alloc time)
    let y_size = (src_w * src_h) as usize;
    let y_buf = &mut dst[..y_size];
    let uv_buf = &mut dst[y_size..];
    
    // Y plane
    for (i, chunk) in src.chunks_exact(4).enumerate() {
        let y = (chunk[0] as f32 * 0.299 + chunk[1] as f32 * 0.587 + chunk[2] as f32 * 0.114) as u8;
        y_buf[i] = y;
    }
    
    // U/V planes (half resolution)
    let uv_w = src_w / 2;
    for uv_y in 0..src_h / 2 {
        for uv_x in 0..uv_w {
            let px_base = ((uv_y * 2) * src_w + (uv_x * 2)) as usize;
            let r = src[px_base * 4] as f32;
            let g = src[px_base * 4 + 1] as f32;
            let b = src[px_base * 4 + 2] as f32;
            
            let u = ((b - (0.299*r + 0.587*g + 0.114*b)) / 1.772 + 128.0) as u8;
            let v = ((r - (0.299*r + 0.587*g + 0.114*b)) / 1.402 + 128.0) as u8;
            
            uv_buf[(uv_y * uv_w + uv_x) as usize] = u;
            uv_buf[((src_w * src_h / 4) + uv_y * uv_w + uv_x) as usize] = v;
        }
    }
    Ok(())
}
```

### 4e. Alpha Composite (ops_composite.rs)

Reuses `blend_over` / `pack` / `unpack` from `display/compositor.rs`:

```rust
pub fn composite_src_over(src: &[u8], dst: &mut [u8], alpha: u8) -> Result<(), String> {
    for (i, s_chunk) in src.chunks_exact(4).enumerate() {
        let j = i * 4;
        let src_a = (s_chunk[3] as u32 * alpha as u32) / 255;
        let dst_chunk = [dst[j], dst[j+1], dst[j+2], dst[j+3]];
        let result = blend_over(s_chunk, &dst_chunk, src_a as u8);
        dst[j..j+4].copy_from_slice(&result);
    }
    Ok(())
}

pub fn composite_dst_over(src: &[u8], dst: &mut [u8], alpha: u8) -> Result<(), String> {
    // dst-over: dst composited on top of src
    for (i, s_chunk) in src.chunks_exact(4).enumerate() {
        let j = i * 4;
        let dst_chunk = [dst[j], dst[j+1], dst[j+2], dst[j+3]];
        let dst_a = (dst_chunk[3] as u32 * alpha as u32) / 255;
        let result = blend_over(&dst_chunk, s_chunk, dst_a as u8);
        dst[j..j+4].copy_from_slice(&result);
    }
    Ok(())
}

pub fn composite_multiply(src: &[u8], dst: &mut [u8], alpha: u8) -> Result<(), String> {
    for (i, s_chunk) in src.chunks_exact(4).enumerate() {
        let j = i * 4;
        let dst_chunk = [dst[j], dst[j+1], dst[j+2], dst[j+3]];
        dst[j] = ((s_chunk[0] as u32 * dst_chunk[0] as u32) / 255) as u8;
        dst[j+1] = ((s_chunk[1] as u32 * dst_chunk[1] as u32) / 255) as u8;
        dst[j+2] = ((s_chunk[2] as u32 * dst_chunk[2] as u32) / 255) as u8;
        dst[j+3] = ((s_chunk[3] as u32 * alpha as u32) / 255) as u8;
    }
    Ok(())
}
```

### 4f. ImageShm Lifecycle (shm.rs)

```rust
impl ImageShm {
    pub unsafe fn new(src_w: u32, src_h: u32, dst_w: u32, dst_h: u32, fmt: PixelFormat) -> io::Result<Self> {
        // Calculate sizes (B5: round YUV420 up to even)
        let src_h_adj = if fmt == PixelFormat::Yuv420 { (src_h + 1) & !1 } else { src_h };
        let dst_h_adj = if fmt == PixelFormat::Yuv420 { (dst_h + 1) & !1 } else { dst_h };
        
        let src_bytes = fmt.frame_bytes(src_w, src_h_adj);
        let dst_bytes = fmt.frame_bytes(dst_w, dst_h_adj);
        let total_bytes = src_bytes + dst_bytes;
        
        // memfd_create (inheritable, no MFD_CLOEXEC — B3 requires pre-open /proc/self/fd)
        let fd = libc::syscall(libc::SYS_memfd_create, b"imgproc\0".as_ptr(), 0) as i32;
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        
        // ftruncate
        if libc::ftruncate(fd, total_bytes as libc::off_t) != 0 {
            libc::close(fd);
            return Err(io::Error::last_os_error());
        }
        
        // mmap MAP_SHARED
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            total_bytes,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        
        if ptr == libc::MAP_FAILED {
            libc::close(fd);
            return Err(io::Error::last_os_error());
        }
        
        Ok(ImageShm {
            fd,
            ptr: ptr as *mut u8,
            total_bytes,
            src_bytes,
        })
    }
}

impl Drop for ImageShm {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.total_bytes);
            libc::close(self.fd);
        }
    }
}
```

---

## 5. WASM Plugin Interface

**No nested WASM plugins in R71.** The `VYOMA_IMAGE:submit` protocol itself IS the plugin interface. Apps compose filter chains as strings; supervisor executes them natively.

Future phase (P20+) will support WASM-compiled filters, loaded at runtime and sandboxed.

---

## 6. Manifest Capability Extension

Add `image_processing` field to `Capabilities` struct in `manifest.rs`:

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub stdio: Option<bool>,
    pub filesystem: Option<bool>,
    pub network: Option<bool>,
    pub display: Option<bool>,
    pub shell: Option<bool>,
    pub mouse: Option<bool>,
    pub image_processing: Option<bool>,  // NEW (B1)
    // ... other existing fields
}
```

Apps declare `image_processing = true` in `vyoma.toml`:
```toml
[capabilities]
stdio = true
image_processing = true
```

---

## 7. Cargo Dependencies

**Zero new dependencies.** All algorithms use:
- `libc` (already present) for memfd_create, mmap, ftruncate
- `std::sync::mpsc` for request channels
- Existing `display/compositor.rs` for blend_over / pack / unpack

---

## 8. Blocking Issues (B1–B5)

| Issue | Description | Resolution |
|-------|-------------|-----------|
| **B1** | `deny_unknown_fields` on Capabilities blocks new fields | Add `image_processing: Option<bool>` before deploying VYOMA_IMAGE handler |
| **B2** | shm split boundary ambiguity | Allocate explicitly as `src_half + dst_half`; split at exactly `src_bytes` (not max(src, dst)) |
| **B3** | App cannot resolve memfd fd after inheritance | When `image_processing=true`, pre-open `/proc/self/fd` read-only on app WASI import |
| **B4** | Long filter chains block app reader thread | Spawn short-lived rayon thread for blur/resize >100×100; send done via stdin callback |
| **B5** | YUV420 requires even dimensions | Round up alloc to `(h + 1) & !1` at memfd creation; return error if app submits odd dimensions |

---

## 9. Testing Strategy

### Unit Tests (supervisor/tests/)
- ImageShm allocation, mmap success, split boundary correctness
- FilterChain parsing: valid ops, invalid ranges, malformed syntax
- Bilinear resize: upscale, downscale, same-size (identity)
- Gaussian blur: radius 1–7, corner pixels
- Color space conversions: RGBA↔Grey, RGBA↔BGRA, RGBA→YUV420
- Alpha composite: src-over, dst-over, multiply modes
- YUV420 even-dimension rounding

### Integration Test (smoke test in QEMU)
- App allocates buffer → supervisor returns fd + path
- App submits resize + blur chain → supervisor executes → app reads result
- App calls free → supervisor munmaps + closes fd

---

## 10. Performance Targets

- **Bilinear resize 640×480→1280×960**: <50 ms (single-threaded)
- **Gaussian blur (radius=3) 1280×960**: <30 ms (rayon spawned)
- **Brightness/contrast/saturation**: <10 ms (chunks_exact_mut LLVM vectorized)
- **Color space conversions**: <20 ms
- **Composite**: <15 ms

All benchmarks on x86_64 desktop target. Embedded targets (ARM64) expect 2–5× longer.

---

## 11. Future Extensions (P20+)

1. **WASM filter plugins**: Load .wasm filters at runtime, sandbox via wasmtime
2. **GPU acceleration**: OpenGL/Vulkan for large frames (>2K×2K)
3. **Streaming pipelines**: Incremental processing of video frames
4. **Histogram / lut operations**: Adaptive histogram equalization
5. **Affine transform**: Rotation, perspective, shear
6. **Morphological ops**: Dilate, erode, open, close

---

**Approval Status**: ✅ Ready for Phase 19 implementation (Q2 2026)
