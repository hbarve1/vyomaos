# Round 15 Critic: Color Management & ICC Profiles

## 1. Verdict

**ACCEPT WITH MANDATORY CHANGES**

The architect produced a coherent first draft. The qcms-over-lcms2 choice is
the single best decision in the spec — it eliminates the C UB / SIGSEGV class of
security bugs that have historically plagued ICC implementations. The LRU registry,
per-app accounting, path validation, and platform matrix are all structurally
sound.

However eight blocking defects must be resolved before this design is safe to
ship, and several non-blocking gaps would leave macOS-level color fidelity out of
reach.

**Blocking issues summary:**

- `catch_unwind` does not protect against qcms integer overflow → OOM in release
- Transform handle encoding packs (from, to, intent) into a u32 with collisions
- `apply-transform` double-copies 8 MB of pixels across the WASM boundary per frame
- Surface compositor blit allocates a `converted_surface` clone every frame
- `ImageData.color_space` added as a plain field to an existing struct without
  migration of existing decode callsites — all callers silently get `Srgb` default
- `detect_gamut_from_edid` reads one chromaticity coordinate; false-positive rate
  is high on cheap sRGB monitors with slightly shifted primaries
- qcms `Transform::new_to` wraps the result in a `Box<Transform>`; leaking it via
  `Box::into_raw` then casting to `*mut c_void` causes a double-free on drop
- `host_create_transform` ignores `from_handle`/`to_handle` entirely when both
  spaces are analytic — returns a handle that silently re-derives from the handle
  arithmetic rather than the registered profiles

---

## 2. Blocking Issues (Critical or High Severity)

### 2.1 `catch_unwind` Does Not Catch OOM or Integer Overflow — Critical

Section 15.1 claims "`catch_unwind` is an effective isolation boundary" for
adversarial ICC files because qcms is pure Rust. This is partially true: qcms
panics on malformed curves and invalid tag counts. However two classes of failure
slip through:

**Integer overflow → `usize` wrap → massive allocation.** An adversarial ICC file
can craft a tag count or curve size that wraps a `usize` multiplication to a small
value, causing qcms to allocate a small buffer then read past it. In release mode
(`overflow-checks = false`, which musl/release targets use by default) this does
not panic — it silently allocates the wrong size and reads garbage. `catch_unwind`
cannot intercept a memory read from an incorrectly sized allocation.

**Global allocator abort on OOM.** If qcms allocates a legitimately large intermediate
buffer for a 4MB profile with many LUT entries, the global allocator may call
`handle_alloc_error`, which calls `abort()`. `abort()` is not a Rust panic; it
terminates the process with SIGABRT. `catch_unwind` does not intercept SIGABRT.

Required fix: set `overflow-checks = true` in the supervisor release profile, and
add an explicit pre-parse size check on the profile's internal tag table before
calling qcms:

```toml
# supervisor/Cargo.toml
[profile.release]
overflow-checks = true
```

Add a `validate_icc_header` function that reads the declared file size from the
ICC header (bytes 0–3, big-endian u32) and rejects files where the declared size
exceeds `MAX_ICC_BYTES` or does not match `data.len()`.

### 2.2 Transform Handle Encoding Has Collisions — High

`host_create_transform` encodes its return value as:

```rust
let handle = (from_handle & 0xFF) | ((to_handle & 0xFF) << 8) | ((intent_raw as u32) << 16);
```

This packs `from_handle` and `to_handle` into 8 bits each, truncating any profile
ID above 255. An app with 256 loaded profiles gets `from_handle = 256` truncated
to `0`, which aliases with the sRGB builtin. Subsequent `apply-transform` calls
silently use the wrong profile.

Additionally, the handle encodes `from_handle` as a `ColorSpace` u8 in
`host_apply_transform` even though `host_create_transform` received it as a
`ProfileId`. The two decodings are inconsistent: `create` treats the value as a
profile handle (registry lookup), while `apply` treats it as a raw `ColorSpace`
wire value. These will disagree for any profile handle above 5.

Required fix: use a `HandleTable<Arc<ColorTransform>>` (a `Vec` with free-list)
returning opaque incrementing u32 handles. Store the actual `Arc<ColorTransform>`
in the table rather than re-deriving from encoded fields.

### 2.3 `apply-transform` Double-Copy for Large Frames — High

`host_apply_transform` accepts `pixels: Vec<u8>` (already a copy from WASM linear
memory) and returns `Result<Vec<u8>, String>` (another copy back to WASM linear
memory). For a 4K frame (3840×2160 BGRA32) this is 2 × 33 MB = 66 MB of copies
per frame, or ~3.9 GB/s at 60 fps. On mobile this exceeds available memory bandwidth.

This was flagged as a known limitation in the WIT comment ("copies pixels across
the WASM boundary twice") but the spec offers no resolution path beyond a vague
"shared-memory extension". That deferral is acceptable only if it is explicitly
listed as a blocking limitation on mobile, not buried in a comment.

Required fix for this round: add a `apply-transform-to-surface` call that takes a
surface handle (from R11) rather than raw bytes. The supervisor already holds the
surface pixel buffer; no copy is needed:

```wit
/// Apply color transform to a named surface in-place.
/// No pixel bytes cross the WASM boundary.
apply-transform-to-surface: func(
  transform-handle: u32,
  surface-handle:   u32,
) -> result<_, string>;
```

Document `apply-transform` as a "convenience for small images only; do not use
for surfaces".

### 2.4 Compositor Allocates Full Surface Clone Per Frame — High

`blit_surface_color_managed` constructs:

```rust
let converted_surface = Surface {
    pixels: converted,  // Vec<u8>, 8–33 MB
    ..src.clone()
};
blit_surface_raw(dst, &converted_surface);
```

This allocates and immediately frees an 8–33 MB heap buffer every compositor
pass for every wide-gamut surface. At 60 fps with 5 app windows this is 300
allocations/second of multi-MB buffers, which causes fragmentation and measurable
latency spikes.

Required fix: `blit_surface_raw` must accept a pixel-slice override parameter, or
`convert_pixels` must write into a pre-allocated scratch buffer reused across
frames:

```rust
pub struct CompositorScratch {
    buf: Vec<u8>,
}

impl CompositorScratch {
    pub fn convert_and_blit(&mut self, dst, src, transform) {
        self.buf.resize(src.pixels.len(), 0);
        convert_pixels_into(&src.pixels, &mut self.buf, ...);
        blit_raw_pixels(dst, &self.buf, src.x, src.y, src.width, src.height);
    }
}
```

Keep one `CompositorScratch` on the compositor's flush-pass stack; it reuses the
allocation across frames.

### 2.5 `ImageData.color_space` Field Added Without Migration — High

Section 11 adds `pub color_space: crate::color::ColorSpace` as a plain field to
the existing `ImageData` struct. Every existing callsite that constructs `ImageData`
(all decoders from R14: `decode_png`, `decode_jpeg`, `decode_bmp`, `decode_qoi`)
will fail to compile because they do not initialize `color_space`.

If the field is given a `Default` impl, all decoders silently default to
`ColorSpace::Srgb`. That is the correct default for PNG and BMP, but JPEG files
may carry an embedded ICC profile chunk (APP2 marker) or an Exif ColorSpace tag
(value 1=sRGB, 65535=uncalibrated). Without reading that tag, a JPEG decoded from
a camera with a custom color profile will be mismanaged.

Required fix:
1. Add `color_space: ColorSpace` to `ImageData` with `#[serde(default)]` and a
   struct-update syntax fallback of `ColorSpace::Srgb`.
2. Update the JPEG decoder to read APP2 ICC chunk and set `color_space`
   accordingly. Stub as `ColorSpace::Srgb` with a `log::debug!` if no ICC found.
3. Add a unit test: decode a JPEG with an embedded DisplayP3 ICC chunk; assert
   `ImageData.color_space == ColorSpace::DisplayP3`.

### 2.6 EDID Gamut Detection Has High False-Positive Rate — High

`detect_gamut_from_edid` uses a single threshold:

```rust
if rx > 0.670 { ColorSpace::DisplayP3 } else { ColorSpace::Srgb }
```

sRGB specification red primary X = 0.640. However cheap IPS panels often report
red X ≈ 0.657–0.668, only slightly below the threshold. A 1% manufacturing
tolerance on chromaticity reporting means a sRGB monitor may read `rx = 0.671`
and be incorrectly tagged as DisplayP3. The compositor then applies a P3→sRGB
transform, desaturating all content by ~10% on a display that is actually sRGB.

Display P3 red X = 0.680. The threshold should be at least 0.675 to leave a
safety margin. Additionally, gamut detection should be confirmed by checking all
three primary coordinates (R, G, B), not just red X:

```rust
// Also check green Y: sRGB green Y ≈ 0.330, P3 green Y ≈ 0.330 (similar)
// Better signal: red X > 0.675 AND blue X < 0.145.
// sRGB blue X ≈ 0.150, P3 blue X ≈ 0.131.
let is_p3 = rx > 0.675 && bx < 0.145;
```

### 2.7 qcms Box Leak + Cast Causes Double-Free — Critical

`build_transform` does:

```rust
let t = qcms::Transform::new_to(...).ok_or(...)?;
Ok(Box::into_raw(Box::new(t)) as *mut std::ffi::c_void)
```

Then `QcmsTransformHandle::drop` calls:

```rust
qcms_sys::qcms_transform_release(self.ptr as *mut _);
```

`qcms::Transform` is a Rust struct with a `Drop` impl that releases internal
qcms state. `Box::into_raw(Box::new(t))` leaks the `Box` wrapper (intentionally)
but does **not** call `Transform::drop` on creation — correct so far. However
`qcms_transform_release` is a C-level function that operates on a raw
`qcms_transform_t *` pointer, not on a `Box<qcms::Transform>`. Casting a
`*mut qcms::Transform` to `*mut qcms_transform_t` is only valid if
`qcms::Transform` is `#[repr(transparent)]` over the C type.

In the qcms crate (v0.9 as of 2026) `qcms::Transform` is **not** transparent over
the C struct; it is a Rust-owned wrapper. Calling `qcms_sys::qcms_transform_release`
on a pointer to a `Box<qcms::Transform>` is undefined behavior.

Required fix: use qcms's safe Rust API exclusively. Store `Box<qcms::Transform>`
directly (with `unsafe impl Send + Sync` after verifying the qcms docs), and drop
via Rust's normal `Drop` chain. Do not call `qcms_sys` FFI functions directly:

```rust
pub struct QcmsTransformHandle {
    inner: Box<qcms::Transform>,
}
// SAFETY: qcms::Transform is safe for concurrent read-only apply calls.
unsafe impl Send for QcmsTransformHandle {}
unsafe impl Sync for QcmsTransformHandle {}
// Drop is automatic via Box<qcms::Transform>'s own Drop.
```

### 2.8 `host_create_transform` Ignores Registered Profiles — High

`host_create_transform` calls:

```rust
let _transform = state.transforms.get_or_create(
    from_profile.space,
    to_profile.space,
    intent,
    &state.registry,
    from_profile,
    to_profile,
);
```

`from_profile` and `to_profile` are the registered profiles looked up from the
registry — correct. But the returned `_transform` is immediately discarded.
The returned handle is then computed purely from handle arithmetic, bypassing
the cache:

```rust
let handle = (from_handle & 0xFF) | ...;
```

So `apply-transform` re-derives the transform from `ColorSpace` wire values,
ignoring whatever profile bytes were in the registry. A custom AdobeRgb ICC file
loaded by the app is never actually used for pixel conversion.

Required fix: store the `Arc<ColorTransform>` in a per-host-state handle table
and return the table index as the opaque handle.

---

## 3. Non-Blocking Issues (Medium Severity)

### 3.1 No Tagged Color Space on Surface / Window

R11 defines `Surface` as a BGRA32 pixel buffer with no color space annotation.
The compositor in section 13 uses `src_space: ColorSpace` as a parameter to
`blit_surface_color_managed`, but there is no field on `Surface` to carry this
value. The caller must supply it externally — which means the app must communicate
its surface's color space through some side channel.

This side channel is not specified. The compositor does not know which space a
given surface is in unless the supervisor tracks it per-app from the manifest.
Surfaces produced by R14 `ImageData::apply_color_transform` may have been
converted already; compositing them a second time would double-convert.

Recommended fix: add `pub color_space: ColorSpace` to `Surface` (R11 type). Set
it at surface creation from `ColorCapability.working_color_space()`. The compositor
reads it directly.

### 3.2 No HDR / EDR Support

BT.2020 is listed as a supported `ColorSpace`, but VyomaOS has no mechanism to
signal HDR metadata to the display. DRM HDR requires the `DRM_MODE_OBJECT_CONNECTOR`
`HDR_OUTPUT_METADATA` property set via `ioctl`, and a display that reports
`HDR Capabilities` in EDID extension block CTA-861-G.

The spec says nothing about:
- How apps declare HDR intent
- Whether the framebuffer is 10-bit (`XRGB2101010` / `ARGB2101010`)
- PQ/HLG transfer functions (qcms does not support PQ)
- Tone-mapping wide-gamut BT.2020 content to P3 displays

This is acceptable as a deferred item, but the spec must explicitly mark
BT.2020 as "color space tag supported; tone-mapping and HDR output deferred"
rather than listing it in the platform matrix as a first-class supported space.

### 3.3 Server-Headless "ICC for Compositing" Is Underspecified

The platform matrix says server-headless gets ICC enabled for "sRGB→LinearSRGB
for compositing". But server-headless has no display, no EDID, and no
`display-color-space` that makes sense. Yet `host_display_color_space` will
return `ColorSpace::Srgb` (wide_gamut == false), and the compositor will still
call `detect_display_profile` which calls `read_edid_bytes` and logs a warning.

This is benign but wastes startup time and fills logs. The server-headless
platform should skip `detect_display_profile` entirely and hard-code
`display_profile = SRGB_BUILTIN, wide_gamut = false` in
`color_config_for_platform`.

### 3.4 `ProfileRegistry.release_app` Does Not Remove Profiles

`release_app` decrements per-app byte accounting but leaves profile entries in
the map and LRU order. The comment says "individual profile entries remain;
they are evicted by LRU". This is correct for shared system profiles (sRGB,
display P3) that many apps may reference. However an app-specific custom profile
(from `profiles/custom.icc`) will linger until evicted by LRU, potentially
occupying a registry slot after the app has exited.

With 16 profile slots and 10 apps each loading one custom profile, slots exhaust
after 6 more registrations. This is low-probability in practice but should be
documented explicitly and the `max_profiles = 16` limit should be raised to 32 on
desktop-full.

### 3.5 `extract_desc` Tag-Table Walk Has Bounds Issues

The hand-rolled ICC `desc` tag parser uses `u32::from_be_bytes` on bytes from the
tag table without checking whether `data_off + data_len <= data.len()` before
slicing. The early `break` in the inner loop exits only the for-loop, not the
outer loop. If `data_len` is zero and `data_off` is valid, `text_start =
data_off + 12` may exceed `data.len()`, causing a panic in the slice indexing.

The check is present (`if data_off + data_len > data.len() || data_len < 12`)
but it is guarded by `if sig == b"desc"` so it only fires for the `desc` tag.
Tags before `desc` in the table are walked without the bounds check, which
could panic if `data_off` in an earlier tag points beyond the file.

This is isolated to `extract_desc`; it is behind `catch_unwind` in
`parse_icc_profile_bytes`. However the panic will be caught and logged as
`ParsePanic`, causing the profile to be rejected even though the pixel data is
fine. Low severity — fix by adding early-exit bounds checks at the top of the
tag-table loop.

---

## 4. What the Spec Got Right

### 4.1 qcms Over lcms2 — Correct Security Decision

The spec explicitly justifies using qcms over lcms2 on security grounds:
lcms2 is C code whose out-of-bounds memory accesses cannot be intercepted by
`catch_unwind`. This is the right call. Historical lcms2 CVEs (CVE-2013-4276,
CVE-2018-16435) involved heap buffer overflows in LUT parsing that would have
crashed the supervisor. qcms has no C component; all panics are Rust panics.

### 4.2 `MAX_ICC_BYTES = 4 MB` is Appropriate

Most real-world display ICC profiles are 1–500 KB. The 4 MB ceiling blocks the
class of "large LUT" attacks while accepting all legitimate profiles. lcms2
documentation recommends a similar ceiling.

### 4.3 Per-App Byte Accounting

Tracking bytes-per-app-PID in the registry prevents a single misbehaving app
from crowding out the system display profile. The limit logic is structurally
sound (check before parse, not after).

### 4.4 Platform Matrix Is Correct

Disabling ICC on mcu-minimal, iot-edge, and robotics-rt is correct. These
platforms have no wide-gamut displays and no memory budget for a color pipeline.
The Cargo feature gate (`color-icc` / `qcms`) correctly excludes the qcms
dependency from embedded builds.

### 4.5 Analytic sRGB ↔ LinearSrgb Path

Handling the sRGB↔LinearSrgb case analytically (without qcms) is correct and
important: this conversion is needed on every compositor pass for linear-light
blending, not just for app-requested transforms. The piecewise approximation
(s ≤ 0.04045 branch) matches the IEC 61966-2-1 standard exactly.

### 4.6 WIT Return Types

`display-color-space` returning `u8` (not `color-space` enum) is forward-
compatible: apps that predate a new enum value will not fail to compile when a
new `ColorSpace` is added. The same pattern was used successfully in R12 GPU.

### 4.7 `ProfileRegistry` LRU Is Correct

The LRU eviction strategy is appropriate. The alternative (FIFO eviction) would
evict the system display profile after 16 apps each load one custom profile.
LRU keeps the display profile hot as long as the compositor accesses it every
frame (which it does).

---

## 5. Questions the Spec Must Answer

### Q1: Default Behavior for Apps Without `[capabilities.color]`

An app that does not declare `[capabilities.color]` can still call
`VYOMA_DRAW:fill_rect` and `VYOMA_DRAW:draw_text`. On a wide-gamut display,
are those drawing commands treated as sRGB content and transformed before
compositing, or are they left in "display native" encoding?

The spec does not answer this. The compositor code in section 13 uses
`src_space` as a parameter but does not say what value is used for apps without
a color manifest entry. Defaulting to `ColorSpace::Srgb` is correct, but it
must be stated explicitly and the compositor must always have a `src_space`
regardless of whether the app declared `[capabilities.color]`.

### Q2: Does `blit-image` in R14 Trigger Automatic Color Management?

R14 defines a `blit-image` WIT call that blits decoded `ImageData` onto a
surface. `ImageData` will now carry a `color_space` field. Does `blit-image`
automatically transform pixels from `ImageData.color_space` to
`Surface.color_space`?

If yes: R14's implementation must be updated to perform this transform on every
blit, adding latency and allocation pressure. If no: image color is silently
wrong for DisplayP3 images on sRGB surfaces.

The spec must define a policy: "auto-transform on blit if color spaces differ,
using `RenderingIntent::Perceptual` by default". This should be a flag on the
blit call: `blit-image-cm` with an explicit intent, and `blit-image` as a
fast path that assumes the source is already in the surface's color space.

### Q3: How Does the Compositor Know Each Surface's Color Space?

Section 13 calls `blit_surface_color_managed(dst, src, src_space, dst_space,
...)` but never specifies where `src_space` comes from. The `Surface` type (R11)
has no `color_space` field. The manifest `working_color_space` sets the intent
at app startup, but an app could change its surface's color space at runtime by
calling `apply-transform` and then blitting different content.

This question is answered by the recommendation in §3.1 (add `color_space` to
`Surface`), but the architect spec must say so explicitly rather than leaving it
as a parameter with an unspecified source.

### Q4: Can Two Apps Share a Loaded Profile?

App A loads `sRGB-display.icc`, gets handle 3. App B loads the same file path,
gets handle 4. Are these the same registry entry (deduplicated by path) or two
separate entries consuming two of the 16 slots?

The spec implies two separate entries (next_id increments per registration). This
wastes slots. The registry should deduplicate by content hash (SHA-256 of the
ICC bytes) and return the existing handle if found. Document the deduplication
strategy explicitly.

### Q5: What Happens on Display Hotplug?

The supervisor detects the display ICC profile once at startup in
`detect_display_profile`. If the user plugs in a second monitor with different
gamut (common on desktop), `ColorConfig.display_profile` is never updated.

The compositor continues transforming content for the first monitor's gamut.
Content on the second monitor renders incorrectly. macOS ColorSync handles this
via `CGDisplayRegisterReconfigurationCallback`.

The spec should either: (a) define a display-change event that triggers
re-detection and emits a `VYOMA_EVENT:display_changed:` IPC message to apps with
`[capabilities.color]`, or (b) explicitly state that multi-monitor color
management is out of scope and document the limitation.

---

## 6. Closing Assessment

The architect made the most important decision correctly: qcms over lcms2,
eliminating an entire class of memory-safety bugs that have historically
compromised OS-level color pipelines. The module decomposition, platform matrix,
and registry design are structurally sound.

The two critical bugs (integer overflow bypass and `Box<Transform>` double-free)
must be fixed before any production use. The transform handle collision and
"registered profiles ignored" bugs mean the WIT API as written does not do what
it says — a functional correctness failure, not just a performance or UX gap.

The double-copy problem in `apply-transform` is the same architectural issue
flagged in R14 for `decode-image`. The pattern needs resolution at the
VyomaOS ABI level: either shared memory regions (which WASI Preview 2 supports
via the `wasi:memory` proposal), or surface-handle-based operations that keep
pixel data in supervisor memory. Both the image pipeline and color management
specs are blocked by the same missing primitive.

Once the eight blocking issues are resolved, Round 15 is approvable.
