# Round 17 — Advanced Rendering API: Critic Review
## VyomaOS Subsystem 17 — VYOMA_DRAW v3

**Date**: 2026-05-29  
**Reviewer**: Critic Agent  
**Subject**: `17-rendering-api.md` — VYOMA_DRAW v3 Architect Draft

---

## Verdict

This is a thoughtful spec. The three-tier protocol strategy is architecturally sound, the
Rust types are precise, the WIT interface is complete, and the rasterizer description gives
implementers enough to begin. The R13 glyph integration call chain is the clearest text
rendering spec this series has produced. However, there are five blocking issues — including
a throughput problem in the v2 protocol, a vsync threading hazard, an undefined protocol
recovery path, and two missing color-space correctness definitions — that would cause
visible production failures in a shipping UI. Eleven non-blocking issues will compound into
maintenance debt and user-visible quality gaps. The spec needs revision on all blocking
items before implementation begins.

---

## Blocking Issues

### B1: v2 Text Protocol Throughput Makes Production UI Impossible at 60Hz

Section 5 describes the v2 extended text protocol: path construction via lines like
`VYOMA_DRAW:curve_to:...`. Each line is written to stdout, read by the supervisor's
stdout reader thread, matched against a string prefix, then parsed into f32 fields via
`split(',')` and `str::parse::<f32>()`.

The spec does not state the benchmark, so we must calculate it.

A moderately complex UI frame — a settings panel with rounded rectangles, a progress bar,
a text label with inline icon, a highlighted row — might involve:

- 4 rounded rects × 8 segments each = 32 `curve_to` lines
- 2 progress bars × 2 segments each = 4 `line_to` lines
- 3 gradient fills × 1 `linear_gradient` + 3 `gradient_stop` lines = 12 lines
- 6 draw_text calls = 6 lines
- 4 save_state + 4 restore_state = 8 lines
- 4 flush calls = 4 lines

Total: approximately 66 stdout lines per frame, before any actual content. At 60Hz
that is 3,960 string parse operations per second from a single app. A UI with three
simultaneous apps doubles that. Each `str::parse::<f32>()` call for a `curve_to` parses
six floats from a single comma-separated string — that is 18,000–24,000 individual float
parses per second per complex app.

By comparison, macOS CoreGraphics uses a CGPathRef — a typed binary struct that the
kernel never sees as text. Flutter uses a binary serialization channel. React Native uses
a JSI bridge that passes typed values. Every production 2D graphics API treats text
serialization as a debug format, not a production data path.

The spec argues v2 is for "transitional UI" while v3 handles "full UI apps." But this
creates a migration trap: an app that starts with v1 for simplicity, adds v2 for rounded
rects, and then discovers that at 60Hz with 4 apps running, the supervisor's stdout
reader thread is the throughput bottleneck. The v3 WIT interface is the correct answer,
but the spec offers no migration tooling, no deprecation timeline for v2, and no
admission that v2 is unsuitable for 60Hz applications. An implementer reading this spec
will build v2 as a first-class feature, not as a stepping stone to v3.

**Required fix**: Add a clear statement to Section 5 that v2 is rate-limited to
applications targeting < 30Hz update rates or drawing fewer than 20 path segments per
frame. Add a "Why v3 exists" subsection under Section 1 that quantifies the v2 parse
overhead and states explicitly that apps with any animation must use v3. Add a migration
example showing a v2 rounded-rect replaced with v3 `fill-path` calls.

---

### B2: Path Rasterization on the Compositor Thread Blows the Vsync Budget

Section 6 states that draw commands are dispatched through a `draw_command` function that
calls into submodules. Section 7 describes the rasterizer as running a scanline fill.
Section 8 states "A 960×540 full-screen gradient fill ≈ 2ms."

The spec does not say which thread runs the rasterizer.

The R11 architecture (supervisor/src/display.rs) runs a vsync compositor that fires
every 16.6ms, reads all dirty surfaces, and composites them to the framebuffer. If the
rasterizer runs on the same vsync thread, a single complex path draw by a single app
consumes the entire vsync budget. Multiple apps each submitting complex paths in the same
frame produce vsync misses (displayed as dropped frames or tearing).

Here is the budget breakdown the spec omits:

```
16.6ms vsync budget
  - R16 animation interpolation for all layers: ~0.5ms
  - R11 surface compositing (blit all dirty surfaces to framebuffer): ~0.5ms
  - Available for path rasterization: ~15.6ms
  - Per-app rasterization budget (4 apps): ~3.9ms each
  - A 960×540 gradient fill: 2ms (spec's own number)
  - A rounded rect with AA (200×100 px, 8 cubic segments): ~0.3ms
  - A frame with 5 rounded rects + 1 gradient fill: ~3.5ms
```

This barely fits for a single app. With four apps drawing simultaneously, the rasterizer
needs to be parallelized across apps, or rasterization must move off the vsync thread
entirely.

The spec does not define this. "The rasterizer... rasterizes paths on the vsync thread"
is implied by the module structure (draw commands flow from the display parser to
`draw_command`) but never stated, let alone justified. The spec mentions GPU acceleration
as a future flag but does not address the more urgent question of CPU-side parallelism.

**Required fix**: Add a Section 6.5 "Threading Model" that explicitly states: (a) which
thread runs `draw_command` dispatch, (b) whether per-app rasterizers run in parallel, (c)
whether rasterization is synchronous with the vsync tick or buffered. The recommended
architecture is: each app's `GraphicsContext` maintains a `command_queue: Vec<DrawCommand>`
filled by protocol/WIT dispatch (any thread). At vsync, the compositor spawns one Rayon
task per dirty surface to rasterize in parallel, then composites the results. This delivers
near-linear scaling with app count.

---

### B3: v2 Protocol State Machine Has No Defined Recovery From App Crash Mid-Path

Section 10 ("App Crash Mid-Path") states: "On app exit, the supervisor calls
`app_state.draw_protocol_state = ProtocolState::default()`."

This handles the case where the OS kills the app process. But there are three additional
mid-path failure modes the spec does not address:

**Case 1: App hangs without exiting.** An app calls `begin_path`, writes 500 `curve_to`
lines, then blocks waiting for a lock. The supervisor's stdout reader has 500 pending
segments in `ProtocolState::PathBuilding`. The app never calls `fill` or `stroke`.
The path state sits in memory forever. How long does the supervisor wait before discarding
in-progress path state? There is no timeout defined.

**Case 2: App writes ill-formed lines mid-path.** After `begin_path`, the app writes
`VYOMA_DRAW:curve_to:not_a_float,x,y,z,w` due to a formatting bug. The spec says parse
errors are "logged as warnings." Does the supervisor discard the line and continue building
the path (skipping a segment, silently corrupting the shape), or does it abort the path
and return to Idle? Neither behavior is documented.

**Case 3: Protocol interleaving.** The spec states apps mix v1 and v3 in the same frame
but does not address v1 inside a v2 path sequence. App writes `begin_path`, then
`fill_rect:0,0,100,100,0xFF0000FF` (v1). Does the supervisor execute the `fill_rect`
immediately (executing a draw while a path is open), reset the path state, or queue the
fill_rect until the path closes? The spec says "a warning is logged and the current path
state is discarded" but this is buried in a parenthetical in Section 5 with no coverage
of what happens to the v1 command that caused the discard.

**Required fix**: Add a "Protocol Error Recovery" subsection to Section 5 defining all
three cases with explicit state transitions. Specify a `path_state_timeout_ms` (suggested:
100ms) after which an open `ProtocolState::PathBuilding` is automatically discarded and an
error is logged. Define parse error behavior as: log + discard segment + continue building
path (not abort), since aborting on any bad segment creates fragile app behavior.

---

### B4: Gradient Color Space Is Defined Once and Then Contradicted

Section 6 (`supervisor/src/draw/gradient.rs`) states: "All interpolation in linear-light
sRGB (R15 `ColorConvert` used for stop values if input is tagged sRGB)."

Section 3 (`blit_glyph_alpha`) does math directly on `u8` pixel values without any
mention of color space. The solid-color fast path in `blit_glyph_alpha` computes:
```rust
let out_a = src_alpha + dst_a * (1.0 - src_alpha);
```
This is correct Porter-Duff, but only if `src_r`, `src_g`, `src_b` are in linear-light.
The glyph paint comes from `VyomaPaint::SolidColor(r, g, b, a)`. The spec never states
what color space these f32 values are in. If an app passes sRGB values (0.5 = 128/255
in sRGB ≈ 0.214 linear) directly as `SolidColor`, but the blending math assumes linear,
colors will be 2× darker than expected.

This is not a hypothetical. The existing v1 protocol uses packed u32 RGBA like
`0xFF8000FF` (orange, 8-bit sRGB). When an app migrates to v3 and passes
`set-fill-color(1.0, 0.502, 0.0, 1.0)` expecting orange, are those values sRGB or linear?
The WIT interface has no annotation. The spec has no normative statement about what color
space WIT color parameters use.

The gradient section says "if input is tagged sRGB" but WIT f32 parameters carry no tags.
The tagging must be a convention, not a runtime feature, and that convention must be
stated in the spec.

**Required fix**: Add a "Color Space Contract" paragraph to Section 2 (Core Rust Types)
and to the WIT interface in Section 4: "All color values passed to `vyoma:draw@3.0.0`
WIT functions are in linear-light sRGB with premultiplied alpha, using the f32 range
[0.0, 1.0]. Apps working with sRGB (gamma-encoded) u8 values must convert via R15
`color-convert` before calling draw functions. Failure to convert produces darker colors
than expected." State the same rule in the v2 text protocol (Section 5): colors passed
to `fill:` and `stroke:` are in the same linear-light sRGB convention.

---

### B5: The WIT Interface Has No Way to Batch Commands

The v3 WIT interface in Section 4 defines one WIT function per drawing operation. Each
function is a synchronous cross-component call through the Wasmtime linker. The latency
of a WIT host function call is approximately 50–200ns (context switch overhead, ABI
marshaling). A frame with 100 path segments at 100ns each = 10µs per frame. This is
acceptable. But a frame with 1,000 WIT calls (common for a complex document renderer or
a chart with many data points) = 100µs per frame, and with the segment cap at 10,000
WIT calls = 1ms just in call overhead, before any rasterization.

More importantly: the current interface provides no transaction or batch concept
equivalent to v2's stateful path-building (`begin_path` → many segments → `fill`).
Apps must call `new-path`, then N calls to `move-to`/`line-to`/`curve-to`, then
`fill-path`. Every one of those is a separate round-trip through the Wasmtime linker.
CoreGraphics avoids this with `CGPathRef` (a heap-allocated struct the app builds
client-side and passes by reference). WASM components can't share heap pointers.

The spec acknowledges neither this overhead nor the design pattern to mitigate it.

**Required fix**: Add a WIT function `submit-path-buffer: func(ctx: u32, data: list<u8>) -> result<u32, string>` that accepts a compact binary encoding of path segments (4 bytes op-code + 6 × f32 floats per cubic segment = 28 bytes/segment). A path with 1,000 segments is one WIT call with a 28KB list, not 1,002 individual calls. Define the binary encoding in an appendix. This matches the approach used by Skia's `SkPaint::getFillPath` and Chrome's Blink path serialization.

---

## Non-Blocking Issues

### N1: AffineTransform Applied to Path Creates a Hidden Allocation

`supervisor/src/draw/transform.rs` defines `apply_to_path(path) -> VyomaPath`. This
creates a new `VyomaPath` with all segments transformed. For a path with 500 segments,
this allocates a new `Vec<PathSegment>` of 500 elements every time the path is drawn.
If the app draws the same path at a different position every frame (e.g., a moving icon),
this is one heap allocation per frame per path.

The preferred approach: the rasterizer takes both the path (in user space) and the
current `AffineTransform`, applying the transform during edge-table construction — no
new path allocation. The `apply_to_path` function should be marked as a utility for the
few cases where a transformed copy is genuinely needed (e.g., storing a transformed path
for hit-testing).

### N2: `VyomaPath::bbox` Cache Is Not Invalidated on Transform Change

The path stores a `bbox: Option<[f32; 4]>` described as "Cached bounding box in user
space." The rasterizer uses this for dirty-rect tracking. But if the `AffineTransform`
changes between frames without touching the path, the cached bbox is stale — it's in the
wrong coordinate space. The cache must be invalidated when the context transform changes,
not just when path segments are modified. Either move the bbox to device space (post-
transform) or annotate the cache with the transform it was computed under.

### N3: `PatternRef` Is an Opaque Index With No Lifecycle Documentation

Section 2 defines `PatternRef(pub u32)` as an opaque index into a "per-context pattern
table." Section 4's WIT defines `create-pattern` and `drop-pattern`. But Section 6
(module descriptions) contains no mention of where the pattern table lives. The
`context.rs` module description talks about state stack and clip stack but not pattern
storage. The `image.rs` description handles `draw_image` but not pattern evaluation.
`gradient.rs` handles linear/radial but not pattern. Which module owns pattern
rasterization? This gap will cause implementers to scatter pattern logic across files.

### N4: Clip Path Implementation Has Two Incompatible Models

Section 4 defines `clip-path: func(ctx, path, rule)` which clips to a path shape.
Section 5 defines `VYOMA_DRAW:clip_path` which "clips to the currently built path."
Section 6 (`context.rs`) says "clip_stack stores a stack of `Option<VyomaPath>` clip
masks; intersection of the active clips is computed at paint time."

"Computed at paint time" means computing the intersection of two arbitrary paths during
rasterization — a non-trivial operation equivalent to boolean path operations (clipping
path A by path B). This is hard to implement correctly and expensive for complex paths.
The simpler and more common approach (used by Skia and CoreGraphics) is to rasterize the
clip mask into a per-pixel alpha buffer (the "clip mask") and multiply it with the source
coverage during blending. The two approaches have different implementations and different
edge cases. The spec must commit to one model.

### N5: `draw_text` Font Selection Has No API

The WIT `draw-text` function signature is:
```
draw-text: func(ctx: u32, text: string, font-size: f32, x: f32, y: f32) -> result<_, string>
```

There is no font family, weight, or style parameter. Section 3 says "Resolve font
descriptor from ctx.current_font." But there is no `set-font` function in the WIT
interface. How does an app set the font before calling `draw-text`? The v3 interface
as specified always uses whatever the context's default font is, which is not documented.
This is a usability cliff: an app cannot render bold text, a different typeface, or a
monospace font without an unreachable escape hatch.

Add `set-font: func(ctx: u32, family: string, weight: u32, style: u32, size: f32)` or
accept a font-descriptor record in `draw-text` directly.

### N6: Image Handle Lifecycle Across Component Boundary Is Undefined

The WIT `draw-image` takes `image: u32` described as an "image handle (R14)." But in
the WASM component model, a `u32` returned from one WIT interface is not automatically
valid in another. The R14 `image-load` WIT interface returns a handle typed in the R14
world. Passing that handle's `u32` value to the R17 `draw-image` function relies on the
supervisor using the same underlying integer-keyed table for both R14 and R17. This is
an undocumented cross-interface contract.

Either: (a) define a shared `image-handle` resource type in a `vyoma:images` base
package that both R14 and R17 import, or (b) add an explicit statement that the
supervisor's image handle table is global and R14 handle integers are valid in R17 calls.
Without (a) or (b), an implementer who scopes image handles to the R14 world will produce
a bug where R17 receives an integer that fails handle lookup.

### N7: `MAX_FLATTEN_DEPTH = 32` Is Undersized for Sub-Pixel Curves

Section 7 sets a maximum de Casteljau recursion depth of 32. At each level, the curve
is halved. After 32 halvings, the parametric step is `2^-32 ≈ 2.3e-10`. For a cubic with
control points spanning a 960-pixel surface, the maximum error at depth 32 is approximately
`960 × 2^-32 ≈ 2.2e-7` pixels — well within tolerance.

However, the cap is stated as a defense against "degenerate inputs where endpoints are
numerically identical." A degenerate cubic with all four control points at the same
coordinates `(0, 0)` never gains flatness on subdivision because all split points are
also `(0, 0)`. The recursion hits depth 32 and stops. This produces a single point in
the output, which is correct. But the log will emit "flatness limit hit" for a degenerate
path that is geometrically valid. Add a degenerate-case pre-check: if all control points
are within `tolerance` of each other, emit a single `LineTo` the endpoint and return
without recursion.

### N8: `stroke-rect` Is Not Equivalent to Stroking a Rect Path

The WIT defines `stroke-rect: func(ctx, x, y, w, h)` as a convenience function.
Stroking a rectangle with `line_join = Miter` and `line_width = 20.0` produces corners
that extend outside the bounding box by up to `line_width / 2 * miter_limit`. Apps may
be surprised that `stroke-rect(0, 0, 100, 100)` with a thick stroke draws outside the
[0, 100] × [0, 100] area. This is correct behavior (consistent with every other 2D API)
but should be documented explicitly, since v1's `fill_rect` is always within bounds and
developers migrating from v1 to v3 will assume the same for `stroke-rect`.

### N9: The Protocol State Machine Has No Per-App Instance Documentation

Section 5 defines the protocol state machine as transitioning between `Idle` and
`PathBuilding`. Section 10 says `app_state.draw_protocol_state = ProtocolState::default()`
on app exit. But Section 6 (`protocol.rs`) says "parse_v2_line(state: &mut ProtocolState,
line: &str)" takes `state` as a mutable reference. Where is this `ProtocolState` stored?
Is it in `AppState`? Is it in a per-window `DrawSurface`? Is it in the per-window
`GraphicsContext`? An app with multiple windows would need multiple `ProtocolState`
instances. The ownership chain from `AppState` → `ProtocolState` is never drawn.

### N10: `flush: func(ctx)` Has No Defined Atomicity

Section 4 defines `flush: func(ctx)` as: "Commits all pending draw commands to the
surface pixel buffer. The surface becomes visible at the next vsync tick (R11)."

R11 uses a double-buffer model (draw to back buffer, flip at vsync). But "commits all
pending draw commands to the surface pixel buffer" is ambiguous: does flush (a) rasterize
all queued commands into the back buffer synchronously, then return, or (b) enqueue the
command list for the vsync thread and return immediately? Model (a) means flush blocks
the app thread until rasterization is complete — potentially 2ms+ for complex frames.
Model (b) means the app can modify data (e.g., free an `ImageHandle`) before the vsync
thread reads it, creating a use-after-free.

This must be defined. The recommended model: (a) flush rasterizes synchronously into the
DrawSurface pixel buffer on the calling thread, then sets `dirty_rect`. The vsync
compositor reads dirty_rect in the next tick and blits to the framebuffer. No inter-thread
races because the DrawSurface pixel buffer is not shared after flush returns.

### N11: `set_alpha` v2 Command Interacts With Premultiplied Pixels in an Undefined Way

Section 5 defines `VYOMA_DRAW:set_alpha:<value>` as "global alpha applied on top of
per-paint alpha." Section 2 defines `DrawSurface` pixels as "BGRA8 premultiplied."

When global alpha is 0.5 and a solid-color fill paints a pixel with alpha 1.0, the
effective alpha is 0.5. But the destination pixel already has premultiplied color values.
If the blending formula multiplies the source by global alpha before Porter-Duff src-over,
the math works. But if global alpha is applied by post-multiplying the premultiplied
source pixel (a common implementation mistake), the output is incorrect. The spec must
state exactly where in the blend pipeline global alpha is applied: "global_alpha
multiplies src_a before Porter-Duff compositing, equivalent to
`final_src_a = paint_alpha * global_alpha`."

---

## Got Right

### R1: Three-Tier Strategy Is the Correct Abstraction Boundary

The v1/v2/v3 separation is architecturally honest. It acknowledges that the existing v1
protocol cannot be broken, that binary WIT is the right long-term answer, and that an
intermediate v2 is needed for the transition period. Most specs paper over the
compatibility problem; this one names it explicitly and assigns each tier a platform scope.

### R2: WIT Interface Coverage Is Complete

The WIT interface in Section 4 covers all operations needed for a production UI app:
path construction, gradients, transforms, clipping, state stack, image blit, text,
blend modes, and the full gradient extend model. Nothing critical is missing from the
interface surface. The naming follows WebGPU and W3C Canvas conventions, which will
feel familiar to implementers.

### R3: Glyph Integration Call Chain (Section 3) Is the Best in the Series

The R13 glyph atlas integration is specified more concretely than any prior round. The
two code paths (`blit_glyph_alpha` for solid color, `fill_glyph_as_path` for gradients)
are the right split: the fast path avoids per-pixel paint evaluation for the common case,
and the gradient path is a natural extension. The `Arc<Mutex<GlyphAtlas>>` ownership
model is correct for multi-threaded rasterization.

### R4: Security Caps Are Specific and Justified

`MAX_PATH_SEGMENTS = 10_000`, `MAX_GRADIENT_STOPS = 64`, save/restore depth 64. Each cap
has a stated rationale. The handle isolation model (per-app table, dropped on app exit)
correctly prevents cross-app handle spoofing. The arc clamping (`sweep_deg` to ±360°,
`rx/ry` to `(ε, 65536]`) is the right set of constraints with the right error semantics.

### R5: Dirty Rect Tracking Is Included

The spec includes dirty rect tracking on `DrawSurface` and connects it to the R11 vsync
compositor. This is often omitted from first-pass rendering API specs and then retrofitted
painfully. Including it here, with the explicit statement that only the dirty region is
copied to the framebuffer, means the compositor will be efficient from day one.

### R6: Platform Matrix Is Honest

Stating that v2/v3 are absent from server-headless is correct: a headless server has no
display device and no reason to parse path commands. The graduated feature matrix
(iot-edge gets basic paths, not curves) matches real RAM constraints. The footnotes on
what "basic" means for iot-edge are specific enough to implement.

### R7: Module Boundary at 500 Lines Is Enforced by Design

Splitting the rendering engine into ten modules — each with a stated purpose and an
explicit target line count — is the right way to enforce the repo's 500-line rule
before implementation rather than after. Naming `rasterizer.rs`, `stroke.rs`,
`gradient.rs`, `blend.rs` as separate files prevents the common failure mode where all
of this logic ends up in one 2,000-line `draw.rs`.

---

## Questions for the Architect

**Q1**: Section 1 says "v2 adds paths, gradients, images via extended text commands."
But Section 9's platform matrix shows iot-edge gets v2 "basic" (no curves, no gradients).
If iot-edge supports v2 at all, the supervisor on iot-edge must compile `protocol.rs`.
Does compiling `protocol.rs` pull in the rasterizer? If yes, the draw subsystem's RAM
cost on iot-edge is non-trivial. What is the estimated binary size of `protocol.rs` +
`rasterizer.rs` + `stroke.rs` when compiled for `aarch64-unknown-linux-musl`? Is this
within the iot-edge 4MB RAM budget?

**Q2**: Section 7 describes the scanline fill as producing coverage values via 4×4 MSAA
supersample. 4×4 MSAA requires evaluating 16 subpixel samples per edge-adjacent pixel.
For a 200-pixel-tall closed path, this is 3,200 subpixel evaluations for edge pixels.
Has the architect benchmarked this specifically? The claim "~1M pixels/sec on a single
core" in Section 8 is for "simple polygons." Does this number hold for AA paths, or is
the actual AA pixel rate lower?

**Q3**: Section 6 (`context.rs`) says clip_stack stores `Option<VyomaPath>`. The "intersection
of the active clips is computed at paint time." Does this mean VyomaOS will implement
arbitrary path intersection (Sutherland-Hodgman or Weiler-Atherton clipping)? These
algorithms are correct but complex. Skia and CoreGraphics both use alpha-mask clipping
instead. Is there a decision here, or is this an implementation detail left to the
implementer? If left to the implementer, the spec should say so explicitly.

**Q4**: The v2 `draw_image` command takes `<image_handle>` as an integer. Where does this
integer come from? R14 defines an image WIT interface. But an app that uses only v2 (text
protocol, no WIT) cannot call R14 WIT to obtain an image handle. Is there a v2 text
command to load an image? Something like `VYOMA_DRAW:load_image:<path>` that returns a
handle? If not, v2 apps cannot use `draw_image`, and the command in Section 5 is
unreachable for pure-v2 apps. This is a protocol design gap.

**Q5**: Section 8 mentions "path caching" as a future optimization. The prerequisite
is `VyomaPath::bbox`. But a more valuable prerequisite is an immutable path type: once
a path is built and its edge table computed, the edge table can be reused across frames
as long as the transform doesn't change. Has the architect considered a `PathCache` type
that stores the flattened edge table? This would reduce per-frame rasterization cost for
static UI elements (icons, borders, fixed shapes) from O(N segments) to O(scanlines),
which is the dominant cost in steady-state UI rendering.

---

## Closing

The core architecture is correct: a typed WIT v3 interface for production apps, a text
v2 for transitional use, and v1 preserved forever for simple apps. The rasterizer design
is implementable. The security model is solid. The R13 glyph integration is concrete
enough to code from directly.

The five blocking issues are concentrated in two areas: protocol performance and threading
correctness (B1, B2, B5) and color-space/state-machine contract gaps (B3, B4). None of
these require redesigning the architecture — they require adding subsections that the spec
currently omits. B5 (batch path submission) requires one new WIT function and a binary
encoding appendix.

The non-blocking issues are real quality debt: the clip-path implementation model (N4),
font selection gap (N5), image handle cross-interface contract (N6), and flush atomicity
(N10) will all produce bugs if left unresolved. They can be resolved in a v1.1 revision
during implementation rather than blocking the initial implementation start, as long as
implementers are warned they are open questions.

Recommend: revise on B1–B5, then hand to implementer with N1–N11 logged as tracked
issues. This is a near-ready spec.
