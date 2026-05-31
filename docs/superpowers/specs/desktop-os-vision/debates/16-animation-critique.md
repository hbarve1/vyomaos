# Round 16 — Animation Engine: Critic Review
## VyomaOS Subsystem 16 — Animation Engine

**Date**: 2026-05-29  
**Reviewer**: Critic Agent  
**Subject**: `16-animation.md` — Animation Engine Architect Draft

---

## Verdict

This is a strong, detailed spec that correctly identifies the Core Animation two-layer
model as the right foundation for a WASM-first animation engine. The spring physics
coverage is more thorough than most OS-level specs bother to be, the security model
addresses real attack vectors, and the R11-R15 integration story is coherent. However,
there are five blocking issues that would cause production failures before the first
release, and ten non-blocking issues that will accumulate into real maintenance debt.
The spec should not be handed to an implementer until the blocking issues are resolved.

---

## Blocking Issues

### B1: The Stdout Protocol Creates a Compositing Pipeline Stall

The spec states: "The display parser thread reads stdout lines and sends AnimCommand
messages to the animation engine via a bounded crossbeam_channel::bounded(256) channel.
The animation engine processes commands in batch at the beginning of each vsync tick."

This architecture has a race condition that corrupts animation timing on every heavily
loaded frame.

The stdout parser thread and the vsync compositor thread are two different threads.
The vsync thread calls `tx_queue.drain_committed()` at tick T. Simultaneously, the
stdout parser thread is writing new transactions into the channel from a committed
`VYOMA_ANIM:commit` that arrived at tick T. Whether those transactions land in tick T
or tick T+1 depends on thread scheduling — a non-determinism that creates a one-frame
jitter (16.6ms at 60Hz) that manifests as a visible stutter on every animation start.

The spec does not define where the channel lock lives relative to `drain_committed()`.
If the stdout parser and the animation engine both hold references to `TransactionQueue`
via `Arc<Mutex<TransactionQueue>>`, the lock contention path is:

  stdout parser thread: lock TxQueue → push change → commit → release lock
  vsync thread (16.6ms boundary): lock TxQueue → drain_committed → release lock

On a loaded system where both threads reach the lock boundary simultaneously, one
thread parks. If the vsync thread parks waiting for the stdout parser, the vsync
deadline is missed. If the stdout parser parks waiting for the vsync thread's drain,
the buffered app stdout pipe backs up. Neither outcome is acceptable.

**Required fix**: Double-buffer the committed transaction queue. The stdout parser
writes to the "back" queue. The vsync thread atomically swaps front/back at tick start
via a single `AtomicPtr` swap. The vsync thread processes from the "front" queue it
just claimed exclusively. No lock needed during compositing. This is the same pattern
Core Animation uses with its render server transaction double-buffer.

### B2: Spring Physics Settle Time Estimate Is Wrong for Default UI Params

The spec's `settle_time_estimate` function returns `6.908 / (zeta * omega0)` for
underdamped springs. This formula computes the time to reach within 1/e^6.908 ≈ 0.1%
of target. That is correct for ζ = 1. For a typical UI spring with ζ = 0.7 and
omega0 = 20 rad/s (iOS-style "snappy"), this gives:

  settle_time = 6.908 / (0.7 * 20) = 0.493 seconds

But the spec then maps this `settle_time` onto the `duration_ms` the app declared in
the transaction. If the app said `duration_ms = 300` (300ms), the spring evaluator
runs `t_s = t * 0.493`, meaning at `t=1.0` the spring has only progressed 493ms of
simulation. The spring WILL reach its target within 300ms of real time, so the
animation ends correctly — but the app declared 300ms precisely because it wants a
300ms feel. The settle_time mapping distorts the subjective speed: the spring will
actually settle in ~150ms of wall time and then freeze at its final value for the
remaining 150ms because `advance()` returns `None` after `elapsed_ms >= duration_ms`.

The fundamental problem: for springs, `duration_ms` is semantically ambiguous. Does it
mean "stop evaluating the spring at this time" or "the spring should have settled by
this time"? Core Animation resolves this by not exposing `duration` for spring animations
at all — the spring settles when it settles, and the caller gets a callback. The spec
forces springs into the same `duration_ms` slot as cubic bezier animations, which is
wrong.

**Required fix**: For `AnimationTimingFunction::Spring`, `duration_ms` should be
treated as a maximum timeout, not a target duration. The animation terminates when the
spring settles within threshold (e.g. |x - target| < 0.001) OR when `duration_ms`
elapses, whichever comes first. The settlement check must happen in `advance()`, not
just via the elapsed counter. Document this explicitly in the WIT interface so app
developers understand why their 300ms spring might complete in 180ms.

### B3: NaN Sanitization in `sanitize()` Is Incomplete and the Compositor Can Still Crash

The spec's `Transform3D::sanitize()` replaces NaN/Inf with identity elements column by
column. The identity matrix has specific non-identity values at diagonal positions
(m[0][0]=1, m[1][1]=1, m[2][2]=1, m[3][3]=1) and zero elsewhere. The sanitize code
replaces `self.m[col][row]` with `identity.m[col][row]` if not finite. This is correct.

However, the compositor's `compute_screen_rect` extracts `layer.transform.m[3][0]` and
`layer.transform.m[3][1]` as translation components. An app can pass a matrix where
the diagonal is valid (no NaN) but the translation is ±f32::MAX (3.4e38). This passes
`is_finite()` — f32::MAX is finite. The downstream `fb.fill_rect_rgba(screen_rect, ...)`
then receives a rect with x = 3.4e38, triggering an integer overflow when converting
to pixel coordinates.

Further: the spec says "full matrix projection is handed off to R12 GPU path." In that
path, `layer_uniforms.transform` is passed directly to the GPU shader. A matrix with
f32::MAX translation values will generate clip-space coordinates that overflow the
rasterizer's fixed-point representation, causing implementation-defined GPU behavior.
The sanitize step does not protect against large-but-finite values.

**Required fix**: After sanitize, clamp translation components to the display bounds
plus a margin: `m[3][0].clamp(-65536.0, 65536.0)` and `m[3][1].clamp(-65536.0, 65536.0)`.
Clamp scale components `m[0][0]`, `m[1][1]` to `[-256.0, 256.0]` (256x scale is
already extreme for any UI use case). Document that the supervisor enforces these
clamp bounds regardless of the app's request.

### B4: Implicit Animation — The Spec Does Not Define When Implicit Animations Fire

The spec states: "when app commits a property change WITHOUT a transaction, supervisor
auto-animates with default duration (250ms, EaseInOut) if `implicit_animations_enabled`."

But the protocol for property changes WITHOUT a transaction is not defined. Looking at
the WIT interface: `set-position(layer, x, y)` — when called outside a transaction,
this enqueues a LayerChange into... what? There is no open transaction. The spec says
"apply immediately as an implicit animation if none is open" in the WIT comments, but
`apply_transaction()` in the engine only processes `AnimationTransaction` structs. There
is no code path that converts a bare `set-position` call outside a transaction into an
`AnimationTransaction` with `duration_ms=250`.

The stdout protocol has the same gap: `VYOMA_ANIM:set_pos:<l>,<x>,<y>` outside of a
`begin...commit` block — what happens? The spec says implicit animations, but the
protocol parser has no state machine to detect "we are outside a transaction." The
`TransactionQueue::push_change()` returns `Err("no open transaction")` for this case.
The spec says "or apply immediately as an implicit animation" but the code for this
path is entirely absent.

**Required fix**: In `wit_handlers.rs`, if `push_change()` returns `Err("no open
transaction")` AND `implicit_animations_enabled` is true, wrap the change in a new
`AnimationTransaction` with `duration_ms = IMPLICIT_DURATION_MS` and
`timing = EaseInOut`, then commit it immediately. Add this code path explicitly. Add a
test: `implicit_animation_default` (listed in the spec but not implemented).

### B5: `add-animation` Takes `list<u8>` msgpack — This Introduces an Unspecified Dependency

The spec says `add-animation: func(layer: u32, key: string, anim-data: list<u8>)` where
`anim-data` is a "msgpack-encoded AnimSpec." There is no msgpack crate listed in the
supervisor's dependencies. The CLAUDE.md repo has a 500-line file limit and no external
serialization in the supervisor today. Pulling in a msgpack crate adds a dependency
that must be audited for `no_std` compatibility (the supervisor is not `no_std` but is
musl-linked), must be vendored in the hermetic Docker build, and must be cross-compiled
for `x86_64-unknown-linux-musl`.

More critically: the msgpack `AnimSpec` schema is defined only in a comment, not in
machine-readable form. There is no WIT record type that maps to it, no validator, no
version field. When the schema evolves (e.g., adding a `spring_initial_velocity` field
in R17), apps compiled against the old schema will silently fail because msgpack is
schemaless from the decoder's perspective.

The stated reason for using `list<u8>` is to avoid "a new structured WIT type" — but
WIT already supports records and variants. The right solution is:

```wit
record keyframe {
  time: f32,
  value: anim-value,
  timing: option<timing-id>,
}

variant anim-value {
  float-val(f32),
  point-val(tuple<f32, f32>),
  rect-val(tuple<f32, f32, f32, f32>),
  color-val(tuple<f32, f32, f32, f32>),
}

add-animation: func(layer: u32, key: string,
                    property: u8, duration-ms: u32,
                    repeat-count: f32, autoreverse: bool,
                    fill-mode: u8, timing: u8,
                    keyframes: list<keyframe>) -> result<_, string>;
```

This is more verbose but type-safe, zero-dependency, and versionable via WIT interface
versions. The `list<u8>` approach trades type safety for nothing.

**Required fix**: Replace `add-animation(list<u8>)` with structured WIT types. Remove
the msgpack dependency entirely. The AnimSpec comment-schema should become a WIT record.

---

## Non-Blocking Issues

### N1: `LayerTree::flatten()` Z-Order Cache Is Not Granular Enough

The `z_order_dirty` flag is set on any structural change (create, drop, add_child) AND
on any `set-z-position`. But in a desktop with 10 apps each animating a
`ZPosition` property, `z_order_dirty` is set every frame and `flatten()` rebuilds from
scratch every vsync tick. The rebuild does a depth-first traversal of the entire tree
plus a `sort_by()`. With 512 layers × 10 apps = 5,120 layers, the sort is O(n log n)
where n = 5,120. At 60Hz that is ~307,200 sort steps/second. In practice this is fast
(sort on f32 pairs, likely <1ms), but it is wasteful.

Improvement: maintain a `BTreeMap<OrderedFloat<f32>, Vec<LayerId>>` that is updated
incrementally on z-position changes. Only the affected bucket needs resorting. The
`flatten()` then becomes an `O(n)` iterator over the BTreeMap rather than `O(n log n)`
sort. Implement this in v2 when profiling shows the sort appearing in vsync flame graphs.

### N2: Completion Callback — `VYOMA_ANIM_COMPLETE` for Group Animations Is Wrong

The spec says completion_token is per `AnimationTransaction`. But a transaction can
contain changes to N properties on M layers, resulting in N×M `RunningAnimation`
entries. Each `RunningAnimation` independently finishes (different layers may have
different timing). The spec attaches the token to the `AnimationEngine`'s completion
handler, but the code in `apply_transaction()` creates one `RunningAnimation` per
`LayerChange`, each with `completion_token = txn.completion_token`. This means the
`VYOMA_ANIM_COMPLETE:<token>` callback is sent once for each `LayerChange` in the
transaction, not once for the whole transaction.

If a transaction has 10 changes, the app receives 10 completion callbacks for one
`VYOMA_ANIM:commit`. This contradicts macOS CA semantics where the completion block
fires once when all animations in the transaction settle.

**Fix**: Track a `pending_count: u32` for each completion token. Decrement on each
`RunningAnimation` finish. Fire the callback only when `pending_count` reaches zero.

### N3: The Two-Layer Model Has No `hitTest` Equivalent

Core Animation's `CALayer.hitTest(point)` uses the presentation layer geometry (not
the model layer) to determine which layer the user touched. Without this, mouse and
touch input hit-testing against animating layers will be wrong — a button mid-slide
will be hittable at its destination (model) position, not its current animated position.

The spec integrates with R11's input routing but does not address how mouse events from
R11's `VYOMA_INPUT:mouse:` are routed when layers are animating. The `LayerHandleTable`
resolves handles, but there is no `hit_test(x: f32, y: f32) -> Option<LayerId>` function
defined anywhere.

**Required before v1 ship**: Add `hit-test: func(x: f32, y: f32) -> option<u32>` to
the WIT interface. The implementation must use `PresentationLayer` geometry, not model
geometry, to be correct during animation.

### N4: No `CAReplicatorLayer` Equivalent — Assess v1 Criticality

macOS `CAReplicatorLayer` creates N copies of its sublayers with per-copy transforms,
enabling particle trails, clock faces, loading spinners, and audio visualizers. The spec
explicitly omits it. This is fine for v1 but should be noted: any attempt to implement
a loading spinner in VyomaOS with this API will require N individual layers each
manually positioned. This is O(N) memory and CPU versus the O(1) replicator model. The
spec should include a forward-compat note: replicator semantics will be added in v2 as
a special `LayerKind::Replicator` variant in the `LayerContents` enum.

### N5: `CAShapeLayer` / Vector Path Layers Are Missing

The spec supports `background_color` (filled rect) and `border_width` (stroked rect) but
has no vector path primitive. There is no way to draw a rounded-rect with only two
rounded corners, a circle that animates its radius, or a custom bezier shape. On macOS
`CAShapeLayer` handles all of these. Without it, apps must pre-render shapes into a
Surface (R11) and upload as image content — which defeats the purpose of GPU-accelerated
layer compositing and prevents smooth `cornerRadius` animation on non-rectangular paths.

Assessment: `CAShapeLayer` is v1.5-critical (not day-one blocking, but required before
shipping a real UI toolkit). Add to the "Next" section.

### N6: `sublayer_transform` Is Declared But Never Used

The `Layer` struct declares `pub sublayer_transform: Transform3D` (the perspective
transform applied to all children). The compositor's `blit_layer()` and
`compute_screen_rect()` never read this field. The `interpolate.rs` module has no
handling for it in `PresentationLayer`. The `presentation.rs` `apply()` method has no
case for it.

This is dead code in the spec — a field that exists on the struct but is never set,
never applied, and never animated. Either implement it properly (children's transforms
are pre-multiplied by `sublayer_transform` before compositing) or remove it from the
struct and note it as a v2 feature.

### N7: `TransactionQueue::drain_committed()` Clears Per-App Counts Unconditionally

```rust
pub fn drain_committed(&mut self) -> impl Iterator<Item = AnimationTransaction> + '_ {
    let items: Vec<_> = self.committed.drain(..).collect();
    self.per_app_pending.clear(); // <-- this is wrong
    items.into_iter()
}
```

Clearing all per-app pending counts on every drain means: if App A has 14 pending
transactions (within the 16-transaction limit) and App B has 1, and the vsync tick drains
all 15, App A's count resets to 0. But App A may have submitted 2 more transactions
BETWEEN drain() and the next push_change() check — those 2 transactions were already
counted against the pre-drain limit of 14 but would be allowed past the post-drain limit
of 0+2=2. The race is narrow but exists when the stdout parser and drain() run concurrently.

**Fix**: Decrement per-app counts as transactions are drained, rather than clearing the
entire map. `per_app_pending.entry(txn.pid).and_modify(|c| *c = c.saturating_sub(1));`

### N8: Spring Physics Initial Velocity Semantics Are Underspecified

The `Spring { initial_velocity }` field represents velocity at time t=0. But the spec
does not define the units. Is it pixels/second? Fraction of target distance per second?
WebKit's spring uses "unit displacement per second" (i.e., normalized to the animation's
total displacement). iOS Core Animation uses points/second in display coordinates. These
differ by a factor of (display_scale × bounds_size), which means a spring with
`initial_velocity = 500.0` behaves radically differently for a 10-pixel move versus a
500-pixel move if units are px/s.

For gesture handoff (a drag releases into a spring animation inheriting the gesture's
velocity), incorrect units cause the animation to overshoot by orders of magnitude or
barely move. Document explicitly: units are "fraction of total animation displacement
per second" (the WebKit normalization), and provide a conversion helper:
`gesture_velocity_pixels_per_sec / (target - current)`.

### N9: No Per-Layer Dirty Tracking in `sync_static_layers()`

`sync_static_layers()` iterates ALL model layers on every tick to copy hidden/z_position/
contents to presentation for non-animated layers. With 5,120 layers and 60Hz, this is
307,200 field copies per second. Most are no-ops (the model hasn't changed). The
`PresentationLayer::dirty` flag is set by `apply()` for animated layers but is cleared
on every tick for static layers without checking whether the model actually changed.

Add a `model_dirty` flag to `ModelLayer` (set on any property write via the handle
table's `apply_change_to_model()`). `sync_static_layers()` skips the copy if
`!model_dirty`. This reduces the steady-state cost of the sync loop from O(all layers)
to O(changed-this-frame layers).

### N10: `LayerContents::GpuTexture(u32)` Is a Dangling Handle Risk

The `GpuTexture(u32)` variant holds a raw u32 that is a wgpu texture handle from R12.
If the app drops its wgpu texture via the R12 hostcall interface while the layer tree
still holds this handle, the compositor will attempt to blit a freed texture. The R12
spec uses `wgpu::Device` with ref-counting internally, but the exported handle is an
opaque u32 that does not participate in Rust's drop semantics.

Unlike `Surface(SurfaceId)` (which the supervisor owns via R11's surface allocator) and
`Image(ImageHandle)` (which R14 manages with reference counting), the GPU texture handle
has no supervisor-side lifetime management.

**Fix**: The GPU texture variant should carry a `GpuTextureRef` — a supervisor-side
Arc-wrapped ref to the wgpu texture. When an app drops its R12 texture handle, the
supervisor decrements the ref count. The layer tree holds the second ref. Only when
both are dropped is the wgpu texture actually freed. This mirrors how Metal's
`MTLTexture` works with `CALayer.contents`.

---

## What the Spec Got Right

**R1: The two-layer model is correctly implemented.** ModelLayer/PresentationLayer is
the correct architecture. Many homegrown animation engines get this wrong by mutating
the model directly during interpolation, causing flicker when the app reads back a
property while it is animating. This spec does it right.

**R2: Spring physics covers all three damping regimes.** The common mistake (seen even
in some Flutter internals) is to implement only the critical damping formula and call it
a spring. This spec provides under/over/critical solutions with the correct ζ-dependent
formula and a reasonable numerical clamp. The `initial_velocity` field for gesture
handoff is present and the normalization concern is raised in N8 (because the spec was
close but not complete — this is a critique, not a compliment to the spec for omitting
it, but the architecture accommodates the fix).

**R3: The `LayerHandleTable` isolation model is correct.** Per-app opaque handles that
do not alias across app PIDs is exactly the right security primitive. The implementation
is clean and reviewable.

**R4: The animation duration cap (30s) addresses a real DoS vector.** Many animation
engines set no upper bound on duration. A 10-year animation with infinite repeat would
consume CPU every vsync tick forever. The 30s cap with `validate_animation_spec()` is
applied before the animation enters the running queue.

**R5: `TransactionQueue` has both per-app and global caps.** Most queue implementations
have only one or the other. Per-app caps prevent a single misbehaving app from filling
the queue. The global cap prevents cooperative exhaustion from many apps simultaneously.
Both are needed.

**R6: Color interpolation is correctly routed through linear light (R15).** The
`srgb_to_linear()` conversion on ingestion in `wit_handlers.rs` means all interpolation
math operates in linear light, preventing the dark-band artifact that appears when
animating between saturated colors in gamma-compressed space. This is a detail that most
tutorial-level animation implementations miss.

**R7: The file map respects the 500-line rule.** 11 files, each purpose-bounded, with
clear responsibilities. The decomposition is sensible and mirrors how Core Animation's
QuartzCore headers are organized.

**R8: Completion callback backpressure is handled with non-blocking writes.** The spec
acknowledges the full-pipe case and drops the callback rather than blocking the vsync
thread. Logging the drop is correct. The app is responsible for draining stdin — this is
the right default.

**R9: `FillMode::Removed` is the default and `Forwards` must be explicit.** This matches
Core Animation semantics and prevents the common pitfall of animations permanently
overriding the model value. Apps that want "stick at end" must explicitly opt in.

**R10: The unit test list covers the key security and correctness invariants.** The listed
tests (`nan_in_transform`, `completion_callback_full_pipe`, `layer_handle_isolation`,
`layer_quota`) map directly to the blocking and non-blocking issues. If all listed tests
pass, the spec's stated security properties hold.

---

## Questions for the Architect

**Q1**: The spec says the compositor runs on the vsync thread. R12's GPU compositor also
runs on the vsync thread (it submits a wgpu command buffer). When `gpu_compositor = true`,
both `AnimationCompositor::composite_frame()` and `gpu_compositor.submit_layer_batch()`
are called on the same thread. Is there a risk of two wgpu submits per frame (one from
R12's existing GPU compositor, one from the animation path)? The R12 spec's compositor
already handles surface compositing. Does the animation compositor replace R12, delegate
to it, or is there a two-pass model (software for simple layers, GPU for complex)?

**Q2**: The `VYOMA_ANIM:create_layer` command expects the supervisor to reply
`VYOMA_ANIM_LAYER:<id>` on app stdin. But the stdout parser thread and the stdin writer
are on different threads. When a WASM app calls `create_layer` via the stdout protocol,
it must then block reading stdin to get the ID before it can call `set_pos` on the new
layer. This synchronous request/response over async pipes is fragile. Has the spec
considered making `create_layer` a WIT-only operation (where the return value is
synchronous) and removing it from the stdout protocol entirely? The stdout protocol
could be restricted to property mutations only.

**Q3**: `sync_static_layers()` copies `contents` from model to presentation for all
non-animated layers every tick. `LayerContents` is `Clone` — this clones a
`SmallString`-equivalent for `SurfaceId` and an `Arc`-equivalent for `ImageHandle`.
At 5,120 non-animated layers at 60Hz, this is 307,200 Arc increments/decrements per
second. Have you profiled whether this is acceptable, or should `contents` be a
`Cow<LayerContents>` that only clones on mutation?

**Q4**: The spec says `LayerTree::flatten()` is called from `composite_frame()`, which
runs on the vsync thread. `flatten()` takes `&mut self` (it mutates `z_order_cache`).
`AnimationEngine::tick()` also mutates `layer_tree` (via `presentation.get_mut()`). Both
are called sequentially within one tick, so there is no data race today. But if vsync
compositing is ever moved to a dedicated render thread separate from the animation
engine thread (a common optimization), `LayerTree` will need interior mutability or a
separate presentation snapshot. Is this future-proofing in scope?

**Q5**: The `repeat_count: f32` field allows `f32::INFINITY` for forever-looping
animations. `f32::INFINITY` serialized through the stdout protocol
(`VYOMA_ANIM:add_keyframe:...,<repeat>,...`) as the string "inf" — does the protocol
parser handle this? Rust's `f32::from_str("inf")` returns `Ok(f32::INFINITY)`, so the
parse would succeed, but a VyomaOS app written in C or AssemblyScript might emit the
literal string "-1" to mean infinite. The protocol spec should define the canonical
encoding for infinite repeat (e.g., the literal "inf" or the sentinel value -1.0).

---

## Closing Assessment

The architecture is sound. The two-layer model, spring physics breadth, handle isolation,
and R15 color integration are genuinely well-designed. The five blocking issues are all
fixable without structural redesign — they are gaps in the spec's synchronization story
(B1), spring duration semantics (B2), large-but-finite value attacks (B3), the implicit
animation code path (B4), and the msgpack dependency (B5). None require rethinking the
core data model.

The non-blocking issues cluster around three themes: completion semantics (N2, N7),
missing features that are not v1-blocking but will be requested in the first month
(N3 hit testing, N4 replicator, N5 shape layers), and implementation efficiency details
that matter at scale (N1 z-sort, N9 dirty tracking, N10 GPU texture lifetime).

Resolve B1–B5, add the hit_test WIT function (N3 is effectively blocking for any app
with animated interactive UI), and this spec is ready for implementation.

The comparison to macOS Core Animation is apt. CALayer shipped in Mac OS X 10.5 (2007)
and the fundamental two-layer model has not changed in 18 years. Getting this foundation
right in VyomaOS is worth the extra revision pass. The blocking issues identified here
are not fundamental design flaws — they are synchronization gaps and API ambiguities
that are cheaper to fix in a spec than in production code. A one-week revision cycle
should be sufficient to address all five blocking issues and produce an implementation-
ready spec.

The non-blocking issues (N1–N10) are catalogued here for the implementer's awareness,
not as pre-conditions for starting implementation. N2 (completion callback count), N3
(hit testing), and N10 (GPU texture lifetime) should be scheduled for the first sprint
after the initial implementation is running. N4 (replicator) and N5 (shape layer) are
features, not bugs, and belong in the v1.5 roadmap.

Priority order for the revision: B5 (eliminate the msgpack dependency before any code
is written), B4 (implicit animations are load-bearing for UI feel), B1 (deadlock
prevention), B2 (spring semantics documentation), B3 (sanitization completeness).

**Recommended status**: Revision required. Return to architect for B1–B5 + N3 fixes.

---

## Appendix: Fix Effort Summary and Risk Register

| Fix | Effort | Key change |
|-----|--------|-----------|
| B1: double-buffer TxQueue | 2h | Add back VecDeque, swap on tick start, lock only during swap (~100ns) |
| B2: spring duration semantics | 1h | Add settlement convergence check in advance(); document early completion |
| B3: clamp large-finite transforms | 30m | sanitize_for_compositor() with platform-configurable clamp constants |
| B4: implicit animation code path | 3h | begin_implicit() in wit_handlers + protocol; bypasses per_app_pending cap |
| B5: replace msgpack with WIT records | 4h | Define WIT record keyframe; remove msgpack dep; zero callers to migrate |

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Spring CPU at max layers exceeds vsync budget | Medium | High | Add spring_budget_ms cap in AnimationConfig; profile before ship |
| App leaks layers (never calls drop_layer) | High | Medium | GC all app layers on app exit in LayerHandleTable destructor |
| msgpack schema drift if B5 not fixed | High | Medium | B5 fix eliminates this risk entirely |
| GPU texture dangling handle (N10) | Low | Critical | Fix N10 before enabling GpuTexture variant in any build |
| Completion callback storm on bulk transactions | Low | Medium | N2 pending_count fix limits to 1 callback per transaction handle |
<!-- end of review -->
