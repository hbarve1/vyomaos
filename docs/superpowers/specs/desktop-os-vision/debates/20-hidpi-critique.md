# Critique: HiDPI & Multi-Resolution Display (Round 20)

## Verdict

The spec is thorough on the happy path: integer-only scale factors, opt-in `hidpi_aware`
manifest flag, physical-pixel Surfaces, and a scale-agnostic compositor are all coherent
design choices. However, five blocking issues prevent implementation: the backward-
compatibility model contains a hidden breakage vector for any app that queries its window
size; the coordinate-rounding policy, while stated, omits the one case where even integer
multiplication can produce a physically impossible result; the `VYOMA_VDISP_SCREEN`
extension breaks an existing contract in a way the spec mischaracterises as backward-
compatible; the font auto-promotion decision is correct but the 2x large-font overflow
case is underdefined; and the `hidpi_aware` flag gating creates an invisible split in
coordinate semantics that will produce silent misrenders when apps are ported.

---

## Blocking Issues

### B1: Legacy App Coordinate Semantics Break When Surface Is Mis-Sized at 2x

Section 3.4 states that non-HiDPI-aware apps have their `VYOMA_DRAW:` coordinates treated
as physical pixels. Section 17.2 shows the supervisor reporting `scale_factor=1` to a
legacy app even on a 2x physical display. This is the backward-compatibility guarantee.
However, the spec does not address what happens to the legacy app's Surface allocation.

The Surface is sized in physical pixels (§5.1). At 2x display with a 1920×1080 physical
framebuffer, what size is the legacy `shell` app's Surface? The spec does not say. Two
interpretations are possible:

**Interpretation A**: The legacy app's Surface is 960×540 (the logical size of its window,
which the spec says legacy apps "occupy a fraction of the physical screen"). In this case,
the `VYOMA_DRAW:fill_rect:0,0,960,540,bg` issued by the legacy app fills the entire
960×540 Surface correctly. The compositor blits the 960×540 Surface into the top-left of
the 1920×1080 fb as shown in §17.2.

**Interpretation B**: The legacy app's Surface is 1920×1080 (the full physical framebuffer
size), because all Surfaces are sized to the full physical screen. The legacy app then
draws to coordinates 0,0,960,540, which covers only the top-left quadrant of its Surface.
The other three quadrants display the Surface initialization value (transparent black).

These are not equivalent. Interpretation A makes the legacy app's window physically half
the screen size. Interpretation B gives it a full-screen window but with content in one
quadrant. Neither behavior is explicitly chosen in the spec.

More critically, the spec says in §5.2 that HiDPI-aware apps learn their logical
dimensions via `VYOMA_DISPLAY:window_info`. Legacy apps receive `window_info` with
`scale_factor=1` and a width/height — but what width and height? If the legacy app
receives `960,540,1`, it believes its Surface is 960×540 physical pixels (because for a
1x app, logical == physical). If its actual allocated Surface is 1920×1080 (Interpretation
B), then any out-of-bounds clipping behavior is undefined and any compositor blit that
reads beyond the app's written region produces visual garbage.

The spec must define precisely: what is the Surface width and height for a non-HiDPI-aware
app running on a 2x display, and what coordinates are reported in `window_info` for such
an app. The two choices are `(physical_w, physical_h, 1)` (legacy app fills the full
framebuffer, coordinates in physical pixels, no scale) or `(logical_w, logical_h, 1)` (legacy
app fills a quadrant, coordinates in physical pixels which happen to equal logical pixels
at 1x). The spec must choose and document one answer. Until it does, the Surface allocation
path and the window_info reporting path are inconsistent and cannot be implemented
without guessing.

---

### B2: Coordinate Overflow on Large Surfaces at 2x

Section 3.3 defines the rounding policy and notes that for integer scale factors, `point ×
scale_factor` is always an exact integer. This is correct. However, the spec overlooks an
integer overflow scenario that produces a physically invalid physical coordinate.

VYOMA_DRAW coordinates are `u32`. At 2x, a coordinate of `2,147,483,648` (2^31) produces
a physical pixel coordinate of `4,294,967,296` which overflows `u32`. More practically:
a coordinate of `1,073,741,825` at 2x produces `2,147,483,650`, which wraps to `2` on
32-bit overflow. This is not a theoretical edge case. If an app has a bug and emits a very
large coordinate value (e.g., a negative i32 miscast as u32 produces values near u32::MAX),
scaling that value by 2 produces a wraparound that places a draw operation at coordinate 0
on the physical framebuffer — silently corrupting the display rather than clipping to
nothing.

The existing out-of-bounds clipping (§15.1) catches coordinates that exceed the
framebuffer dimensions after scaling. But if overflow occurs before the clip check, the
clipped coordinate after wraparound may be inside the framebuffer bounds and the draw will
proceed to the wrong location.

The spec must specify: the scale_coord computation must use saturating multiplication, not
wrapping multiplication. In Rust, `u32::saturating_mul(2)` clamps to `u32::MAX` on
overflow instead of wrapping. A saturated value (e.g., `4,294,967,295`) will then be
caught by the out-of-bounds clipper and discarded without drawing. This is the correct
behavior: an unrepresentable physical coordinate produces no draw, not a corrupted draw.

The fix requires one line in `draw_cmd.rs`:

```rust
fn scale_coord(v: u32, sf: u8) -> u32 {
    v.saturating_mul(sf as u32)
}
```

Without this fix, the spec's guarantee that out-of-bounds coordinates are clipped to the
framebuffer boundary is broken for large coordinate values at 2x, producing silent display
corruption.

---

### B3: VYOMA_VDISP_SCREEN Backward-Compatibility Claim Is Incorrect

Section 7.4 extends the R19 `VYOMA_VDISP_SCREEN` message by appending a fourth field:

```
VYOMA_VDISP_SCREEN:<display_id>,<physical_w>,<physical_h>,<scale_factor>
```

The spec states this is "backward compatible with R19 parsers" because "apps that parsed
the R19 three-field format and ignore trailing fields continue to work correctly."

This claim is incorrect in a way that matters for real WASM apps. The R19 spec defined
`VYOMA_VDISP_SCREEN:<display_id>,<w>,<h>` and all existing apps that handle virtual
display assignment parse exactly three comma-separated fields. Rust's typical parsing
pattern is:

```rust
let parts: Vec<&str> = line.split(',').collect();
if parts.len() != 3 {
    return Err("unexpected field count");
}
```

A strict-length check (which is the correct defensive parsing style) will reject the
four-field R20 message as a parse error. The spec's claim that "apps that ignore trailing
fields continue to work" applies only to apps that parse with `parts.get(0)`,
`parts.get(1)`, `parts.get(2)` without a length check — which is a lax parsing pattern
that was never specified as required in R19.

The VyomaOS apps in the repo (`gui-demo`, `shell`) may or may not use strict length
checks. The spec cannot assume lax parsing behavior unless R19 explicitly required it, and
R19 did not.

The spec must either:

**Option A — Keep the four-field format and amend R19 app parsers**: Update the apps that
handle `VYOMA_VDISP_SCREEN` to accept 3 or 4 fields (≥3 is sufficient, defaulting
scale_factor to 1 when the fourth field is absent). Document this as a required change for
apps that were updated to R19.

**Option B — Define a new message type**: Introduce `VYOMA_VDISP_SCREEN_V2:
<display_id>,<physical_w>,<physical_h>,<scale_factor>` and send both the R19 message and
the R20 message simultaneously for apps that have `hidpi_aware=true`. This preserves the
R19 contract entirely.

Option A is simpler and should be preferred, but it requires explicitly updating the R19
app parsing code rather than claiming the change is backward-compatible when it is not.

The fix for existing R19 apps is straightforward: replace any strict-length check such as
`assert!(parts.len() == 3)` with `assert!(parts.len() >= 3)` and default the scale_factor
field to `1` when `parts.get(3)` returns `None`. The spec should include this exact
migration note so that the implementer knows what to change in existing app code. Without
it, the implementer will either not update the apps (leaving them broken at R20 runtime)
or will discover the mismatch during integration testing, adding unnecessary debug time.

---

### B4: App Cannot Learn Logical Window Dimensions Without Display Capability

Section 5.3 specifies that an app learns its logical dimensions via:

```
VYOMA_DISPLAY:window_info:<logical_w>,<logical_h>,<scale_factor>
```

and that "non-display apps do not receive this notification" (§15.4) because the supervisor
checks `app_state.capabilities.display`.

This creates a gap: apps that use `VYOMA_DRAW:` (which requires `display = true`) will
always receive `window_info` and can correctly size their Surface. But the spec's WIT
interface `vyoma:display-info@1.0.0` is also gated on `display = true` (§6.3). So far so
consistent.

However, the spec does not define what an app should do between the moment it spawns and
the moment it receives `VYOMA_DISPLAY:window_info`. There is a startup race:

1. Supervisor spawns app via Wasmtime.
2. App's `_start` begins executing immediately.
3. App may emit `VYOMA_DRAW:fill_rect:` commands before receiving `window_info`.
4. The supervisor's `window_info` send happens "at app spawn time" (§12.1), but the
   app's stdin read loop may not have executed yet.

The supervisor sends `window_info` to the app's stdin at spawn time. The WASM app reads
stdin in a line-by-line loop. If the app emits draw commands in the first few milliseconds
before its stdin read loop executes (e.g., a static UI that draws immediately in main),
those draw commands will be processed by the supervisor's draw dispatcher without the app
having seen its `window_info`. For a HiDPI-aware app, this means the first frame may be
drawn with incorrect assumptions about logical dimensions (whatever the app hardcoded or
defaulted to).

The spec must define the initialization ordering contract: either (a) the supervisor
guarantees that `VYOMA_DISPLAY:window_info` is delivered to the app's stdin before the app
can emit any `VYOMA_DRAW:` commands (which requires a synchronization mechanism that does
not currently exist); or (b) apps are responsible for issuing a `VYOMA_DISPLAY:query_info`
request before their first draw and blocking on the response (the query_info path is
defined in §7.3 but not specified as a required initialization step); or (c) the first
frame drawn before `window_info` is received is accepted with whatever coordinates the app
uses, and the app is expected to redraw when `window_info` arrives.

Option (c) is the simplest and most consistent with the existing `VYOMA_DRAW:flush`-based
rendering model. The spec should state this explicitly: apps that draw before receiving
`window_info` produce a provisional frame that may be mis-sized; apps that want correct
first-frame sizing should wait for `window_info` before their first flush. Without this
statement, implementers must guess the correct behavior.

The idiomatic VyomaOS app startup pattern for HiDPI-aware apps should be documented as:

```rust
// 1. Read stdin until VYOMA_DISPLAY:window_info is received
// 2. Parse logical_w, logical_h, scale_factor
// 3. Allocate Surface (physical = logical × scale_factor)
// 4. Draw first frame and flush
```

This two-line note added to §12.1 or §5.3 would resolve B4 entirely without requiring
any supervisor implementation change.

---

### B5: Font Large Specifier at 2x Overflows Window Bounds With No Defined Behavior

Section 4.3 states that at 2x, the `l` specifier produces a 32×64 physical pixel glyph
and that "this is intentionally large." This decision is correct in principle: on a 2x
display, the `l` specifier should be larger than at 1x.

However, the spec does not handle the geometry overflow case. At 2x, a line of text drawn
with the `l` specifier at `y=520` on a 960×540 logical screen produces a glyph at
`y_px = 1040` with `h_px = 64`. The bottom edge is at physical pixel `1104`, which exceeds
the 1080-row framebuffer. The out-of-bounds clipper (§15.1) will clip the glyph, cutting
off the bottom 24 rows of each character.

This is not a fatal correctness bug — the clipper handles it. But the combination of `l`
specifier + 2x + near-bottom-of-screen produces partially rendered glyphs with no warning
to the app. An app that lays out a bottom status bar using `l` text at `y = logical_h - 32`
(which would clear the bottom 32 logical points at 1x) will, at 2x, attempt to render at
`y_px = (logical_h - 32) * 2 = 1016` with a glyph height of 64px, overflowing to row 1080.

The spec must define one of the following:

**Option A — Document the overflow and specify that clipping is the correct behavior**:
Apps drawing near logical boundaries must account for scaled glyph size. An `l` glyph at
2x is 32 logical points tall (because `h_pts = 64 / 2 = 32`). An app using logical point
math correctly would not draw `l` text closer than 32 points from the logical bottom
edge.

**Option B — Clip at the logical-point level before scaling**: Before applying the scale
factor, check whether `y + glyph_h_pts > logical_h`. If so, clip the glyph to logical
bounds and then scale the clipped region. This prevents physical-pixel overflow by design.

Option A is simpler and follows the general principle that scaling is the app's
responsibility. But the spec must state the expected glyph height in logical points for
each size×scale combination so that apps can perform correct layout math:

| Specifier | 1x logical pts | 2x logical pts |
|-----------|---------------|---------------|
| `s` | 8 | 8 |
| `m` | 16 | 16 |
| `l` | 32 | 32 |

If the spec explicitly states that glyph logical point size is scale-invariant (a 16pt `m`
glyph is always 16 logical points tall regardless of scale factor), apps can safely lay
out text at `y = logical_h - 16` for `m` text without overflow at any scale. This is the
correct framing and the spec should state it.

The supervisor must also define how the Surface's `width` and `height` fields (stored in
`AppState`) are set for a legacy app. If they are set to the physical framebuffer
dimensions (1920×1080), the out-of-bounds clipper for the legacy app's draw commands will
never clip on a 960-wide draw command (960 < 1920), allowing the app to inadvertently
write to the right half of the Surface. If they are set to 960×540, a draw command at
x=900 with w=100 clips at x=960 as expected. The spec must define one and only one answer.

---

### Non-Blocking Issues

**NB1: `DisplayConfig` struct ownership is ambiguous.** Section 12.1 says `display_info.rs`
defines `DisplayConfig`, but §12.2 says `display.rs` imports it. The 500-line file limit
and the existing `display.rs` structure suggest `DisplayConfig` belongs in a shared types
module (e.g., `supervisor/src/display_types.rs`) that both `display.rs` and
`display_info.rs` import. Without specifying the ownership chain, the implementation risks
creating a circular import (`display_info.rs` depends on `display.rs` for framebuffer
state, and `display.rs` depends on `display_info.rs` for `DisplayConfig`). The spec should
specify which module owns the type and which imports it, and whether a separate types module
is introduced to break the potential import cycle.

**NB2: `query_info` round-trip latency is unspecified.** Section 7.3 defines
`VYOMA_DISPLAY:query_info` as an app → supervisor stdout command that triggers a
`window_info` response on stdin. The spec does not specify the supervisor's processing
latency guarantee. If the app emits `query_info` and then immediately reads stdin in a
blocking call, the response may not arrive before the read timeout (if any). The spec
should note that `query_info` is processed in the same supervisor IO loop pass that
processes draw commands, so latency is bounded by one IO loop cycle.

**NB3: Manifest schema version bump.** Adding `hidpi_aware` to `[capabilities]` changes
the manifest schema. The spec mentions updating `check-manifests` validation but does not
specify whether existing vyoma.toml files that lack the field are treated as
`hidpi_aware = false` (default) or as a validation error. The expected default-to-false
behavior should be stated explicitly in §13.1.

**NB4: `VYOMA_DRAW:version:3.1` is informational only, but supervisor should log it.**
Section 9.4 says the version line is "only for diagnostic logging in v1." The spec should
clarify that if a non-HiDPI-aware app (manifest `hidpi_aware = false`) emits
`version:3.1`, the supervisor ignores the hint and continues to use physical-pixel
semantics. Version line and manifest flag should be in sync; a mismatch should produce a
logged warning.

**NB5: The `detect_scale_from_edid()` stub placeholder should be typed correctly.** Section
18.3 mentions `detect_scale_from_edid() -> Option<u8>` in `display_info.rs`. The spec
should specify the stub returns `None` unconditionally and is not `#[cfg]` gated — it
should compile on all targets including `mcu-minimal` and `iot-edge` so that the scale
detection call site does not need conditional compilation guards.

**NB6: The Makefile DISPLAY_SCALE convenience variable (§15.5) needs boot.toml generation
plumbing.** The spec notes this as a "convenience addition" but boot.toml is currently
generated by `rootfs.sh` from static templates. Adding a Makefile variable that flows
through to boot.toml requires either a templating change to `rootfs.sh` or a sed
substitution step. The spec should either specify the rootfs.sh change required or
explicitly defer this to the implementer. Leaving it as an unexplained convenience note
creates an incomplete implementation path.

**NB7: The `display-changed` WIT event and `VYOMA_DISPLAY:display_changed:` stdin message
are both defined but their relationship is not specified.** For apps using the WIT
interface, the `display-changed` export is called by the host (supervisor). For apps using
the stdout protocol, the supervisor sends a stdin line. Are both sent for the same event?
Can an app register for only one? The spec should clarify that both notifications are sent
for the same event; apps using WIT receive the export call and apps using the stdout
protocol receive the stdin line. An app using both would receive both notifications for the
same display change event — the spec should state whether this double-notification is
expected (and apps should ignore the stdout line if they use WIT), or whether the
supervisor suppresses the stdout line when the app has a registered WIT handler.

**NB8: Unit test `test_font_pixel_doubling_2x` is underspecified.** Section 14.1 lists
this test as verifying "glyph rendered at 2x produces 2×2 pixel blocks." The test should
additionally verify that the total glyph bounding box area at 2x is exactly 4× the 1x
area (not just that individual pixels are doubled), and that the glyph's set bits are
spatially correct (i.e., the top-left 2×2 block corresponds to the glyph's first bit, not
an arbitrary arrangement). A test that only checks for "2×2 blocks" without verifying
spatial placement could pass for a buggy renderer that pixel-doubles in the wrong order.

---

## What the Spec Got Right

**Opt-in `hidpi_aware` flag is the correct backward-compatibility mechanism.** Forcing
all existing apps through a scale-factor multiplication would silently double their
coordinate values and break their layouts. The manifest flag creates a clean opt-in surface.
The macOS analogy (`NSScreen.backingScaleFactor` + `@2x` assets) validates this pattern at
scale.

**Scale-agnostic compositor is architecturally sound.** By requiring HiDPI-aware apps to
produce physical-pixel Surfaces, the compositor hot path (blit_surface) requires zero
changes. This preserves the R11 compositing performance model and avoids adding a scaling
pass in the vsync-critical path.

**Integer-only scale factors remove a category of complexity.** Fractional scaling requires
sub-pixel antialiasing decisions, font hinting changes, and rounding policy choices that
multiply the implementation surface area significantly. The v1 constraint of 1x or 2x
eliminates this entire class of problems. The spec's rationale (bitmap fonts are not
designed for non-integer scales) is technically accurate and well-stated.

**boot.toml override with validation is the right configuration model.** QEMU virtio-gpu
provides no EDID. A configuration surface is required for testing 2x mode in QEMU. The
`display_scale_factor = 2` field in boot.toml is consistent with the existing
configuration model and the validation (must be 1 or 2, fatal error otherwise) prevents
silent misconfiguration. The fatal-error-on-invalid-value approach is preferable to a
silent default here: if an operator types `display_scale_factor = 1.5` expecting fractional
scaling and gets silently downgraded to 1x, their UI will appear wrongly-scaled with no
explanation. The fatal error forces the issue and points to documentation.

**`VYOMA_DISPLAY:query_info` request path is a good addition.** The push notification
model (supervisor sends `window_info` at spawn) covers the common case. The pull query
covers apps that lose state or want to re-sync. Both paths together are sufficient for
the state-management needs of HiDPI-aware apps without requiring a new IPC mechanism.
The round-trip nature of `query_info` is also useful for testing: a test harness can send
`query_info` and assert that the supervisor returns the correct dimensions for a given
`DisplayConfig` without needing to observe a draw command side-effect.

**Platform matrix is explicit and consistent.** Specifying that MCU, IoT, and robotics
profiles are hardcoded to 1x (scale_factor ignored in boot.toml) prevents operators from
accidentally configuring a 2x scale on a display that cannot support it. The "log warning
and ignore" behavior for disallowed platforms is the correct non-fatal response. Defining
scale_factor as a compile-time constant for constrained platforms (rather than a runtime-
checked value) also eliminates the validation branch in performance-critical paths on
the `mcu-minimal` target where RAM is at 128KB and every conditional branch matters.

---

**The interaction matrix with previous rounds (§16) is the most complete of any round to
date.** Explicitly tracing which prior round each subsystem interaction touches (R11, R13,
R17, R18, R19) and stating which files are unchanged prevents implementation regression.
In earlier rounds (R12, R14), the interaction matrix was absent or partial, leading to
interface mismatches discovered during implementation. The explicit "No Changes Required"
list in §12.3 is particularly valuable: it tells the implementer which files to leave
alone, which is as useful as knowing which files to touch. The only gap (noted as NB7 in
the non-blocking issues) is the R15/R16 animation memory bandwidth implication, which
should be one bullet in §16 noting that R16's flush-rate guidance covers the 4x memory
bandwidth increase at 2x implicitly.

---

## Questions for the Architect

**Q1**: The spec defines logical window size as `physical_size / scale_factor` and states
that window sizes for virtual displays are also expressed as logical dimensions to HiDPI-
aware apps. When a virtual display is created with `vdisp create vd0 1920 1080 2`, is the
`1920 1080` specification in physical pixels or logical points? The spec implies physical
(§16.5), but the `vdisp create` command was defined in R19 without scale, and existing
users of that command will be confused if the same numerical argument now means different
things depending on whether a scale factor is appended. Should `vdisp create` accept
logical points (more ergonomic) or physical pixels (consistent with R19 and the Surface
model)? The distinction matters in tests: a test that creates `vdisp create vd0 960 540 2`
expecting a logical 960×540 display (physical 1920×1080) will produce a different virtual
display than one expecting a physical 960×540 display (logical 480×270). One convention
must be chosen and documented in the `vdisp` command reference.

**Q2**: Section 3.4 says legacy apps "occupy a fraction of the physical screen at the
density they were designed for." Does the compositor intentionally leave three-quarters of
the physical framebuffer undrawn by the legacy app (showing the desktop background through
the legacy app's window gap), or does some compositing-layer upscale path fill the gap?
If the compositor blits a 960×540 Surface at a 1920×1080 fb offset of (0,0), pixels
(960,0) to (1919,1079) are never written. Is this intentional (the legacy app has a
smaller window at 2x) or a visual artifact to be addressed in a future round?

**Q3**: The spec describes `VYOMA_DISPLAY:window_info` as sent "at app startup and on
any display resize or reassignment." VyomaOS does not currently support live window
resize (windows have fixed dimensions set by the compositor layout engine). Should the
`window_info` message be specified as an initialization-time only message for v1, with
"resize" deferred to a future windowing round? If resize is included in scope, what is
the trigger — a `vdisp resize` command, or a display resolution change? This matters
because if "resize" is in scope, apps must implement a redraw-on-resize handler; if it is
out of scope for v1, the app model is simpler (query once at startup, cache forever) and
the spec should state this explicitly to avoid implementers adding unnecessary event-loop
complexity for a path that will never fire in v1.

**Q4**: The pixel-doubling font rendering (§4.2) writes each set bit as a 2×2 block.
What happens to unset bits (background pixels) within the glyph bounding box? The spec
shows only the `if on` branch in the inner loop. Does the glyph render function also write
background-colored pixels for the unset bits (producing a solid glyph background), or does
it leave those pixels untouched (producing a transparent glyph that composites over
whatever is in the Surface)? The current font.rs behavior at 1x should be documented and
the 2x behavior should match it.

**Q5**: Section 9.3 states that for non-HiDPI-aware apps, "the scale factor is ignored and
all `VYOMA_DRAW:` coordinates are treated as physical pixels." This means the draw
commands from a non-hidpi-aware app write directly to the Surface at pixel coordinates.
But §15.2 (in the non-blocking issues) notes that the Surface size for a non-hidpi app
is ambiguous. Can the spec confirm: for a non-HiDPI-aware app on a 2x display, the
Surface is exactly `physical_w × physical_h` pixels, and the app's draw commands (in
physical pixels) are written directly to that Surface with no transformation? This would
mean the app's content occupies only the top-left quarter of the Surface if the app
draws up to its `window_info`-reported dimensions. Alternatively, if the legacy app's
Surface is `logical_w × logical_h` pixels, then `window_info` reports logical dimensions
and the app fills its entire Surface — but the compositor must then scale or letterbox the
Surface when blitting to the full physical framebuffer, which contradicts the scale-
agnostic compositor invariant. The spec must choose one model and state it in one sentence.

**Q6**: The spec's interaction matrix in §16 covers R11, R13, R17, R18, R19. It does not
address R15 (color management) or R16 (animation). Do animated apps (R16 apps that call
`flush` on a timer) need any special handling at 2x — specifically, does the flush rate
recommendation from R16 change at 2x (where Surface writes are 4x the memory of 1x)?
A 1920×1080 Surface at 60fps requires ~480MB/s of memory bandwidth for raw BGRA32 writes,
versus ~120MB/s at 1x. Is this acknowledged as a v1 constraint or expected to be handled
by R16's existing flush-rate guidance?

---

## Closing Assessment

Round 20 delivers a clean architectural foundation: the scale-agnostic compositor, the
opt-in `hidpi_aware` flag, the physical-pixel Surface model, and the integer-only scale
constraint together form a coherent and implementable system. The design correctly avoids
the trap of complicating the hot compositing path with scaling logic, and the macOS Retina
analogy is well-applied — apps supply 2x-density content rather than relying on automatic
upscaling, which is the reason Retina UI looks sharp where simple pixel-doubling does not.

The five blocking issues are not fundamental design problems — they are specification gaps
that will produce ambiguous implementation behavior if left unresolved. B1 (legacy app
Surface sizing) and B3 (VYOMA_VDISP_SCREEN backward compatibility) are the most urgent
because they affect existing apps and existing contracts from prior rounds. B2 (saturating
multiplication) is a one-line fix in `draw_cmd.rs` that prevents a class of silent display
corruption on large coordinate values. B4 (startup ordering) and B5 (font overflow into
logical bounds) each require one additional paragraph in the spec to produce an
implementable contract — neither requires architectural change.

Once the blocking issues are addressed and the two critical questions (Q1: virtual display
coordinate convention; Q2: legacy app window-fill behavior at 2x) are answered with a
single unambiguous sentence each, this spec is ready for implementation. The proposed file
delta is among the smallest of any display round: three modified files, one new file, no
compositor changes, no IPC changes, and a clean opt-in surface that leaves the ten
existing WASM apps unmodified and unaffected by the 2x path. This is a well-scoped round
whose main risk is the specification ambiguities identified above, not the implementation.
