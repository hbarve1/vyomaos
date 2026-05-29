# Critique: Touch & Stylus Input (Round 33)

**Critiquing**: `33-touch-stylus.md`
**Critic role**: blocking issues only — correctness, threading safety, integration coherence

---

## CRITIQUE — 5 Blocking Issues

---

### B1: `flush_touch_frame` Never Emits `Ended` Events — Tracking-ID-Release Logic Broken

**Problem**:

In §2.2, `TouchSlot::end_tracking` sets `active = false`. In `flush_touch_frame` (§2.1),
the processing loop checks `if slot.active` to emit events. Slots with `active = false` are
skipped entirely. But the `Ended` phase must be emitted for slots whose `tracking_id` was
just set to -1 in this SYN_REPORT frame — i.e., slots where `active` transitioned from
`true` to `false` since the last flush.

The current code has no `just_ended` flag (only `just_began`), so `Ended` events are never
emitted. Apps receive `Began` and `Moved` events but never `Ended` or `Cancelled`. Touch
sequences never terminate from the app's perspective, violating invariant §6.4 #1:
"Every `began:<id>` is followed by exactly one `ended:<id>` or `cancelled:<id>`."

Additionally, `just_began` is set in `begin_tracking` but never cleared after the first
flush — so every `Moved` event on a slot will have `just_began = true` until a new
tracking assignment, emitting spurious repeated `Began` events for a held touch.

**Proposed fix**:

Add a `just_ended: bool` field to `TouchSlot`, mirroring `just_began`:

```rust
struct TouchSlot {
    active:      bool,
    just_began:  bool,
    just_ended:  bool,  // set by end_tracking, cleared after flush
    tracking_id: i32,
    // ...
}

impl TouchSlot {
    fn begin_tracking(&mut self, id: i32) {
        *self = TouchSlot { active: true, just_began: true, tracking_id: id, ..Default::default() };
    }
    fn end_tracking(&mut self) {
        self.active = false;
        self.just_ended = true;
    }
}
```

In `flush_touch_frame`:

```rust
for slot in slots.iter_mut() {
    if slot.just_ended {
        events.push(TouchEvent { phase: TouchPhase::Ended, touch_id: slot.tracking_id, ... });
        slot.just_ended = false;
    } else if slot.active {
        let phase = if slot.just_began { TouchPhase::Began } else { TouchPhase::Moved };
        events.push(TouchEvent { phase, ... });
        slot.just_began = false;  // clear after first emission
    }
}
```

Note: `flush_touch_frame` receives `&mut` slots to allow clearing `just_began`/`just_ended`.
Update the function signature: `fn flush_touch_frame(slots: &mut [TouchSlot; MAX_TOUCH_SLOTS], ...)`.

---

### B2: Stylus Has No `Began`/`Ended` Phase — `BTN_TOUCH` Events Not Mapped for Stylus

**Problem**:

The stylus path in §2.1 only emits `TouchPhase::Moved` events. Stylus contact with the
screen surface should emit `Began` when the pen tip touches down and `Ended` when it lifts.
On Linux, stylus tip contact is reported via `BTN_TOUCH` (`value=1` = tip down,
`value=0` = tip lifted) on the same evdev device.

The current code maps `BTN_TOUCH` to `slots[0].btn_touch` (§2.1), which is the single-touch
type-A path for finger input. For stylus, `BTN_TOUCH` has different semantics. More
importantly, there is no code that converts changes in `stylus.active` (or
`StylusState.btn_touch`) to `Began`/`Ended` phases in `flush_touch_frame`.

Apps that use stylus pressure data to drive drawing expect `Began` (pen-down) and `Ended`
(pen-lift) events to demarcate each stroke. Without these phases, apps cannot know when a
stroke starts or ends and will draw a continuous smear across all strokes.

**Proposed fix**:

Add `btn_touch: bool` and `prev_btn_touch: bool` to `StylusState`:

```rust
struct StylusState {
    // ...
    btn_touch:      bool,
    prev_btn_touch: bool,  // state from last flush
    active:         bool,  // true while in-contact or hovering
}
```

In `EV_KEY` handling:
```rust
BTN_TOUCH => {
    if stylus.tool != StylusTool::None {
        stylus.btn_touch = value != 0;
    } else {
        slots[0].btn_touch = value != 0;
    }
}
```

In `flush_touch_frame` stylus section:

```rust
if stylus.tool != StylusTool::None {
    let phase = match (stylus.prev_btn_touch, stylus.btn_touch) {
        (false, true)  => TouchPhase::Began,
        (true,  true)  => TouchPhase::Moved,
        (true,  false) => TouchPhase::Ended,
        (false, false) => {
            // Hovering: only emit if tilt/position changed significantly
            if stylus_hover_changed(stylus) { TouchPhase::Moved } else { return; }
        }
    };
    events.push(TouchEvent { phase, touch_id: STYLUS_TOUCH_ID, ... });
    stylus.prev_btn_touch = stylus.btn_touch;
}
```

Where `STYLUS_TOUCH_ID` is a reserved sentinel (e.g., `i32::MIN`) that does not conflict
with multi-touch tracking IDs.

---

### B3: `hit_test_touch` Is Not Defined — `route_touch` Calls It But §5.2 Says "Reuses hit_test from R32" Without Specifying How Stylus Is Hit-Tested

**Problem**:

Section §5.2 states "`hit_test_touch` reuses `hit_test(x, y, &APPS_MAP)`" but §6.2 says
"Stylus events are always delivered to the **focused app** only — not hit-tested by
position." This creates a contradiction:

- `route_touch` in §5.1 calls `hit_test_touch(ev.x, ev.y)` for all events.
- For stylus events, §6.2 says delivery should go to the focused app regardless of position.

If a stylus hover event comes in at coordinates outside the focused app's window (stylus
hovering over an empty desktop area), `hit_test` returns `None` and the stylus event is
dropped. But the focused app (e.g., a drawing app) needs hover events even when the pen
is outside its window boundary — hovering near the edge is a common workflow.

More critically, the stylus has `touch_id = -1` (§3.1), which is the sentinel for "no
tracking ID." If `route_touch` calls `hit_test` for stylus events and the stylus is off
all windows, the drawing app receives no data about stylus position, breaking hover-distance
calculations (Apple Pencil reports hover data up to 12 mm above screen).

**Proposed fix**:

Split stylus routing from touch routing at the top of `route_touch`:

```rust
pub fn route_touch(ev: &TouchEvent) {
    // ...lock-level gate and epoch check...

    if ev.touch_type == TouchType::Stylus || ev.touch_type == TouchType::Eraser {
        // Stylus always goes to focused app regardless of position
        let focused = FOCUSED_APP.load();
        let hit = lookup_hit_by_name(&focused, &APPS_MAP);  // single APPS_MAP lock
        if let Some(hit) = hit {
            deliver_touch(&hit, ev);
        }
        return;
    }

    // Finger touch: hit-test by position
    let hit = hit_test(ev.x, ev.y, &APPS_MAP);
    let Some(hit) = hit else { return; };
    deliver_touch(&hit, ev);

    if ev.phase == TouchPhase::Ended && ev.touch_type == TouchType::Finger {
        maybe_trigger_virtual_kbd(&hit);
    }
}
```

`lookup_hit_by_name(name: &str, apps_map: &AppsMap) -> Option<HitResult>` acquires
`APPS_MAP` once and clones the `stdin`/`win_x`/`win_y` from the named entry. This is the
same single-lock discipline as R32's `hit_test`.

Add a note to §6.2: "Stylus hover events are delivered to `FOCUSED_APP` rather than
hit-tested by position, because stylus hover may occur at coordinates outside the focused
app's window."

---

### B4: Touch-to-Mouse Synthesis Sends `MouseKind::Move` for `Began` Phase — App Never Receives Button Press for Tap

**Problem**:

Section §5.3 says single-finger touch is "synthesized as mouse events" for apps with
`mouse = true` but not `touch = true`. The implementation calls
`synthesize_mouse_from_touch(hit, ev)` but this function's body is never defined in the spec.

Based on the `TouchPhase` → `MouseKind` mapping, the synthesized events should be:

| TouchPhase | Expected MouseKind |
|-----------|-------------------|
| Began     | Press (Left)       |
| Moved     | Move               |
| Ended     | Release (Left)     |
| Cancelled | Release (Left)     |

If this mapping is wrong (e.g., emitting Move for Began), apps that use click detection
will never see button presses from touch. A drawing app that listens for
`VYOMA_INPUT:mouse:press` will never activate from touch input.

Furthermore, the function must update `CURRENT_BUTTONS` and `MOUSE_DRAG_CAPTURE` (R32
globals) to keep the drag capture state consistent. Without this, a touch-drag will not
trigger drag capture, and if the user starts a touch drag and raises their finger at the
edge of the window, the app will receive `Moved` events outside the window (which is
correct for drag capture) only if `MOUSE_DRAG_CAPTURE` is set.

**Proposed fix**:

Define the full `synthesize_mouse_from_touch` function:

```rust
fn synthesize_mouse_from_touch(hit: &HitResult, ev: &TouchEvent) {
    let (kind, button) = match ev.phase {
        TouchPhase::Began     => (MouseKind::Press,   MouseButton::Left),
        TouchPhase::Moved     => (MouseKind::Move,    MouseButton::None),
        TouchPhase::Ended     => (MouseKind::Release, MouseButton::Left),
        TouchPhase::Cancelled => (MouseKind::Release, MouseButton::Left),
    };

    // Update CURRENT_BUTTONS for R32 on_input_lock_fall compatibility
    match ev.phase {
        TouchPhase::Began     => CURRENT_BUTTONS.fetch_or(0b001, Ordering::Release),
        TouchPhase::Ended |
        TouchPhase::Cancelled => CURRENT_BUTTONS.fetch_and(!0b001, Ordering::Release),
        _ => 0,
    };

    let mouse_ev = MouseEvent {
        kind, button,
        button_mask: if CURRENT_BUTTONS.load(Ordering::Acquire) & 0b001 != 0 {
            ButtonMask::LEFT
        } else {
            ButtonMask::empty()
        },
        x: ev.x, y: ev.y, dx: 0, dy: 0,
        scroll_dx: 0, scroll_dy: 0,
        modifiers: 0,
        source: MouseSource::Touch,
        epoch: ev.epoch,
    };

    // Set/clear MOUSE_DRAG_CAPTURE for drag tracking
    if ev.phase == TouchPhase::Began {
        MOUSE_DRAG_CAPTURE.store(Arc::new(Some(hit.name.clone())));
    } else if ev.phase == TouchPhase::Ended || ev.phase == TouchPhase::Cancelled {
        MOUSE_DRAG_CAPTURE.store(Arc::new(None));
    }

    // Deliver via stdin (no APPS_MAP re-lock)
    let line = format_mouse_line(/* ... from mouse_ev ... */);
    let _ = hit.stdin.lock().unwrap().write_all(line.as_bytes());
}
```

---

### B5: Two Touch-Dispatch Threads — R32 `mouse-dispatch` and R33 `touch-dispatch` Both Call `hit_test` Concurrently — APPS_MAP Contention + `LAST_HOVER_TARGET` Race

**Problem**:

R32's `mouse-dispatch` thread calls `hit_test` and `synth_enter_leave_if_needed`, which
reads and writes `LAST_HOVER_TARGET: ArcSwap<Option<String>>`. R33's `touch-dispatch` also
calls `hit_test_touch` (which uses the same `hit_test` function) and `deliver_touch`.

Two separate threads now both call `hit_test` + potentially `synth_enter_leave_if_needed`
concurrently. While `APPS_MAP.lock()` prevents data corruption, `LAST_HOVER_TARGET` is an
`ArcSwap` — two threads can both read the old value, compute different new values, and both
store, with one store overwriting the other. This causes the hover target to oscillate
between the mouse's target and the touch's target on every frame, producing Enter/Leave
storms when mouse + touch input arrive simultaneously (e.g., stylus + trackpad hybrid use).

Furthermore, the R32 drag capture state (`MOUSE_DRAG_CAPTURE`) and `CURRENT_BUTTONS` are
shared between R32 mouse synthesis (§5.3) and R32 mouse dispatch. If both threads write
to `MOUSE_DRAG_CAPTURE` concurrently (touch synthesizes mouse press, mouse dispatch also
starts a drag), the ArcSwap stores race and one drag capture is silently lost.

**Proposed fix**:

Mouse events and touch-synthesized-as-mouse events must be dispatched by the **same single
thread**. The cleanest solution: touch-to-mouse synthesis does **not** call `deliver_touch`
directly but instead pushes a `MouseEvent` with `source = MouseSource::Touch` into R32's
existing `MouseEventQueue`. R32's `mouse-dispatch` then processes touch-synthesized events
alongside real mouse events, maintaining single-threaded ownership of `LAST_HOVER_TARGET`,
`MOUSE_DRAG_CAPTURE`, and `CURRENT_BUTTONS`.

```rust
fn synthesize_mouse_from_touch(hit: &HitResult, ev: &TouchEvent, mouse_q: &MouseEventQueue) {
    // ... construct mouse_ev as above ...
    mouse_q.lock().unwrap().push(mouse_ev);
    // touch-dispatch does NOT write CURRENT_BUTTONS or MOUSE_DRAG_CAPTURE directly;
    // mouse-dispatch handles that when it processes the synthesized event.
}
```

For native touch events (apps with `touch = true`), `touch-dispatch` still delivers
directly via `hit.stdin`. No `LAST_HOVER_TARGET` involvement because touch doesn't use
hover — every touch begins/ends discretely.

Update §11 Threading Model to note: `touch-dispatch` NEVER writes `MOUSE_DRAG_CAPTURE`,
`LAST_HOVER_TARGET`, or `CURRENT_BUTTONS`. Touch-to-mouse synthesis pushes to `MouseEventQueue`
and lets `mouse-dispatch` own all mouse state.
