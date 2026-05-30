# Critique: Menu Bar & System Chrome (Round 23)

## Verdict

The spec achieves its core goal cleanly: system chrome is a privileged WASM app that lives
in space 0, draws with the standard `VYOMA_DRAW:` protocol, and communicates with the
supervisor and other apps through a well-defined line-oriented protocol. The decision to
implement chrome as a WASM app rather than a compiled-in supervisor module is the correct
call — it preserves the principle that all UI is rendered by apps, keeps the supervisor free
of rendering logic, and means chrome is independently testable and replaceable without
recompiling the supervisor. The Z-order defence-in-depth design (both a compositor blit-last
guarantee AND a y-clip in `blit_surface`) is sound and provides redundant protection against
buggy apps leaking pixels into the menu bar region. The 64-message startup queue for routing
`VYOMA_CHROME:` events during chrome's Launching state is a practical and complete solution
to the most obvious race.

However, five blocking issues prevent safe implementation in this form: the startup fallback
for the top 24 pixels before chrome reaches Running state is undefined, creating a visual
hole that user-facing apps will expose on every boot; the routing of `VYOMA_CHROME:` lines
from apps when chrome is Suspended or Terminated drains into the startup queue indefinitely
with no backpressure or discard policy; the y-clip in `blit_surface` is specified at the
destination pixel level but the current compositor operates in logical-point space and
applies `scale_factor` inside `blit_surface` — the clip must be stated in physical pixels
with an explicit code site; the consent modal's input lock requires a supervisor-side state
field (`input_locked_to`) that is not defined anywhere in the supervisor's data model and
is not threadsafe given that keyboard events arrive on the TTY input thread; and the 1-second
tick mechanism is underspecified with respect to what happens to `last_chrome_tick` if the
main event loop is blocked for more than 1 second, which can cause tick burst delivery on
unblock. None of these require architectural changes. They require precision additions to
§2, §4.3, §10.2, §7.5, and §5.4 respectively.

---

## Blocking Issues

### B1: Chrome Startup Race — Top 24px Is Undefined Before Chrome Reaches Running

Section 2.1 states that the 64-message startup queue ensures menu_set and focus_changed
events are not lost while chrome is in Launching state. This is correct for protocol
events. But the spec does not define what the compositor renders in the top 24 physical
pixel rows before chrome's Surface exists and has been flushed.

The sequence on boot is:

```
t=0   Supervisor starts; compositor begins vsync flush loop
t=50  Chrome Wasmtime process spawned (LifecycleState: Launching)
t=150 Chrome WASM initialises, reads its stdin
t=200 Chrome draws menu bar, calls VYOMA_DRAW:flush
t=200 Chrome Surface appears in compositor; first correct frame blitted
```

During t=0 to t=200, the chrome app has no Surface registered in the compositor. The
compositor's flush pass iterates over `apps_sorted` (the Z-sorted snapshot). Chrome is not
in `apps_sorted` yet. The blit-last guarantee cannot apply because there is nothing to blit.
Meanwhile, other apps (gui-demo, calculator) may already be Running with surfaces that
overlap y=0. The y-clip in `blit_surface` is supposed to prevent them from drawing in the
top 24px. But the spec's description of the y-clip in §10.2 says "if app_name !=
chrome_app_name, clip dest_y_min = max(dest_y_min, CHROME_HEIGHT_PX)." This clip IS
applied regardless of chrome's lifecycle state — it is a static per-blit rule. So the top
24px rows would be unrendered: no app can draw there, and chrome has not drawn there yet.
The result is undefined framebuffer content — whatever happened to be in the fbdev buffer
from a prior frame or from kernel initialisation. On real hardware this produces a visible
visual glitch lasting 100–300ms on every boot.

The fix requires a **supervisor-owned fallback strip**: a static 24px-tall filled rectangle
that the compositor blits at `y=0` unconditionally when no chrome Surface is registered.

Implementation: In `compositor.rs`, before the `apps_sorted` blit loop, check if a
chrome Surface is registered. If not, blit a hardcoded 24pt × logical_w solid fill at
y=0 in colour `0x1E1E2EFF` (the menu bar background) directly from the supervisor. This
fallback is a raw `memset`-style fill on the physical framebuffer, not a Surface. It is
replaced automatically as soon as chrome's first flush registers its Surface and the blit-
last ordering takes over. The supervisor keeps a `chrome_surface_ready: bool` flag
(set to `true` on chrome's first `VYOMA_DRAW:flush`, cleared on chrome exit) to decide
which path to take.

This change must be added to §2 (Chrome App Architecture) and §10 (Security and Clipping)
of the spec. The fallback fill is not a security concern because it is produced by the
supervisor, not by an app.

---

### B2: Chrome Suspended or Terminated — Unbounded Queue Growth

Section 4.3 defines a 64-message bounded queue for routing `VYOMA_CHROME:` lines when
chrome is not yet Running. The queue drains when chrome transitions to Running. This covers
the Launching → Running transition correctly.

But the spec does not define what happens when chrome is in `Suspended` or `Terminated`
state (R22 lifecycle states). Both states are reachable:

- **Suspended**: if a space-switch somehow moves chrome out of the active space (unlikely
  given space=0 always-on-top semantics, but §10 does not explicitly forbid it and R21's
  `wm::compact_z_order` does not have a guard for chrome).
- **Terminated**: chrome crashes. R22 restart=always would restart it, but there is a
  window between crash and restart where chrome is Terminated.

During this window, the `chrome_router.rs` routing code checks "if chrome is not in
Running state, queue the message." But the queue was sized for the startup race (typically
~10 messages). If chrome is Suspended for 30 seconds (unlikely but legal), and each app
sends a `menu_set` on every focus change, the queue fills in seconds. The spec says "bounded
capacity: 64 messages" but does not define the overflow policy: does the queue drop oldest
messages (ring buffer), drop newest (discard), or block the writing thread?

If the queue blocks the writing thread, the app whose `VYOMA_CHROME:` line triggered the
queue-full condition has its entire stdout processing path blocked. This stalls the app's
display output (since stdout is processed sequentially in the supervisor's output-reading
thread for that app). An app that calls `menu_set` and then immediately calls `VYOMA_DRAW:`
would find its draw commands blocked behind a full chrome queue.

The fix is two-part:

**Part A (overflow policy)**: The queue must use a drop-oldest ring buffer policy. When the
64-message capacity is reached, the oldest queued message is discarded and the new message
is appended. This is correct because the most recent `menu_set` supersedes all older ones,
and the most recent tick supersedes older ticks. No message type in the `VYOMA_CHROME:`
protocol requires in-order reliable delivery of all historical messages. The spec must
explicitly state: "overflow policy: drop-oldest. The last-writer-wins property of menu_set
and tick makes this safe."

**Part B (Terminated state)**: When chrome is in `Terminated` state and the restart policy
has exhausted its retries (or restart=never), the queue must be disabled entirely. All
`VYOMA_CHROME:` lines are discarded. The spec must add: "if chrome is Terminated and no
restart is pending, the router discards all VYOMA_CHROME: lines immediately without
queuing." The spec must also explicitly address the Suspended case for space=0 apps: add a
note to §10 that chrome's win_space=0 means the WM's space-switch logic must never
transition chrome to Suspended. An assertion in the space-switch handler should verify this.

---

### B3: Y-Clip Must Be Stated in Physical Pixels with an Explicit Code Site

Section 10.2 states:

> if app_name != chrome_app_name { dest_y_min = max(dest_y_min, CHROME_HEIGHT_PX) }

The pseudo-code is correct in intent but the spec uses `CHROME_HEIGHT_PX` without
specifying where this value is computed or how it interacts with the existing `blit_surface`
call signature.

Examining the R11 and R17 display architecture: `blit_surface` in
`supervisor/src/display/compositor.rs` operates on physical pixel coordinates. The Surface
buffer is in physical pixels (allocated as `width_px × height_px`). The `win_x` and `win_y`
fields in `AppState` are in logical points. The `flush_pass` function converts them to
physical pixels by multiplying by `scale_factor` before calling `blit_surface`. Therefore:

- `win_y_px = (win_y_pts * scale_factor).round() as i32`
- The clip threshold is `CHROME_HEIGHT_PX = (24.0 * scale_factor).round() as u32`

The clip must be applied in the `flush_pass` function, NOT inside `blit_surface`, because
`blit_surface` does not have access to `scale_factor` or the chrome app name. The correct
implementation is:

```rust
// In flush_pass, after computing win_y_px and before calling blit_surface:
let effective_dest_y = if app_name == chrome_app_name {
    win_y_px
} else {
    win_y_px.max(chrome_height_px as i32)
};
// Also clip the source row range to skip rows that would blit above chrome:
let src_y_offset = (effective_dest_y - win_y_px).max(0) as u32;
blit_surface(fb, &surface.buffer, dest_x_px, effective_dest_y,
             surface.width_px, surface.height_px - src_y_offset, src_y_offset);
```

Without the `src_y_offset` correction, the clip would shift the app's surface content
upward (blitting rows 0..N of the surface at y=24 instead of the rows that actually
correspond to y=24 in logical space). The spec does not describe the src_y_offset
adjustment, which is a critical missing detail that will cause display corruption if
omitted. The implementation would blit the app's topmost rows (e.g. its title bar) at
y=24 physical pixels, which is visually wrong.

The spec must add a code sample to §10.2 showing both `effective_dest_y` and `src_y_offset`
calculations, and must state clearly: "the y-clip is applied in `flush_pass`, not in
`blit_surface`; `blit_surface` takes `src_y_offset` to skip source rows above the clip."

---

### B4: Consent Input Lock Is Not Threadsafe — TTY Thread Has No Access to `input_locked_to`

Section 7.5 introduces `input_locked_to: Option<String>` as a field the supervisor uses to
redirect all keyboard input to the chrome app during a consent modal. The field is checked
by "the supervisor's keyboard router." In the existing VyomaOS architecture, keyboard input
arrives on the **TTY input thread** — a dedicated OS thread that reads `/dev/tty` in raw
mode and dispatches `VYOMA_INPUT:key:` events to the focused app's stdin.

The `input_locked_to` field would live in a shared data structure (likely alongside
`AppState` in the `apps_map` `RwLock<HashMap>`). The TTY input thread must read this field
on every keypress to decide where to route the event. Simultaneously, the main event loop
thread writes it when processing `@supervisor: input_lock_chrome` from chrome's stdout.

The spec does not specify:
1. What type holds `input_locked_to` — is it a field on a new global struct, a field on the
   supervisor's main state, or an `Arc<Mutex<Option<String>>>`?
2. Whether acquiring the `apps_map` read lock in the TTY thread on every keypress creates
   contention with the main event loop, which holds a write lock during app spawning and
   during the flush pass.
3. What happens if chrome issues `input_lock_chrome`, then crashes before issuing
   `input_unlock_chrome`. The lock is never released. Keyboard input is permanently
   redirected to a dead app. All other apps are effectively frozen from an input perspective.

The fix is to use an independent `Arc<AtomicBool>` for the input lock (not a field in
`apps_map`) to avoid RwLock contention:

```rust
// In supervisor global state:
chrome_input_locked: Arc<AtomicBool>,  // true = all kbd → chrome

// In TTY input thread:
let target = if chrome_input_locked.load(Ordering::Acquire) {
    "chrome".to_string()
} else {
    focused_app.load() // existing focus tracking
};
route_keyboard_event(target, key_event);
```

For the crash-without-unlock scenario: the supervisor's chrome lifecycle watcher must call
`chrome_input_locked.store(false, Ordering::Release)` whenever chrome transitions away from
`Running` state (Suspended, Terminating, Terminated). This ensures the lock is always
released on chrome exit regardless of whether chrome issued `input_unlock_chrome`.

The spec must add this data model (§7.5 and §13.2) and must document the auto-unlock on
chrome state transition.

---

### B5: Clock Tick Burst on Event Loop Unblock

Section 5.4 specifies the 1-second tick mechanism as:

> compare monotonic timestamp against last tick emission time on each event loop iteration;
> if elapsed >= 1000ms, write tick and update `last_chrome_tick`

This is correct for the steady-state case. But the supervisor's main event loop is not
guaranteed to execute once per millisecond. It runs on each vsync tick (~16ms at 60fps)
but can be delayed by slow app output processing. If one app emits a large burst of
`VYOMA_DRAW:` lines in a single tick, the event loop thread may spend 200ms parsing and
processing those lines before returning to the tick check.

More seriously: if the event loop is blocked for 3 seconds (e.g. a deadlock-adjacent
condition, a slow disk write on the 9P mount, or heavy app output), then on unblock:
- `elapsed since last_chrome_tick = 3.0 seconds`
- The spec's code emits ONE tick (it updates `last_chrome_tick` after emission).

That single emission is correct. But the spec does not state that only ONE tick is emitted
per event loop iteration regardless of elapsed time. An alternative (and incorrect)
implementation would loop until `elapsed < 1000ms`, emitting multiple ticks. This would
deliver 3 ticks in rapid succession to chrome's stdin, causing three rapid clock redraws
with the same timestamp (since all three are computed from `SystemTime::now()` within the
same event loop iteration).

Additionally, if `last_chrome_tick` is initialised to `SystemTime::UNIX_EPOCH` (the zero
value), the first event loop iteration would compute `elapsed = 56 years` and emit one
tick immediately. This is harmless in practice but produces a clock update at an
unpredictable time before chrome is ready (before its first stdin read), potentially
overflowing the startup queue with redundant ticks before chrome has drawn its initial state.

The fix requires two clarifications in §5.4:

**Clarification A (single emission per loop iteration)**: Add to the code comment: "At most
one tick is emitted per event loop iteration. If `elapsed >= 2000ms`, the excess is
absorbed: `last_chrome_tick` is set to `now`, not `last_chrome_tick + 1000ms`. This
prevents burst delivery."

**Clarification B (initialisation)**: `last_chrome_tick` must be initialised to
`SystemTime::now()` at supervisor startup (not `UNIX_EPOCH`), so the first tick is emitted
approximately 1 second after boot, not immediately.

---

## Non-Blocking Issues

**N1: `resize_surface` command is introduced without specifying its parser site.**
Section 9.4 adds `VYOMA_DRAW:resize_surface:<w>,<h>` as a new draw command. The existing
`VYOMA_DRAW:` parser lives in `supervisor/src/draw_cmd.rs`. The spec should note that
`resize_surface` is added to the `parse_draw_cmd` function in that file, alongside
`fill_rect`, `draw_text`, etc. Without this note, implementers may add it to the wrong
parser or handle it in the wrong thread context.

**N2: `menu_set` item label character restrictions are insufficient.**
Section 4.2 forbids `|`, `\n`, `:`, and non-ASCII characters. But it does not restrict
leading/trailing whitespace. An app sending `VYOMA_CHROME:menu_set: File | Edit ` would
produce menu items with padding spaces, causing misaligned text in the fixed-width slots.
Chrome should trim whitespace from each item label after splitting on `|`.

**N3: The `wit/chrome.wit` file is specified without a WIT package structure.**
Section 8 shows `package vyoma:chrome@1.0.0` but does not show the `world` declaration
required by WIT. Without a `world`, the WIT file cannot be processed by `wit-bindgen` even
for documentation validation. This does not affect R23 (bindings are not generated) but
will block R32. Add a minimal `world chrome-world { import system-chrome; }` declaration.

**N4: The 72pt banner reservation in §11 is advisory only, but the tiling WM still places
windows at y=24.** This means when R73 is implemented and the tiling origin moves to y=72,
all existing app layouts will shift down by 48pt. Apps that hardcode their own y-coordinates
(e.g. gui-demo draws at `y=28`) will have a visible jump. R73 should be documented as a
known layout-breaking change at the time of §11.1.

**N5: The display name mapping in §6.4 is hardcoded in `apps/chrome/src/main.rs`.**
This is explicitly acknowledged as a v1 limitation. No issue with the approach for R23.
Just confirm that the mapping is in `state.rs` (alongside `ChromeState`) rather than
`main.rs` to avoid `main.rs` growing toward the 500-line limit as more apps are added.

**N6: `input_lock_chrome` and `input_unlock_chrome` are shell-capability commands routed
via `@supervisor:`.** Section 7.5 says the supervisor validates that only the chrome app may
issue these. But the IPC handler for `@supervisor:` messages does not currently have access
to the sender's app name in a typed way — it processes the line from chrome's stdout, which
has already been stripped of the `@supervisor:` prefix. The supervisor needs to know which
app sent the command. This is already true for chrome's `consent_*` responses and is not a
new problem introduced by input_lock, but the spec should reference §9.3 explicitly and
note that the IPC handler for `@supervisor:` commands must be aware of the sender (which the
current design supports since each app's stdout is read on a per-app thread).

**N7: The spec introduces `VYOMA_DRAW:resize_surface` in §9.4 without specifying whether it
is idempotent.** If chrome issues `resize_surface:960,24` when its surface is already 960×24
physical pixels, should the supervisor skip reallocation or always reallocate? Reallocation
involves a surface buffer free + alloc + compositor re-registration under the vsync_lock —
a non-trivial operation. The spec should state: "if the requested dimensions match the
current surface dimensions exactly, resize_surface is a no-op." This prevents chrome from
triggering unnecessary reallocations by calling `resize_surface` on every flush.

**N8: Section 13.1 places `consent.rs` inside `apps/chrome/src/` but the consent modal
requires `VYOMA_DRAW:resize_surface`, which is a new supervisor-side draw command not yet
in any app's known command set.** An app developer reading `consent.rs` for reference would
not know that `resize_surface` requires a supervisor change to function. The file header of
`consent.rs` should include a comment: `// Requires supervisor support for VYOMA_DRAW:resize_surface (R23 §9.4).`
This makes the cross-layer dependency explicit at the call site rather than only in the spec.

**N9: The platform matrix in §12 specifies "simplified chrome (status bar at bottom)" for
mobile, but the y-clip enforcement in §10.2 always clips at `y < CHROME_HEIGHT_PX` (top of
screen).** On mobile, where the status bar is at the bottom, the y-clip rule must be
inverted: clip `y > logical_h - STATUS_HEIGHT_PX`. The spec must add a note to §10.2 and
§12.1 that the clip direction is determined by the platform profile's `chrome_position`
field (`top` for desktop, `bottom` for mobile), and that `CHROME_HEIGHT_PX` is reinterpreted
as a bottom-clip threshold on mobile. Without this note, the mobile compositor will clip the
wrong region.

**N10: The `ChromeState` struct in §13.1 stores `menu_items: HashMap` but the key type is
unspecified.** Is it `HashMap<String, Vec<String>>` (app name → item list) or
`HashMap<String, String>` (app name → raw pipe-delimited string)? Given that chrome only
ever displays the focused app's items, a `HashMap<String, Vec<String>>` is the natural
choice, but the implementation could differ. The spec should specify the type in the
`state.rs` description to avoid ambiguity when two engineers implement `state.rs` and
`menu_bar.rs` concurrently.

---

## Got Right

**The chrome-as-WASM-app decision is architecturally sound.** Implementing chrome as a
privileged WASM app rather than a supervisor module keeps the supervisor free of rendering
logic, allows chrome to be hot-reloaded without recompiling the supervisor, and makes
chrome independently testable with `cargo test --target wasm32-wasip2`. The approach is
consistent with VyomaOS's capability-secure philosophy: even the OS UI is just another app,
constrained by its declared capabilities. Critically, it also means that a chrome rendering
bug cannot crash the supervisor — chrome crashes are isolated to the chrome Wasmtime process
and the supervisor continues to run the rest of the system normally.

**The Z-order + y-clip defence-in-depth is exactly right.** Using two independent
mechanisms (compositor blit-last and `blit_surface` y-clip) means neither mechanism is a
single point of failure. A compositor ordering bug does not expose the menu bar region to
app pixels; a clip implementation bug still gets overwritten by chrome's surface. This is
the correct pattern for a trusted display region. The spec explicitly names both mechanisms
and explains why neither alone is sufficient — that level of clarity prevents future
developers from removing the "redundant" clip thinking the Z-order guarantee is enough.

**The 64-message bounded startup queue handles the Launching race cleanly.** The decision
to queue events and drain on transition to Running is better than the alternative (blocking
the supervisor's routing thread until chrome is ready), and better than discarding events
silently. The cap of 64 prevents unbounded memory growth during chrome startup. The drain
ordering (arrival order, not priority) is correct because the most important invariant is
that chrome's first view of focus state and menu items matches the state at the moment it
becomes Running — and FIFO delivery of the queued events achieves this.

**The `input_lock_chrome` / `input_unlock_chrome` protocol correctly recognises that WASM
apps cannot intercept input directly.** Rather than pretending WASM can do something it
cannot (intercept OS-level keyboard routing), the spec defines a round-trip signal through
the supervisor. This is the right abstraction: chrome declares intent (lock input) and the
supervisor enforces it at the OS level. The auto-unlock on chrome state change (once added
per B4's fix) closes the safety loop and makes the consent modal a well-behaved, non-
leaking use of supervisor privilege.

**The `VYOMA_CHROME_FROM:<sender>:` prefix injection correctly prevents injection attacks.**
Section 10.4's reasoning is sound: supervisor-generated events are written directly to
chrome's stdin pipe (not through the stdout-interception code path), so an app cannot
craft a bare `VYOMA_CHROME:focus_changed:` line and have it appear to chrome as a
supervisor event. The prefix distinction is enforced structurally, not by content-filtering.
This is a meaningful security property: it means chrome does not need to validate the
authenticity of events by signature or nonce — the structural routing guarantee is sufficient.

**The `VYOMA_CHROME:` line stripping before display processing (§4.3, I4) is correct.**
The spec states that `VYOMA_CHROME:` lines are stripped from the normal stdout pipeline
before reaching the display subsystem. This prevents two failure modes: the display
subsystem attempting to parse `VYOMA_CHROME:menu_set:File` as a `VYOMA_DRAW:` command
(which would be a benign parse error but would spam the log), and the serial console
printing raw protocol lines to the developer console (which would be confusing during
debugging). The stripping happens in `chrome_router.rs` before both paths, which is the
correct interception point.

**The platform matrix (§12) correctly differentiates chrome variants without creating
separate chrome WASM binaries.** A single chrome binary handles both desktop and mobile
layouts by reading the platform profile context from a startup event. Headless platforms
simply do not include chrome in `boot.toml` at all — the supervisor's chrome router
discards `VYOMA_CHROME:` lines silently when no chrome app is registered (I8). This means
app code that issues `menu_set` is fully portable across platforms without conditional
compilation or capability checks. The fallback-to-discard is the right semantics for a
headless server that happens to run an app originally written for desktop.

---

## Questions for the Architect

1. **Chrome restart policy**: If chrome crashes, the menu bar disappears. Section 15 says
   "tentative: restart=always with 500ms backoff." Should this be a required restart policy
   (enforced by supervisor regardless of the `vyoma.toml` restart field) or a recommended
   default? A crashed chrome that does not restart leaves the system without consent UI,
   which is a security-relevant condition. The supervisor could enforce `restart = "always"`
   unconditionally for any app with `chrome = true`, overriding the manifest field. This
   would be a precedent for supervisor-overriding manifest declarations, which has broader
   design implications.

2. **Focus and chrome**: Section 6.1 says `focus_changed` is sent when the user issues
   `focus <app>`. Can the user focus the chrome app itself? If so, what does the left
   section show? Section 6.4 says chrome "cannot focus itself" but does not specify what
   happens if `focus chrome` is issued — is it rejected, or silently redirected to the last
   non-chrome focused app? Given that keyboard shortcuts for R32 menu navigation may require
   chrome to be "focused" in a functional sense, the spec should define whether focus on
   chrome is a valid state and if so what the WM does with it.

3. **Scale factor change at runtime**: `VYOMA_CHROME:display_resized:` covers screen size
   changes. But does it also signal a scale_factor change (e.g. the user connects a HiDPI
   monitor to a non-HiDPI system)? Chrome needs to know both logical dimensions and
   scale_factor to compute its physical surface size correctly. Should `display_resized`
   carry the new scale_factor as a third field (`VYOMA_CHROME:display_resized:<w>,<h>,<sf>`)?
   Without the scale_factor, chrome cannot correctly size its `resize_surface` request after
   a monitor change.

4. **Chrome and the watchdog**: Chrome has `stdio = true` and therefore presumably
   `watchdog_secs` can be set. If chrome draws its initial menu bar in under 30 seconds
   but then goes quiet (no events → no redraws → no stdout), does the watchdog kill it?
   Chrome should declare `watchdog_secs = 0` in its `vyoma.toml` to opt out. This should
   be called out explicitly in §13.1 as a required field value, not left as an implicit
   expectation that the chrome developer knows to add.

5. **`resize_surface` and the surface allocator**: The command implies the supervisor must
   re-allocate the Surface buffer at runtime. The R11 Surface allocator creates a
   fixed-size buffer at spawn time. Does `resize_surface` require a new allocator API, or
   can it be implemented as "allocate a new buffer, swap it in under the vsync_lock, free
   the old one"? The latter needs careful handling to avoid use-after-free in the compositor
   flush pass if it holds a reference to the old buffer across the resize. The spec should
   specify the synchronisation primitive used: the vsync_lock write-lock must be held for
   the duration of the swap, and the old buffer must not be freed until the flush pass
   holding the previous reference has completed.

---

## Closing Assessment

Round 23 is a well-structured spec with a clear architecture and correct security
reasoning. The chrome-as-WASM-app pattern is original and consistent with VyomaOS's design
principles. The implementation sequence in §16 is particularly valuable: it stages the work
into seven verifiable steps with explicit test checkpoints, which is the right approach for
a feature that touches the compositor, the IPC broker, and the keyboard routing path
simultaneously. The decision to defer `resize_surface` to Step 7 (the last step) is correct
because it prevents the consent modal's unimplemented command from blocking early menu bar
testing.

The blocking issues are all precision gaps, not structural flaws. B1 needs a two-line
supervisor fallback fill and a `chrome_surface_ready` flag; B2 needs a drop-oldest overflow
policy and a Terminated discard rule with an explicit assertion that space=0 apps cannot be
Suspended; B3 needs `src_y_offset` added to the clip code sample with a clear note that the
clip belongs in `flush_pass` not `blit_surface`; B4 needs an `Arc<AtomicBool>` instead of
an RwLock field plus auto-unlock in the chrome lifecycle watcher; B5 needs single-emission-
per-iteration semantics and a `SystemTime::now()` initialisation for `last_chrome_tick`.

After these five fixes are applied to the spec, and the five open questions (chrome restart
policy, focus-on-chrome semantics, display_resized scale_factor field, watchdog_secs=0
requirement, resize_surface synchronisation) receive definitive answers, R23 is ready for
implementation planning. Estimated implementation cost is 3–4 days of focused Rust
development across the chrome WASM app and three supervisor modules.

The non-blocking issues (N1–N10) are all small enough to be resolved inline during
implementation without a spec revision cycle. N3 (WIT world declaration) and N9 (mobile
clip direction) are the two most likely to cause confusion if left undocumented; recommend
adding one-sentence notes to the spec for each before implementation begins.

The questions section reveals one latent design hole that the architect should resolve
before implementation: the `display_resized` event format. If scale_factor can change at
runtime, the current two-field format `<w>,<h>` is insufficient and adding a third field
later is a protocol breaking change. Given that the cost of adding a third field now is
zero (chrome ignores unknown trailing fields if parsing is done greedily), the architect
should make that decision pre-implementation rather than retrofitting it during R45
(multi-monitor support) when the pressure to ship is higher.

Overall quality: strong. Ready for FINAL after blocking issue resolutions.
