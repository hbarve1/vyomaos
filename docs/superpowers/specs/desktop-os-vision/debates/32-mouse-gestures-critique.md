# Critique: Mouse, Trackpad & Gestures (Round 32)

**Critiquing**: `32-mouse-gestures.md`
**Critic role**: blocking issues only — correctness, threading safety, integration coherence

---

## CRITIQUE — 5 Blocking Issues

---

### B1: Lock-Ordering Violation in §10.2 — `route_mouse` Holds `APPS_MAP` Implicitly When Writing to App stdin via `deliver_to_app`

**Problem**:

§10.2 lists the global lock order as:

```
APPS_MAP → MouseEventQueue → GestureState → LAST_HOVER_TARGET → CURSOR_X/Y → app stdin write
```

And the prose claims: *"mouse-dispatch never holds APPS_MAP across a write to
an app stdin pipe; hit_test takes the lock, copies what it needs into a local
snapshot, and releases it before delivery."*

But in §6, `route_mouse` does this:

```rust
let captured = MOUSE_DRAG_CAPTURE.load().as_ref().cloned();
let target = if let Some(c) = captured { Some(c) } else {
    let am = APPS_MAP.clone();
    hit_test(ev.x, ev.y, &am)
};
// ...
synth_enter_leave_if_needed(&target);
// ...
if let Some(t) = target {
    if is_space0_chrome(&t) {
        deliver_to_chrome(&t, ev);
    } else {
        deliver_to_app(&t, ev);
    }
}
```

`deliver_to_app` must look up the app's stdin pipe to write to it. The stdin
pipe lives in `apps_map.get(name).stdin_writer` (the standard pattern from R21
and R28). So `deliver_to_app` *will* re-lock `APPS_MAP` to fetch the
`stdin_writer`, holding the lock for the duration of `write_all`.

Worse, `is_space0_chrome(&t)` likely also reads `APPS_MAP` to check
`state.space == 0`. So the sequence becomes:

1. lock `APPS_MAP`, copy snapshot, unlock (in `hit_test`)
2. lock `APPS_MAP`, read `state.space`, unlock (in `is_space0_chrome`)
3. lock `APPS_MAP`, read `stdin_writer`, **hold lock during write**, unlock (in
   `deliver_to_app`)

Step 3 violates the stated invariant. More dangerously, the `write_all` on an
app's stdin pipe can block if the app's stdin buffer is full (e.g., the app
is unresponsive or stopped). The compositor thread reads `APPS_MAP` once per
frame (R11's flush pass) to enumerate windows. If `mouse-dispatch` is blocked
writing to a stuck app while holding `APPS_MAP`, the compositor stalls — and
because the compositor also signals frame completion to other subsystems,
the whole UI freezes.

The chained re-locking also creates a TOCTOU window: between step 1 and step
3, the target app could exit and be removed from `APPS_MAP`. `deliver_to_app`
would then panic on `apps_map[name]` lookup or write to a closed pipe.

**Proposed fix**:

Inside `hit_test`, return not just the app name but also a *cloned*
`stdin_writer: Arc<Mutex<...>>` and the `is_space0` flag, both extracted under
the single `APPS_MAP` lock. This collapses steps 1–3 into one lock
acquisition. Then `deliver_to_app` operates on the already-cloned `Arc`,
never touching `APPS_MAP` again:

```rust
struct HitResult {
    name:        String,
    stdin:       Arc<Mutex<ChildStdin>>,
    is_space0:   bool,
    wit_inbox:   Option<Arc<Mutex<VecDeque<MouseEvent>>>>,
}

pub fn hit_test(x: i32, y: i32, apps_map: &AppsMap) -> Option<HitResult> {
    let m = apps_map.lock().unwrap();
    // ... walk z-sorted snapshot ...
    let state = m.get(&hit_name)?;
    Some(HitResult {
        name:      hit_name.clone(),
        stdin:     state.stdin_writer.clone(),
        is_space0: state.space == 0,
        wit_inbox: state.wit_inbox.clone(),
    })
}

fn deliver_to_app(hit: &HitResult, ev: &MouseEvent) {
    if let Some(inbox) = &hit.wit_inbox {
        inbox.lock().unwrap().push_back(*ev);
        return;
    }
    let line = format_vyoma_mouse(ev, /* window-local coords */);
    // stdin lock is fine — it's per-app, not the global apps_map.
    let _ = hit.stdin.lock().unwrap().write_all(line.as_bytes());
}
```

Then update §10.2's lock order to: `APPS_MAP → per-app stdin lock` (acyclic;
two locks, one acquired briefly, one held over write). Drop `APPS_MAP` from
`deliver_to_app`'s lock set entirely.

Also add a `try_lock` with timeout on the per-app stdin write (e.g., 5 ms)
so a stuck app cannot starve `mouse-dispatch`; on timeout, drop the event
and log a warning.

---

### B2: Hit-Test Snapshot is Stale Across `synth_enter_leave_if_needed` — Window Movement Mid-Dispatch Causes Spurious Enter/Leave Storms

**Problem**:

§6 calls `hit_test` once per `MouseEvent`. The result `target` is then passed
to `synth_enter_leave_if_needed`, which compares against `LAST_HOVER_TARGET`
and emits Enter/Leave. The invariant in §16 #5 says: *"An Enter event for app
A is never followed by an Enter for app A without an intervening Leave."*

This invariant is violated under the following sequence:

1. User holds mouse still at coordinate (500, 500). App A's window covers
   that point. `LAST_HOVER_TARGET = Some("A")`.
2. Window manager (R21, separate thread) animates app A's window to a new
   position — e.g., user drags the title bar via R28 keyboard shortcut, or
   Stage Manager (R26) animates A off-screen via `anim_x_override`.
3. A `Move` event arrives. `hit_test` runs with the *new* window position; A
   no longer covers (500, 500), so `target = None` (or `target = Some("B")`).
4. `synth_enter_leave_if_needed` emits `Leave` for A, possibly `Enter` for B.
5. A's animation finishes; on the next event, A is back covering (500, 500)
   (if the animation was transient). `Enter` for A is emitted again.

So far so good. But the failure mode is when the *animation is in progress*
during dispatch:

1. `mouse-dispatch` takes `APPS_MAP` lock at frame T to compute hit_test, sees
   A at `effective_x = 500` (mid-animation). Result: `target = Some("A")`.
2. At frame T+1ms, window manager writes a new `anim_x_override` for A
   (mid-animation step).
3. `mouse-dispatch` releases `APPS_MAP`, calls `synth_enter_leave_if_needed`,
   then `deliver_to_app("A", ev)`. So far consistent with frame T.
4. **But** §6 step (d) — `synth_enter_leave_if_needed` — and step (e) —
   `deliver_to_app` — observe `APPS_MAP` mutations between them. If
   `synth_enter_leave_if_needed` re-reads any window state (e.g., to format
   window-local coordinates), it sees the *new* `anim_x_override`. The
   `Enter` event payload now contains `(x - new_anim_x, y - win_y)`, which
   may be negative or out of bounds. The app receives `enter: -50, 30` and
   correctly concludes the cursor is outside its bounds — emitting a
   spurious **second** Leave one frame later when the next Move arrives with
   the same logic.

Even more severely: during a rapid Stage Manager strip animation (R26),
windows move by ≥ 30 px per compositor frame (60 Hz). At 500 Hz mouse poll
rate (§15), `mouse-dispatch` will fire ~8 events between window moves. Each
event re-runs `hit_test` against a freshly-updated `anim_x_override`,
producing alternating Enter/Leave/Enter/Leave for each frame of animation
as the cursor straddles the animating window edge. The app's stdin floods
with thousands of synthetic enter/leave events during a 250 ms animation.

**Proposed fix**:

Two changes:

1. **Use one consistent snapshot of `APPS_MAP` for the full event dispatch.**
   In `route_mouse`, take the lock once, snapshot the entire set of relevant
   window geometries into a local `Vec<(name, x, y, w, h, z, space)>`, and
   use that snapshot for both `hit_test` *and* the window-local coordinate
   conversion in `deliver_to_app`. This way the event is dispatched against
   a single coherent view.

2. **Suppress Enter/Leave during animation.** When `INPUT_LOCK_LEVEL ==
   LL_STAGE_SWITCH` or `LL_MC` or `LL_FS_TRANSITION`, the hover target is
   pinned to whatever it was at lock-level-entry time. No new Enter/Leave
   events are emitted while the lock level is non-zero. On return to
   `LL_NONE`, `mouse-dispatch` synthesises a fresh hit_test and emits at
   most one Leave (for the old target) + one Enter (for the new target) to
   reconcile.

   Concretely, in `synth_enter_leave_if_needed`:

```rust
fn synth_enter_leave_if_needed(target: &Option<String>) {
    let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
    if lock != LL_NONE {
        return;   // hover target is frozen during transitions
    }
    let prev = LAST_HOVER_TARGET.load();
    if prev.as_ref().as_ref() == target.as_ref() { return; }
    // ... existing emit logic ...
}

// On INPUT_LOCK_LEVEL transition back to LL_NONE (in unlock_input):
fn on_input_unlock() {
    let target = hit_test_now();
    synth_enter_leave_if_needed(&target);
}
```

This also resolves the case where the window animates *through* the cursor
position: no events fire mid-animation.

---

### B3: Gesture Interruption (§7.7) Does Not Handle Mid-Gesture `mouse-dispatch` Aborts — In-Flight Events Still Get Delivered

**Problem**:

§7.7 says when `INPUT_LOCK_LEVEL` transitions to non-zero mid-gesture,
`end_gesture` is invoked with synthetic end semantics and *"no further events
from the still-touching fingers are delivered until all slots release."*

The mechanism is supposed to be: `trackpad-evdev` thread holds `GestureState`,
notices `INPUT_LOCK_LEVEL != LL_NONE`, calls `end_gesture`, and stops emitting.

But there are two independent races that break this:

**Race 1: Already-queued events in `MouseEventQueue`.**
At the moment lock level rises (say, MC activates via Ctrl+Up Arrow), the
`MouseEventQueue` may already contain 5–10 pending `MouseEvent`s and gesture
events produced by `feed_frame` over the past several ms. `mouse-dispatch`
drains the queue and processes each event. Section 6's `route_to_chrome_only`
routes them to `mc`, but **gesture events** (e.g., `Pinch`, `ScrollEnd`) are
not strings; they are `MouseEvent` variants delivered via the line protocol
into the same target. So a half-finished pinch gets emitted to MC, which
doesn't understand it and either ignores it (silent failure) or worse —
parses `pinch_end` as a malformed `mouse:` line and crashes its event loop.

**Race 2: `trackpad-evdev` reads `INPUT_LOCK_LEVEL` once per `SYN_REPORT`.**
A pinch in progress emits a `Pinch` event every `SYN_REPORT` (~5 ms apart).
If MC activates between two SYN_REPORTs, the next `feed_frame` call sees the
new lock level and emits `PinchEnd` (good), then sets `in_progress = None`.
But the previous `Pinch` event (with the old lock level) is still in the
queue ahead of the `PinchEnd`. `mouse-dispatch` will deliver both, *but* it
delivers the in-flight `Pinch` to whoever the previous target was (the
focused app), and the `PinchEnd` to MC (because lock level is now `LL_MC`).
The focused app receives a `Pinch` with no matching `PinchEnd`, violating
invariant §16 #7.

Furthermore: §7.7 says *"no further events from the still-touching fingers
are delivered until all slots release"*. The implementation needs to track
"fingers that were down at lock-level-rise time" and ignore them until
their tracking_ids release. But §7.1's `GestureState` has no such field; it
only tracks `last_slots` and `in_progress`. Adding the field is necessary.

**Proposed fix**:

Add an explicit `aborted_slot_ids: SmallVec<[i32; 10]>` to `GestureState`
that records the tracking IDs of all fingers that were active when the
gesture was aborted:

```rust
pub struct GestureState {
    last_slots:        [TouchSlot; MAX_SLOTS],
    in_progress:       Option<GestureKind>,
    aborted_ids:       Vec<i32>,   // tracking_ids to suppress
    scroll_origin:     (i32, i32),
    // ...
}

impl GestureState {
    pub fn feed_frame(&mut self, slots: &[TouchSlot; MAX_SLOTS], mq: &MouseEventQueue) {
        let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
        if lock != LL_NONE && self.in_progress.is_some() {
            // Abort: emit synthetic end, mark all current finger ids as aborted.
            let end_ev = self.synthesize_end();
            mq.lock().unwrap().push(end_ev);
            for s in slots.iter().filter(|s| s.active) {
                self.aborted_ids.push(s.tracking_id);
            }
            self.in_progress = None;
        }
        // Filter out aborted fingers before any processing.
        let active: Vec<&TouchSlot> = slots.iter()
            .filter(|s| s.active && !self.aborted_ids.contains(&s.tracking_id))
            .collect();
        // Garbage-collect aborted_ids when those fingers release.
        self.aborted_ids.retain(|id|
            slots.iter().any(|s| s.active && s.tracking_id == *id));
        // ... existing dispatch ...
    }
}
```

To fix Race 1, `mouse-dispatch::route_mouse` must check whether the event
was generated before the lock-level rise. The cheap fix: tag every
`MouseEvent` with a `seq: u64` and an `epoch: u8` (incremented by the
supervisor every time `INPUT_LOCK_LEVEL` changes). On dispatch, discard any
event whose epoch != current epoch *if* the event is a gesture-class event.
Move and click events are still delivered (their lock-level routing is
already correct via `route_to_chrome_only`); only mid-gesture stragglers are
dropped:

```rust
pub fn route_mouse(ev: &MouseEvent) {
    let cur_epoch = INPUT_EPOCH.load(Ordering::Acquire);
    if ev.epoch != cur_epoch && matches!(ev.kind, MouseKind::GesturePinch | MouseKind::GestureScroll) {
        return;
    }
    // ... rest ...
}
```

Where `INPUT_EPOCH: AtomicU8` is bumped in the same critical section that
writes `INPUT_LOCK_LEVEL`.

---

### B4: Cursor Compositing Race in §10.3 is Wrong About Hit-Test Safety — Drag Capture Path Bypasses the Argument

**Problem**:

§10.3 argues that the `CURSOR_X`/`CURSOR_Y` torn-read race is safe because
*"hit-testing is done in mouse-dispatch using the local ev.x, ev.y, never via
the global atomics."*

This is true for the hover path, where `hit_test` uses `ev.x, ev.y`. But the
**drag capture** path bypasses hit_test entirely (§5.2):

> "While capture is set, all subsequent Move/Release/Scroll events are
> delivered to the captured app **regardless of hit test**, until the
> corresponding button release frame is dispatched."

So during a drag, the app receives `Move` events with `ev.x, ev.y`. The app
itself reads `CURSOR_X` / `CURSOR_Y` via `vyoma:pointer::get-position` to
correlate with its internal drag state — e.g., a drawing app needs the
current cursor position even when dispatched events have been coalesced or
delayed. The torn read manifests *in the app's logic*, producing a visible
glitch (e.g., a brush stroke that jumps one pixel orthogonally).

More importantly, §11's `get-position` WIT host function presumably reads
both atomics:

```rust
fn get_position() -> (i32, i32) {
    (CURSOR_X.load(Ordering::Relaxed),
     CURSOR_Y.load(Ordering::Relaxed))
}
```

These two loads are not atomic with respect to each other. Worse, they are
both `Relaxed`, which on a weakly-ordered architecture (aarch64 — see
`PLATFORM=iot-edge`, `mobile`, `robotics-rt`, `server-headless`) can be
reordered to observe `Y_old, X_new` even when the writer wrote in the order
`X then Y`. The "1 px tear" rationalisation assumes sequential consistency
and falls apart on ARM64.

The proposed software cursor compositor read in §4.2 has the same problem:

```rust
let cx = CURSOR_X.load(Ordering::Relaxed);
let cy = CURSOR_Y.load(Ordering::Relaxed);
```

On ARM64, the compositor can read a position pair that **never existed**
(combining the X from one update with the Y from a much later update, if
the loads are reordered relative to each other and to other memory
operations). This produces a cursor that visually warps across the screen
intermittently — a real, user-visible bug.

**Proposed fix**:

Pack X and Y into a single `AtomicU64` (or `AtomicI64`):

```rust
// supervisor/src/cursor.rs

pub static CURSOR_POS: AtomicU64 = AtomicU64::new(0);

#[inline]
pub fn cursor_xy() -> (i32, i32) {
    let v = CURSOR_POS.load(Ordering::Acquire);
    let x = (v as u32) as i32;
    let y = ((v >> 32) as u32) as i32;
    (x, y)
}

#[inline]
pub fn set_cursor_xy(x: i32, y: i32) {
    let v = ((y as u32 as u64) << 32) | (x as u32 as u64);
    CURSOR_POS.store(v, Ordering::Release);
}
```

This eliminates the torn-read problem entirely, on every architecture, with
identical performance (a single atomic load is no slower than two; on
x86_64 it's faster because it's one memory op).

Update `vyoma:pointer::get-position` and the compositor cursor draw to use
`cursor_xy()`. Also update `route_mouse` step (a) and `update_cursor_position`
to use `set_cursor_xy`. §10.3's "tear is acceptable" rationale should be
deleted; the new invariant is "cursor position is always read consistently".

---

### B5: `route_mouse` Drag Capture Latches On `Press` Even When Lock-Level Blocks Delivery — Phantom Drags Stick After MC Dismissal

**Problem**:

In §6, the dispatch pipeline does:

```rust
// (c) Drag capture takes precedence over hit test.
let captured = MOUSE_DRAG_CAPTURE.load().as_ref().cloned();
let target = if let Some(c) = captured { Some(c) } else {
    let am = APPS_MAP.clone();
    hit_test(ev.x, ev.y, &am)
};
// (e) Deliver.
if let Some(t) = target {
    if is_space0_chrome(&t) { deliver_to_chrome(&t, ev); }
    else                    { deliver_to_app(&t, ev); }
}
// (f) If this is a Press on a non-captured target, set capture.
if ev.kind == MouseKind::Press && captured.is_none() {
    MOUSE_DRAG_CAPTURE.store(Arc::new(target));
}
```

But the **lock-level gate** in step (b) runs *before* this and short-circuits
when `INPUT_LOCK_LEVEL != LL_NONE`:

```rust
let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
if lock == LL_MC || lock == LL_STAGE_SWITCH || lock == LL_FS_TRANSITION {
    route_to_chrome_only(ev);
    return;     // <-- early return; step (f) never runs
}
```

This *itself* is fine in isolation, but it interacts catastrophically with
the inverse case: `Release` events that arrive *during* a lock level.

Concrete failure scenario:
1. User left-clicks on app A. Lock level is `LL_NONE`. `route_mouse` runs to
   completion. `MOUSE_DRAG_CAPTURE = Some("A")`.
2. User begins dragging.
3. Mid-drag, user invokes Mission Control via four-finger swipe-up. The
   gesture recognizer detects SwipeFour, sets `INPUT_LOCK_LEVEL = LL_MC`,
   spawns `mc` app.
4. The *next* mouse event is a `Move`. Step (b) returns via
   `route_to_chrome_only` → `mc` receives the Move. App A receives nothing.
5. User releases the mouse button while still in Mission Control. `Release`
   event arrives. Step (b) again returns via `route_to_chrome_only`. App A
   **never receives the Release**. `MOUSE_DRAG_CAPTURE` is **never cleared**
   because step (f) never runs.
6. User dismisses Mission Control. `INPUT_LOCK_LEVEL = LL_NONE`.
7. User moves mouse over app B. `route_mouse` runs:

```rust
let captured = MOUSE_DRAG_CAPTURE.load().as_ref().cloned();
// captured = Some("A")  -- STALE!
let target = captured;
// Delivers Move to A even though cursor is over B and no button is pressed
deliver_to_app("A", ev);
```

App A receives Move events forever (until the next physical button press),
believing it is still in a drag. Worse, app A's button state model is now
permanently incorrect — it never saw the Release for the original Press, so
it thinks Left is still held.

**Proposed fix**:

Two complementary fixes:

1. **Always run drag-capture release logic on lock-level rise.** When
   `INPUT_LOCK_LEVEL` rises from `LL_NONE` to anything else, synthesize a
   final `Release` event for the captured target with the current button
   mask, deliver it to the captured app, then clear `MOUSE_DRAG_CAPTURE`.
   This matches macOS NSEvent behaviour where activating Mission Control
   forcibly ends any in-progress mouse drag in the app under the cursor.

```rust
// Called from the same code that writes INPUT_LOCK_LEVEL.
fn on_input_lock_rise() {
    if let Some(captured) = MOUSE_DRAG_CAPTURE.load().as_ref().clone() {
        let (cx, cy) = cursor_xy();
        let synth = MouseEvent {
            kind:        MouseKind::Release,
            x: cx, y: cy, dx: 0, dy: 0,
            button:      MouseButton::Left,
            button_mask: ButtonMask::empty(),
            scroll_dx: 0, scroll_dy: 0,
            modifiers:   mod_state_snapshot(),
            source:      MouseSource::Synthetic,
            epoch:       INPUT_EPOCH.load(Ordering::Acquire),
        };
        deliver_to_app_by_name(&captured, &synth);
        MOUSE_DRAG_CAPTURE.store(Arc::new(None));
    }
}
```

2. **Belt and braces: also clear capture on `LL_NONE` re-entry if the held
   buttons differ from the captured snapshot's button mask.** This catches
   any other path that ends a drag (e.g., the user releases the mouse over
   the MC overlay before MC is dismissed, but MC swallows the release for
   its own UI):

```rust
fn on_input_lock_fall() {
    let captured = MOUSE_DRAG_CAPTURE.load();
    if captured.is_some() {
        // If no buttons are currently held by the physical mouse, clear capture.
        if CURRENT_BUTTONS.load(Ordering::Acquire) == 0 {
            MOUSE_DRAG_CAPTURE.store(Arc::new(None));
        }
    }
}
```

Where `CURRENT_BUTTONS: AtomicU8` is maintained by `mouse-evdev` from the
SYN_REPORT button-mask aggregation (a field that should be added; currently
the spec only tracks buttons inside per-frame locals).

Update invariant §16 #6 to read: *"Every `Press` is paired with exactly one
`Release`, possibly synthetic when a lock-level rise interrupts the drag."*

Update invariant §16 #10 to also require: *"`MOUSE_DRAG_CAPTURE` is cleared
on `INPUT_LOCK_LEVEL` rise (with a synthetic Release delivered to the
captured app)."*
