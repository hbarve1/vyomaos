# Round 14 Critic: Image & Icon Pipeline

## 1. Verdict

**ACCEPT WITH MANDATORY CHANGES**

The architect produced a competent first draft: clean module decomposition, correct
`catch_unwind` placement, reasonable WIT surface, and sound two-tier cache design.
However ten blocking defects must be resolved before this design is safe to ship.

**Blocking issues summary:**
- `usvg::Options::default()` permits external resource loading — critical sandbox escape
- `ImageCacheKey.path_hash: u64` — collision is a silent security bug
- BMP decoder omits BI_RLE4/BI_RLE8/BI_BITFIELDS — misrenders or panics on valid input
- EXIF orientation never applied — all camera JPEGs display sideways
- Single decode worker serializes all apps at startup — startup stall guaranteed
- `check_dimensions` fires AFTER full decode — alloc bomb before the guard
- `decode-image` double-copies image bytes through WASM linear memory
- No source crop in `blit-image` — sprite atlases are impossible
- `build.rs` uses `rustc-cfg` to activate Cargo features — dependencies not compiled
- `resize_bilinear` loop iterates `0..3`, never writes alpha — all output transparent

---

## 2. Blocking Issues (Critical or High Severity)

### 2.1 SVG External Resource Injection — Critical

`decode_svg_inner` constructs `usvg::Options::default()` and passes it to
`usvg::Tree::from_str`. The default options enable external resource resolution.
A malicious SVG with `<image xlink:href="/etc/passwd"/>` or
`@font-face { src: url('/usr/share/fonts/...') }` causes resvg to open arbitrary
host paths. resvg runs in the supervisor process; the WASM sandbox is bypassed.

Required fix — construct Options explicitly:

```rust
let opt = usvg::Options {
    resources_dir: None,
    font_resolver: usvg::FontResolver {
        select_font: Box::new(|_, _| None),
        select_fallback: Box::new(|_, _, _| None),
    },
    image_href_resolver: usvg::ImageHrefResolver {
        resolve_data:   Box::new(|_, _, _| None),
        resolve_string: Box::new(|_, _| None),
    },
    ..Default::default()
};
```

A unit test must prove a crafted SVG with `xlink:href` returns `DecodeFailed`,
not a successful decode that smuggled file bytes into pixel output.

### 2.2 Path Hash Collision — Silent Security Bug — Critical

`ImageCacheKey.path_hash: u64` uses `DefaultHasher`, which is neither stable
nor collision-resistant. Two distinct paths colliding on their u64 hash return
the wrong image silently. A malicious app can craft a path whose hash matches
the system trash icon, injecting spoofed pixel data visible to all apps. With
a shared `SystemIconCache` across 50+ apps, the birthday bound for 64-bit hashes
is reached well below the full icon-path space.

Replace with a full path key:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ImageCacheKey {
    pub canonical_path: String,  // full path, no truncation
    pub width:  u32,
    pub height: u32,
    pub format: PixelFormat,
}
```

Remove `hash_path` from `cache.rs` entirely. If key size is a concern, use
`SipHasher13` with a per-boot random seed stored in `ImageConfig`.

### 2.3 BMP Decoder Accepts Corrupt Inputs and Misrenders Valid Ones — High

`decode_bmp_inner` rejects compression != 0 and != 3, and bit_count != 24/32.
When compression == 3 (BI_BITFIELDS), the decoder reads pixel data as BI_RGB,
ignoring the mask fields at offsets 40–52. Any 16-bit BMP or 32-bit BMP with
non-BGR channel ordering is silently misrendered. Valid 8-bit palette BMPs
from legacy tooling silently error. The "~200 LOC" claim is dangerously optimistic:
correct BI_BITFIELDS is ~80 lines; BI_RLE8 with bounds-checked output cursor is
~150 lines. Either replace with the `bmp` crate or explicitly reject all
non-BI_RGB-24/32 variants and return `UnsupportedFormat`, not a partial decode.

### 2.4 `check_dimensions` Fires After Full Decode — High

Section 20 claims "Dimension gate rejects dimensions before passing to any decoder."
The actual `load-image` handler in `wit_handlers.rs` calls `worker.decode_path`
then `check_dimensions(img.width, img.height)`. A crafted JPEG with a tiny
compressed size but header-declared dimensions of 8192×8192 allocates 256 MB
before the guard fires. The fix: thread `max_image_dimension` into each decoder
and reject at header-parse time:

```rust
// In decode_png_inner, before allocating output buffer:
let info = decoder.read_info()?;
config.check_dimensions(info.info().width, info.info().height)?;
let mut buf = vec![0u8; reader.output_buffer_size()];
```

For `jpeg-decoder`, call `read_info()` to get metadata before `decode()`.

### 2.5 Single Decode Worker Serializes All Apps at Startup — High

`CHANNEL_CAPACITY = 32` and one worker thread processes all apps' decodes
sequentially. VyomaOS boots 10 apps. If each loads 2 images, 20 requests queue
immediately. Each JPEG decode takes ~30 ms; the 20th request waits 570 ms before
starting, exceeding `RECV_TIMEOUT_MS = 500`. Apps 11–20 receive `WorkerTimeout`
on startup. The spec must spawn a pool of N workers (N = platform CPU count, max 4)
with round-robin dispatch from `ImageWorkerHandle`. The R13 FontWorker pattern
scales directly; the monitor thread wraps the entire pool.

### 2.6 `decode-image` Double-Copy Through WASM Memory — High

`decode-image(data: list<u8>)` copies data from WASM linear memory into a `Vec<u8>`
then sends it through the channel. For a 4 MB PNG decoding to a 16 MP image,
peak in-flight is 4 MB encoded + 64 MB decoded = 68 MB, held simultaneously in
supervisor heap. On iot-edge (4 MiB decode budget) the `data.len() * 6` check
rejects any image above ~680 KB — nearly all photos. Either change iot-edge's
budget check multiplier to a format-specific expansion constant, or define a
shared-memory transfer for large images that avoids the full double-copy.

The WASI Preview 2 component model provides a `shared-memory` proposal for zero-
copy buffer passing. Even without that, the supervisor can map an anonymous page,
hand a file descriptor to the WASM app via a future `image-buffer` WIT call, and
decode directly from that page. This path is worth specifying even if deferred
to v1.1, because adding shared-memory decode after apps are shipped breaks ABI.

### 2.7 No Source Crop Rectangle in `blit-image` — High

`blit-image(handle, dst-x, dst-y, dst-w, dst-h)` has no `src-x, src-y, src-w,
src-h` parameters. Sprite atlases, multi-frame animations packed in one PNG,
and CSS-style background positioning are all impossible without a source rect.
This is an ABI-breaking omission — adding it after apps are compiled against
the WIT breaks them. Add before any bindings are generated:

```wit
blit-image: func(
    handle: u32,
    src-x: u32, src-y: u32, src-w: u32, src-h: u32,
    dst-x: i32, dst-y: i32, dst-w: u32, dst-h: u32,
) -> result<_, string>;
```

When `src-w` and `src-h` are 0, treat as full image extent.

### 2.8 `build.rs` Cannot Activate Cargo Features via `rustc-cfg` — High

The `build.rs` fragment emits `cargo:rustc-cfg=feature="image-raster"`. Cargo
`[features]` entries and `optional = true` dependencies are not enabled by
`rustc-cfg`. The SVG and image decoder dependencies will not be compiled on any
platform. The correct mechanism is workspace-level feature selection or emitting
`cargo:rustc-env=CARGO_FEATURE_IMAGE_RASTER=1` combined with a custom build
resolver. As written, `decode_png.rs`, `decode_jpeg.rs`, and all decoder crates
are dead code on every platform profile.

### 2.9 `resize_bilinear` Never Writes Alpha Channel — High

The inner loop is `for ch in 0..3usize`. The branch `if ch == 3` is unreachable.
`out[di+3]` stays at 0 (zero-initialized vec). All bilinearly-resized images are
fully transparent. The fix is `for ch in 0..4usize` — channel 3 falls into the
non-gamma path and is written with `(v * 255.0 + 0.5) as u8`. This is a logic
bug in the reference code that implementers will copy verbatim.

### 2.10 EXIF Orientation Never Applied — High

`jpeg-decoder` returns raw pixels without applying EXIF Orientation. Camera JPEGs
universally store portrait shots with Orientation 6 (90° CW) or 8 (90° CCW).
Without rotation, every user photo displays sideways. After `decoder.decode()`,
call `decoder.exif_data()`, parse tag 0x0112 in IFD0 using the `exif` crate,
and apply an in-place `rotate_90`/`rotate_270` (~30 lines). On server-headless
where display is disabled this can be skipped via a config flag. This is
non-negotiable for any platform displaying user-generated photos.

---

## 3. Non-Blocking Concerns

### 3.1 Animated Formats Entirely Absent
GIF, APNG, and animated WebP are not mentioned. Apps receive first frame silently.
The spec must state "animated formats: first frame only, v1.0 limitation" and
reserve WIT semantics for multi-frame handles so v1.1 is not ABI-breaking.

### 3.2 ICC Color Profiles Silently Discarded
PNG `iCCP` chunks and JPEG APP2 ICC markers are discarded. Images in AdobeRGB or
P3 display with wrong colors. Either convert to sRGB at decode time or document
"all images assumed sRGB" as a stated constraint.

### 3.3 `validate_path` Fails on Non-Existent Files
`canonicalize()` returns `Err` for paths that do not exist yet. This maps to
`PathTraversal` error instead of `NotFound`. Pre-check the root, then verify
the requested path is beneath it via a string prefix check before canonicalizing.

### 3.4 `VYOMA_DRAW_V2:set_icon` Path Not Validated
`DrawCmdV2::SetIcon { path }` stores `args.to_string()` directly. No code path
through `validate_path` is shown before `load_icon_set` is called. An app with
`display = true` but no `[capabilities.images]` can issue `set_icon:../../../../etc/`
and trigger a filesystem scan outside permitted paths.

### 3.5 `WorkerPanic` Loses In-Flight Request Without Retry
Apps receive `ImageError::WorkerPanic` on respawn but have no automatic retry
contract. The spec must require either one automatic retry from the handle, or
document that callers are responsible for retry, with a code example.

### 3.6 `blit_rgba` O(w*h) Per-Pixel Branch Defeats Vectorization
The per-pixel `if sa == 0 { continue }` and `if sa == 255` branches defeat SIMD
auto-vectorization. For a 960×700 full-window blit, this is 672,000 branched
iterations on the compositor hot path. Add a fast-path: when the entire source
image has alpha == 255 (detectable at decode time as a flag on `ImageData`), use
row-wise `copy_from_slice` instead of the blend loop.

### 3.7 `encode_qoi` Panics in Compositor on Non-Bgra32 Input
`assert_eq!(img.format, PixelFormat::Bgra32)` hard-panics in the compositor flush
path. Any code that passes a non-Bgra32 `ImageData` to `encode_qoi` crashes the
compositor. Replace with a `Result<Vec<u8>, ImageError>` return.

### 3.8 `SystemIconCache` Single Mutex Under 50 Apps
All apps share one `Mutex<IconCacheInner>`. Under 50 concurrent apps each calling
`icon-for-size` on repaint, this serializes all icon lookups. Use `DashMap` or
shard by `(path_hash % N)` into N sub-locks.

### 3.9 `next_handle: u32` Will Overflow
A video-frame app loading/unloading at 30 fps overflows `u32::MAX` in ~41 hours.
Overflow wraps to 0, aliasing live handles. Either use u64 or maintain a free-list
of recycled handles and return `ImageError::InvalidHandle` when exhausted.

### 3.10 `blit-icon` WIT Call Missing from `wit_handlers.rs`
`icons::blit-icon` is defined in the WIT but not registered in
`register_image_hostcalls`. Apps calling `blit-icon` receive a Wasmtime linker
error at instantiation, not a runtime error. The missing registration must be added.

### 3.11 `VYOMA_DRAW_V2` Reply Deadlock Risk
`load_image` sends a reply to the app's stdin. If the app has filled its stdin
buffer while blocking for the reply (because it issued more draw commands), both
sides deadlock: supervisor's stdin writer blocks on full pipe; app's stdout reader
blocks waiting for the reply. The spec must mandate that apps drain stdin before
issuing more draw commands after `load_image`, or use a separate reply fd.

### 3.12 `decode_png_inner` Does Not Handle 16-Bit PNG
The `png` crate returns raw 16-bit samples for 16-bit PNGs. The color-type match
arms do not handle this; treating 16-bit samples as 8-bit BGRA produces garbage
without an error. Either call `decoder.set_transformations(Transformations::SIXTEEN_BIT)`
to force 8-bit output, or explicitly detect and reject 16-bit depth before decode.

### 3.13 `load-image` Skips Per-App Format Allowlist Check
The `load-image` handler calls `worker.decode_path` without checking
`s.capability.format_allowed(fmt, &allowed_formats)`. The worker only checks
`config.formats_enabled` (platform-level). An app without `"svg"` in its manifest
can load SVGs via `load-image` if format is auto-detected from magic bytes.

### 3.14 Per-PID Budget Not Decremented on LRU Cache Eviction
`bytes_used` is incremented on `alloc_handle` and decremented on `unload-image`.
When the CPU cache evicts an entry, `bytes_used` is not reduced. An app that loads
100 images will hit budget exhaustion even after 80 are evicted, preventing new
loads despite the memory being freed.

### 3.15 Lanczos3 Intermediate Buffer Memory Spike
`resize_lanczos3` allocates `vec![[0f32; 4]; inter_w * inter_h]` for the horizontal
pass. Resizing a 4096×4096 image to 2048×2048 requires a 256 MB intermediate
buffer before any output is written. On mobile (256 MB total RAM) this triggers
OOM. Lanczos3 must be capped to desktop-full and server-headless, or implemented
with a row-at-a-time streaming pass.

### 3.16 GPU Cache Eviction Has No Ownership Path
`GpuImageCache::evict_lru` returns `Vec<TextureHandle>` that the caller must
pass to R12's `destroy_texture`. The spec does not define who calls `evict_lru`,
when, or how returned handles are sent to R12. This is a dangling GPU resource
path with no ownership assignment.

### 3.17 `ImageFormat::from_magic` Misses BOM-prefixed SVG
SVGs with a UTF-8 BOM (`\xEF\xBB\xBF<?xml`) or leading whitespace fail magic
detection. The format hint is then required for these files, which is undocumented.
Strip leading whitespace and BOM before the magic-byte scan.

### 3.18 `R25` Thumbnail Snapshot on Every Flush Is 1.6 GB/s
`Surface::snapshot()` calls `raw_bgra().to_vec()` — a full memcopy — on every
visible window every compositor flush, even when Mission Control is not active.
For 10 windows at 960×700 BGRA32 at 60 Hz: 10 * 960 * 700 * 4 * 60 = 1.6 GB/s
of unnecessary copies. Gate snapshots behind a `mission_control_active` boolean.

### 3.19 `parse_icon_filename` Scale Fallback Maps `@1x` to Scale 3
`if s[idx+1..].starts_with('2') { 2u8 } else { 3u8 }` maps `@1x`, `@3x`, `@4x`
all to scale 3. Replace with explicit digit parsing:
`s[idx+1..].chars().next().and_then(|c| c.to_digit(10)).map(|d| d as u8).unwrap_or(1)`.

### 3.20 Icon Bundle Directory Convention Undefined for App Authors
`load_icon_set` accepts `16.png`, `32@2x.png`, `icon_256.png`. The strip of
`icon_` means both `icon_16.png` and `16.png` map to size 16. Without a canonical
spec in the manifest schema, every app author invents their own layout and half
produce empty icon sets silently.

### 3.21 `check_budget` Multiplier 6x Rejects Valid JPEG on iot-edge
`data.len() * 6` as worst-case expansion rejects any JPEG above ~680 KB on
iot-edge (4 MiB budget). A 2 MP phone photo JPEG is typically 2–4 MB; actual
decoded size is `2_000_000 * 4 = 8 MB`, not `data_len * 6`. Use format-specific
expansion constants: JPEG ~4x, PNG ~8x, QOI ~4x, BMP ~1.1x.

### 3.22 WIT `world vyoma-app` Conflicts with R12
R12 defines `world vyoma-app` in `vyoma:gpu`. Two packages cannot both define
a world with the same name in the WIT component model. Rename to
`world vyoma-image-app` or compose both worlds at the top-level package.

### 3.23 `Surface::snapshot` Sets `source_format: ImageFormat::Qoi` Incorrectly
`snapshot()` returns raw BGRA pixels tagged as `source_format: ImageFormat::Qoi`.
This corrupts any code-path that inspects `source_format` for format detection
or cache key construction. Tag as a new variant or use a dedicated `SnapshotRaw`
variant.

### 3.24 No WebP Support
WebP is the default camera format on iOS 14+ and dominant on the web. Omitting
it forces apps to transcode before load. The `webp` crate (pure Rust) could be
added as `optional = true` gated behind `image-raster`.

### 3.25 `format_hint: u8` Magic Numbers in WIT
`decode-image` accepts `format-hint: u8` with values 0–5 defined only in Rust,
not in the WIT file. WIT supports `enum`. Define:
`enum image-format { auto-detect, png, jpeg, bmp, qoi, svg }` for type safety
and generated bindings.

### 3.26 No File-Mtime Invalidation on Re-Load
`ImageCache::invalidate_path` is never called on re-load. Stale cached images
persist after on-disk changes. `load-image` must stat the file and compare mtime
against a stored value, or skip the cache for files in writable directories.

### 3.27 `alloc_handle` Does Not Detect Exhaustion Before Overflow
`self.next_handle` increments without checking for `u32::MAX`. The check must
be added before increment and return `ImageError::InvalidHandle` when exhausted.

### 3.28 `decode_bmp_inner` Index Arithmetic Panic
`let row = &data[row_start .. row_start + width as usize * bpp]` panics if
`data.len() < row_start + width * bpp`. The `needed` pre-check uses
`src_stride * height` but the row slice uses `width * bpp` (no padding). A BMP
where `width * bpp < src_stride` and the file is truncated at the padding bytes
passes the `needed` pre-check but panics at the row slice. The fix: derive row
slice length from `src_stride`, not `width * bpp`, and verify the slice fits:
```rust
let row_end = row_start + src_stride;
if row_end > data.len() { return Err(ImageError::DecodeFailed("truncated row".into())); }
let row = &data[row_start .. row_start + width as usize * bpp];
```

### 3.29 `icon-for-size` Returns Unresized Larger Variant Without Warning
When exact size is absent, `icon_for_size` returns the next larger variant
unmodified. The compositor stretches it with whatever filter is active (possibly
Nearest), not Lanczos3. Apps expecting a 16px icon receive a 32px image scaled
down by the display layer — correct in size but wrong in quality. The spec must
require that size synthesis via Lanczos3 occurs at `icon_for_size` time for
non-standard sizes, or document the quality trade-off explicitly.

### 3.30 `AppImageState.icon_sets` Shares Handle Namespace with `handles`
`alloc_icon_set` uses `self.next_handle` and increments it. An icon set handle
and an image handle can have the same numeric value if allocated interleaved.
Passing an icon set handle to `unload-image` silently removes the image handle
with the same number, or vice versa. Use separate `next_image_handle` and
`next_icon_handle` counters, or separate namespaces (even-odd, or high-bit flag).
The simplest safe fix: set bit 31 for icon set handles
(`h | 0x8000_0000`) so the two namespaces are disjoint and type-checked by value.

### 3.31 No Test Coverage for `validate_path` Symlink Chains
`validate_path` calls `canonicalize()` which resolves symlink chains. But the test
suite in `manifest.rs` only tests the positive case (valid relative path). There
are no tests for: symlink pointing outside app root, double-dot traversal after
canonicalize, symlink target that does not exist (returns PathTraversal, not
NotFound), or absolute path to `/proc/self/mem`. Security-critical path validation
requires adversarial unit tests, not just happy-path ones.

### 3.32 `worker.rs` Monitor Thread Holds `rx` While Waiting for Worker to Die
The monitor thread receives from the public `rx` and forwards to the internal `wtx`.
When the worker panics, `wtx.send(req)` returns `Err`. The monitor breaks, sends
`Shutdown` to the dead worker channel (no-op), and calls `worker_thread.join()`.
During `join()`, any new requests arriving on `rx` are not processed — they sit
in the channel. The `CHANNEL_CAPACITY = 32` means up to 32 requests accumulate.
Requests that time out during the join-and-respawn window receive `WorkerTimeout`
even though the worker was respawning, not genuinely stuck. Document the respawn
latency and increase `RECV_TIMEOUT_MS` to at least 2000 ms, or use a non-blocking
probe of worker liveness before starting the forwarding loop.

---

## 4. What the Architect Got Right

1. Module decomposition: one file per decoder, separate cache/worker/icon/manifest
   and WIT handler modules, all within the 500-line file budget.
2. `catch_unwind(AssertUnwindSafe(...))` correctly applied around every decoder
   call, resize, and worker dispatch. Watchdog respawn mirrors R13 FontWorker.
3. `ImageConfig::for_platform` provides a single authoritative capability matrix;
   no scattered platform conditionals across decoder files.
4. Two-tier cache (CPU LRU + GPU texture LRU) is the correct architecture:
   decoded pixels and texture handles have independent lifetimes and eviction.
5. `PixelFormat` enum correctly covers all three native formats (Bgra32, Rgb565,
   Mono1bpp) with a correct `bytes_per_pixel` returning `None` for Mono1bpp.
6. `IconSet::icon_for_size` four-level fallback cascade degrades gracefully
   across scale factors and standard sizes without panicking.
7. sRGB-correct bilinear and Lanczos3 resize via 256-entry LUT is the right
   approach; most image pipelines skip gamma and produce muddy downscales.
8. Two-pass separable Lanczos3 (horizontal then vertical in linear light) is
   algorithmically correct and the standard efficient implementation.
9. `ImagesCapability::validate_path` using `canonicalize()` on both the root and
   the requested path correctly blocks symlink traversal attacks.
10. `ImageFormat::from_magic` magic-byte detection as the primary path is correct;
    format hints are fallbacks, not requirements. The 12-byte minimum is sound.
11. QOI for icon/screenshot lossless storage is a good choice: pure Rust, ~2x
    faster encode/decode than PNG, deterministic output for caching.
12. Feature-gating the entire image pipeline out on `mcu-minimal` is correct;
    the platform cannot afford code size or heap for image decode.
13. `ImageError` enum is comprehensive: all failure modes including handle lifetime,
    worker state, GPU, and path traversal errors are covered.
14. `AppImageState` per-app with its own `bytes_used` counter and handle table
    provides correct per-app budget isolation even with a shared global worker.
15. The `ImageReq::Resize` variant in the worker protocol correctly separates resize
    from decode: callers can request a resize of an already-decoded `ImageData`
    without re-decoding the source file, which is critical for icon size synthesis.
16. Gating `gpu_upload_enabled` separately from `icon_system_enabled` in `ImageConfig`
    correctly reflects that iot-edge and robotics-rt need icons but lack a GPU path.

---

## 5. Questions for Synthesis

1. Should SVG be deferred entirely from v1.0 given the binary-size impact (+2 MB)
   and security surface complexity? What scalable icon format replaces it?
2. Replace BMP with the `bmp` crate, or restrict to BI_RGB 24/32 bpp with an
   explicit `UnsupportedFormat` for other variants?
3. Should animated GIF/APNG be in scope? If not, must the WIT reserve multi-frame
   handle semantics now to avoid an ABI break in v1.1?
4. EXIF orientation: apply in `decode_jpeg_inner` always, or as a manifest flag?
   Should rotation be in-place on pixels or stored as metadata on `ImageData`?
5. Worker pool size: hardcoded per platform in `ImageConfig`, or `min(num_cpus, 4)`
   at runtime? What is the right pool size for iot-edge (single-core ARM)?
6. Should `ImageCacheKey` use the full canonical path string or SipHasher13 with
   a per-boot random seed? The seed approach avoids unbounded key size while
   preventing collision attacks.
7. What is the one canonical icon bundle filename format? Should VyomaOS define
   a `.vyicon` bundle format (like `.icns`) or rely on a flat PNG directory?
8. Should `blit_rgba` get a SIMD fast-path for fully-opaque images (SSE2 row
   copy), given it is on the compositor hot path at 60 Hz?
9. How does `GpuImageCache` receive VRAM pressure signals from R12? Callback,
   polled query, or explicit eviction demand message?
10. Should the `VYOMA_DRAW_V2` stdout protocol be deprecated in favor of WIT-only,
    given the deadlock risk identified in concern 3.11?
11. Is `ImageData.stride` always `width * bytes_per_pixel`, or can it be larger
    for alignment? Callers that assume tight packing corrupt strided images silently.
12. How should `bytes_used` accounting interact with the LRU cache that also holds
    strong `Arc<ImageData>` references? Double-counting must be resolved.
13. What is the policy for an app with `images.decode` but `filesystem = false`:
    can it load images from its bundle root without the general filesystem cap?
14. Should Lanczos3 be capped to desktop-full/server-headless, or implemented
    as a tile-based streaming pass to fit within mobile's 256 MB RAM?
15. Should `image-size` return DPI/PPI from JFIF/pHYs chunks for print-aware apps,
    or are pixel dimensions sufficient for v1.0?
16. Should `validate_path` allow `/data` unconditionally for all apps with `images`
    capability, or must `/data` access also require `filesystem = true`?
17. Should the `ImageWorkerHandle` expose a `decode_path_cached` convenience method
    that checks the CPU LRU cache before dispatching to the worker, or should all
    cache lookups remain in the WIT handler layer to keep the worker stateless?
18. How should the supervisor signal `WorkerPanic` to all apps that have in-flight
    requests when the worker respawns? Should in-flight reply channels receive an
    explicit cancellation, or is timeout the only signal apps can rely on?
19. Should `ImageData` carry a `content_hash: Option<[u8; 32]>` field (SHA-256 of
    the source bytes) to enable content-addressed caching independent of file path?
    This would allow two apps loading the same image from different bundle paths
    to share a single cache entry, halving memory for common system images.

---

## 6. Closing

The v1.1 redesign checklist: replace `usvg::Options::default()` with fully
locked-down options and add a unit test that proves a crafted `xlink:href` SVG
returns `DecodeFailed` with no filesystem side effect; replace the u64 path hash
key with a full canonical string or seeded SipHash and remove `hash_path` from
`cache.rs`; restrict the BMP decoder to BI_RGB 24/32 bpp with explicit
`UnsupportedFormat` returns for all other compression modes, and add slice-bounds
guards in the row loop; move `check_dimensions` into each decoder at header-parse
time before any pixel buffer allocation; add EXIF orientation correction after
JPEG decode using the `exif` crate, controlled by a per-platform flag in
`ImageConfig`; replace the single decode worker with a pool of N workers sized
by `ImageConfig.decode_worker_threads` (set per platform, typically 2–4); add
`src-x, src-y, src-w, src-h` to the `blit-image` WIT signature before any app
bindings are generated — this is an ABI decision, not an optimization; fix
`build.rs` to use workspace-level Cargo feature selection rather than `rustc-cfg`,
and verify that `decode_png.rs` actually compiles on iot-edge by running
`cargo build --features image-raster` in CI; change the `resize_bilinear` inner
loop from `for ch in 0..3` to `for ch in 0..4` and add a test that resizes a
50% transparent image and asserts alpha is preserved; gate `Surface::snapshot()`
in the R25 thumbnail pipeline behind a `mission_control_active: AtomicBool` flag
checked at the top of the compositor flush pass; and define one canonical icon
bundle filename convention (`<size>.png` and `<size>@<scale>x.png` only, with
`icon_` prefix explicitly disallowed) in the manifest schema documentation.
All twelve items above are mandatory before any implementation PR is opened.
The 32 non-blocking concerns in Section 3 must each receive an explicit
accept/defer/reject call in the synthesis document; any deferred item that touches
WIT signatures must be resolved before bindings are generated to avoid ABI churn.

<!-- end of Round 14 Critic -->
