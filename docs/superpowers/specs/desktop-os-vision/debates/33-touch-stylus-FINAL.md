# FINAL Spec: Touch & Stylus Input (Round 33)

**Subsystem**: Touch & Stylus Input  
**macOS Analogue**: `UITouch` / `Apple Pencil` / `PencilKit`  
**Depends on**: R21 (apps_map, AppState), R25 (INPUT_LOCK_LEVEL), R28 (FOCUSED_APP), R31 (kbd_request_virtual, INPUT_EPOCH), R32 (MouseEventQueue, HitResult, hit_test, MOUSE_DRAG_CAPTURE, CURRENT_BUTTONS, LAST_HOVER_TARGET, MouseSource::Touch, on_input_lock_rise/fall)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

The touch/stylus subsystem reads Linux multi-touch type B events and stylus events from
`/dev/input/touchN`, emits `TouchEvent` values, hit-tests against the window list, and
delivers to apps. Three delivery paths:

1. **Touch-native apps** (`touch = true`): `VYOMA_INPUT:touch:` line protocol.
2. **Mouse-only apps** (`mouse = true`, `touch = false`): single-finger touch is
   synthesized as `MouseEvent` with `source = MouseSource::Touch` and pushed into R32's
   `MouseEventQueue` — `mouse-dispatch` handles delivery (B5 fix).
3. **Stylus apps** (`stylus = true`): `VYOMA_INPUT:stylus:` with tilt, twist, force data.

---

## 2. Event Acquisition

### 2.1 touch-evdev Thread

```rust
// supervisor/src/input_touch.rs  (≤ 500 lines)

pub type TouchEventQueue = Arc<Mutex<Vec<TouchEvent>>>;

#[cfg(target_os = "linux")]
pub fn run_touch_evdev(tq: TouchEventQueue, mouse_q: MouseEventQueue) {
    let Some(mut dev) = open_touch_device() else {
        log_info!(Subsystem::Input, None, "touch-evdev: no touch device, skipping");
        return;
    };
    let mut slots: [TouchSlot; MAX_TOUCH_SLOTS] = Default::default();
    let mut active_slot = 0usize;
    let mut stylus = StylusState::default();
    let mut buf = [0u8; 24];
    loop {
        if dev.read_exact(&mut buf).is_err() { break; }
        let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
        let code    = u16::from_ne_bytes([buf[18], buf[19]]);
        let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        match ev_type {
            EV_SYN => flush_touch_frame(&mut slots, &mut stylus, &tq, &mouse_q),
            EV_ABS => match code {
                ABS_MT_SLOT        => { active_slot = (value as usize).min(MAX_TOUCH_SLOTS - 1); }
                ABS_MT_TRACKING_ID => {
                    if value < 0 { slots[active_slot].end_tracking(); }
                    else { slots[active_slot].begin_tracking(value); }
                }
                ABS_MT_POSITION_X  => slots[active_slot].x = value,
                ABS_MT_POSITION_Y  => slots[active_slot].y = value,
                ABS_MT_PRESSURE    => slots[active_slot].pressure = value,
                ABS_MT_TOUCH_MAJOR => slots[active_slot].radius = value,
                ABS_X => stylus.x = value,
                ABS_Y => stylus.y = value,
                ABS_PRESSURE => stylus.pressure = value,
                ABS_TILT_X   => stylus.tilt_x = value,
                ABS_TILT_Y   => stylus.tilt_y = value,
                ABS_Z        => stylus.twist = value,
                _ => {}
            },
            EV_KEY => match code {
                BTN_TOUCH => {
                    if stylus.tool != StylusTool::None {
                        stylus.btn_touch = value != 0;   // stylus tip contact (B2)
                    } else {
                        slots[0].btn_touch = value != 0; // type-A single touch
                    }
                }
                BTN_TOOL_PEN    => { stylus.tool = StylusTool::Pen;    stylus.active = true; }
                BTN_TOOL_RUBBER => { stylus.tool = StylusTool::Eraser; stylus.active = true; }
                BTN_TOOL_PENCIL => { stylus.tool = StylusTool::Pencil; stylus.active = true; }
                BTN_TOOL_FINGER => { stylus.tool = StylusTool::None;   stylus.active = false; }
                BTN_STYLUS  => stylus.barrel_btn  = value != 0,
                BTN_STYLUS2 => stylus.barrel_btn2 = value != 0,
                _ => {}
            },
            _ => {}
        }
    }
}
```

### 2.2 TouchSlot (B1 Fix — just_ended field)

```rust
const MAX_TOUCH_SLOTS: usize = 10;

#[derive(Clone, Copy, Default)]
struct TouchSlot {
    active:      bool,
    just_began:  bool,   // set on begin_tracking; cleared after first flush
    just_ended:  bool,   // set on end_tracking; cleared after Ended event emitted
    tracking_id: i32,
    x: i32, y: i32, pressure: i32, radius: i32, btn_touch: bool,
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

### 2.3 StylusState (B2 Fix — btn_touch + prev_btn_touch)

```rust
#[derive(Clone, Copy, Default)]
struct StylusState {
    x: i32, y: i32, pressure: i32,
    tilt_x: i32, tilt_y: i32,   // raw evdev values
    twist: i32,
    tool: StylusTool,
    active: bool,
    btn_touch:      bool,
    prev_btn_touch: bool,   // state from previous flush (for Began/Ended detection)
    barrel_btn: bool, barrel_btn2: bool,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum StylusTool { #[default] None, Pen, Eraser, Pencil }

const STYLUS_TOUCH_ID: i32 = i32::MIN;   // reserved sentinel; never conflicts with MT tracking IDs
```

---

## 3. TouchEvent Struct

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TouchPhase { Began=0, Moved=1, Ended=2, Cancelled=3 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TouchType { Finger=0, Stylus=1, Eraser=2 }

#[derive(Debug, Clone, Copy)]
pub struct TouchEvent {
    pub touch_id:    i32,         // MT tracking_id; STYLUS_TOUCH_ID for stylus
    pub phase:       TouchPhase,
    pub touch_type:  TouchType,
    pub x:           i32,         // calibrated screen coords
    pub y:           i32,
    pub radius:      f32,         // contact radius in logical pixels
    pub pressure:    f32,         // 0.0–1.0
    pub tilt_x:      f32,         // degrees (-90..90); 0.0 for finger
    pub tilt_y:      f32,
    pub twist:       f32,         // 0.0..360.0; 0.0 for finger
    pub barrel_btn:  bool,
    pub barrel_btn2: bool,
    pub epoch:       u8,          // INPUT_EPOCH at creation (stale-event filter)
}
```

---

## 4. flush_touch_frame (B1 + B2 Fix)

```rust
fn flush_touch_frame(
    slots: &mut [TouchSlot; MAX_TOUCH_SLOTS],
    stylus: &mut StylusState,
    tq: &TouchEventQueue,
    mouse_q: &MouseEventQueue,
) {
    let epoch = INPUT_EPOCH.load(Ordering::Acquire);
    let mut events: Vec<TouchEvent> = Vec::new();

    // ── Finger touches ──────────────────────────────────────────────────
    for slot in slots.iter_mut() {
        if slot.just_ended {
            events.push(TouchEvent {
                touch_id: slot.tracking_id, phase: TouchPhase::Ended,
                touch_type: TouchType::Finger,
                x: calibrate_x(slot.x), y: calibrate_y(slot.y),
                radius: 0.0, pressure: 0.0,
                tilt_x: 0.0, tilt_y: 0.0, twist: 0.0,
                barrel_btn: false, barrel_btn2: false, epoch,
            });
            slot.just_ended = false;
        } else if slot.active {
            let phase = if slot.just_began { TouchPhase::Began } else { TouchPhase::Moved };
            events.push(TouchEvent {
                touch_id: slot.tracking_id, phase, touch_type: TouchType::Finger,
                x: calibrate_x(slot.x), y: calibrate_y(slot.y),
                radius: slot.radius as f32 / RADIUS_SCALE,
                pressure: (slot.pressure as f32 / 255.0).clamp(0.0, 1.0),
                tilt_x: 0.0, tilt_y: 0.0, twist: 0.0,
                barrel_btn: false, barrel_btn2: false, epoch,
            });
            slot.just_began = false;   // clear after first emission (B1 fix)
        }
    }

    // ── Stylus ──────────────────────────────────────────────────────────
    if stylus.tool != StylusTool::None {
        let phase = match (stylus.prev_btn_touch, stylus.btn_touch) {
            (false, true)  => Some(TouchPhase::Began),
            (true,  true)  => Some(TouchPhase::Moved),
            (true,  false) => Some(TouchPhase::Ended),
            (false, false) => None,  // hover only; emit Moved for hover tracking
        };
        let effective_phase = phase.unwrap_or(TouchPhase::Moved);
        events.push(TouchEvent {
            touch_id:    STYLUS_TOUCH_ID,
            phase:       effective_phase,
            touch_type:  if stylus.tool == StylusTool::Eraser { TouchType::Eraser } else { TouchType::Stylus },
            x: calibrate_x(stylus.x), y: calibrate_y(stylus.y),
            radius: 1.0,
            pressure: (stylus.pressure as f32 / 1023.0).clamp(0.0, 1.0),
            tilt_x:  stylus.tilt_x as f32 / 100.0,
            tilt_y:  stylus.tilt_y as f32 / 100.0,
            twist:   stylus.twist  as f32 / 100.0,
            barrel_btn:  stylus.barrel_btn,
            barrel_btn2: stylus.barrel_btn2,
            epoch,
        });
        stylus.prev_btn_touch = stylus.btn_touch;
    }

    if !events.is_empty() {
        tq.lock().unwrap().extend(events);
    }
}
```

---

## 5. Coordinate Calibration

```toml
# supervisor/src/profile/profiles/mobile.toml
[touch_calibration]
x_min = 0; x_max = 4095; y_min = 0; y_max = 4095
screen_w = 1080; screen_h = 2340
```

```rust
fn calibrate_x(raw: i32) -> i32 {
    let cal = &TOUCH_CALIBRATION;
    ((raw - cal.x_min) as i64 * cal.screen_w as i64
        / (cal.x_max - cal.x_min) as i64) as i32
}
```

---

## 6. Dispatch Pipeline (route_touch)

### 6.1 Finger Touch Routing (B3 + B5 Fix)

```rust
// supervisor/src/input_touch.rs

pub fn route_touch(ev: &TouchEvent, mouse_q: &MouseEventQueue) {
    // (a) Lock-level gate
    let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
    if lock == LL_FS_TRANSITION { return; }
    if lock == LL_MC || lock == LL_STAGE_SWITCH { return; }

    // (b) Stale epoch check
    if ev.epoch != INPUT_EPOCH.load(Ordering::Acquire) { return; }

    // (c) Route stylus to focused app; route finger by hit test (B3 fix)
    if ev.touch_type == TouchType::Stylus || ev.touch_type == TouchType::Eraser {
        // Stylus: deliver to FOCUSED_APP regardless of position
        let focused = FOCUSED_APP.load();
        if let Some(hit) = lookup_hit_by_name(&focused, &APPS_MAP) {
            deliver_native_touch(&hit, ev);
        }
        return;
    }

    // Finger: hit-test by position (single APPS_MAP lock via hit_test)
    let hit = hit_test(ev.x, ev.y, &APPS_MAP);
    let Some(hit) = hit else { return; };

    let has_touch = app_has_touch_capability(&hit.name);
    let has_mouse = app_has_mouse_capability(&hit.name);

    if has_touch {
        // Native touch delivery — touch-dispatch owns stdin write (no APPS_MAP re-lock)
        deliver_native_touch(&hit, ev);
    } else if has_mouse {
        // Touch-to-mouse synthesis: push to mouse_q, let mouse-dispatch own delivery (B5)
        synthesize_mouse_event_into_queue(ev, &hit, mouse_q);
    }

    // Virtual keyboard trigger on finger tap end
    if ev.phase == TouchPhase::Ended && ev.touch_type == TouchType::Finger && has_touch {
        maybe_trigger_virtual_kbd(&hit);
    }
}
```

### 6.2 lookup_hit_by_name (B3 Fix)

```rust
fn lookup_hit_by_name(name: &str, apps_map: &AppsMap) -> Option<HitResult> {
    let m = apps_map.lock().unwrap();
    let state = m.get(name)?;
    Some(HitResult {
        name:      name.to_string(),
        stdin:     state.stdin_writer.clone(),
        is_space0: state.space == 0,
        win_x:     state.win_x,
        win_y:     state.win_y,
        wit_inbox: state.wit_inbox.clone(),
    })
    // APPS_MAP released before returning
}
```

### 6.3 Touch-to-Mouse Synthesis (B4 + B5 Fix)

```rust
// B4: correct phase → MouseKind mapping
// B5: push to mouse_q (not deliver directly) so mouse-dispatch owns all mouse state

fn synthesize_mouse_event_into_queue(ev: &TouchEvent, hit: &HitResult, mouse_q: &MouseEventQueue) {
    let (kind, button) = match ev.phase {
        TouchPhase::Began     => (MouseKind::Press,   MouseButton::Left),
        TouchPhase::Moved     => (MouseKind::Move,    MouseButton::None),
        TouchPhase::Ended     |
        TouchPhase::Cancelled => (MouseKind::Release, MouseButton::Left),
    };

    // Note: CURRENT_BUTTONS and MOUSE_DRAG_CAPTURE are updated by mouse-dispatch
    // when it processes this synthesized event — NOT here. (B5 fix)
    let mouse_ev = MouseEvent {
        kind, button,
        button_mask: match ev.phase {
            TouchPhase::Began => ButtonMask::LEFT,
            _                 => ButtonMask::empty(),
        },
        x: ev.x, y: ev.y, dx: 0, dy: 0,
        scroll_dx: 0, scroll_dy: 0,
        modifiers: 0,
        source: MouseSource::Touch,
        epoch: ev.epoch,
    };
    mouse_q.lock().unwrap().push(mouse_ev);
}
```

### 6.4 Native Touch Delivery

```rust
fn deliver_native_touch(hit: &HitResult, ev: &TouchEvent) {
    if !app_has_touch_capability(&hit.name) { return; }
    let local_x = ev.x - hit.win_x;
    let local_y = ev.y - hit.win_y;
    let line = format_touch_line(ev, local_x, local_y);
    // Per-app stdin lock — APPS_MAP already released
    let _ = hit.stdin.lock().unwrap().write_all(line.as_bytes());
}
```

---

## 7. VYOMA_INPUT:touch: Protocol

### 7.1 Finger Touch Lines

```
VYOMA_INPUT:touch:began:<touch_id>,<x>,<y>,<radius_x1000>,<pressure_x1000>
VYOMA_INPUT:touch:moved:<touch_id>,<x>,<y>,<radius_x1000>,<pressure_x1000>
VYOMA_INPUT:touch:ended:<touch_id>,<x>,<y>
VYOMA_INPUT:touch:cancelled:<touch_id>
```

All coordinates are window-local.

### 7.2 Stylus Lines (stylus = true required)

```
VYOMA_INPUT:stylus:began:<x>,<y>,<pressure_x1000>
VYOMA_INPUT:stylus:moved:<x>,<y>,<pressure_x1000>,<tilt_x_x1000>,<tilt_y_x1000>,<twist_x1000>,<barrel>
VYOMA_INPUT:stylus:ended:<x>,<y>
VYOMA_INPUT:stylus:hover:<x>,<y>,<tilt_x_x1000>,<tilt_y_x1000>
```

`hover` is emitted when the stylus is near the screen but `btn_touch = false`
(Apple Pencil hover up to ~12 mm). Coordinates are screen-relative for hover (stylus may
not be over the focused window).

### 7.3 Touch Sequence Invariants

1. Every `began:<id>` is followed by exactly one `ended:<id>` or `cancelled:<id>`.
2. `moved:<id>` only arrives between `began:<id>` and `ended/cancelled:<id>`.
3. All active touches are cancelled (emit `cancelled` for each active `touch_id`) when
   `INPUT_LOCK_LEVEL` rises from `LL_NONE`. `on_input_lock_rise()` handles this.
4. Apps with both `touch = true` and `mouse = true` receive only touch events — not
   duplicate mouse events (B5 fix: `has_touch` check in `route_touch`).

---

## 8. Cancelled Events on Lock-Level Rise

`on_input_lock_rise()` (R32 §6.4) is extended to also cancel all in-flight touch sequences:

```rust
// supervisor/src/input_mouse.rs — on_input_lock_rise extension

pub fn on_input_lock_rise() {
    // R32: synthesize Release for drag capture
    // ... (existing) ...

    // R33: synthesize Cancelled for all active touch sequences
    cancel_all_active_touches();

    INPUT_EPOCH.fetch_add(1, Ordering::Release);
}

fn cancel_all_active_touches() {
    // Iterate ACTIVE_TOUCH_TARGETS (a per-app Vec<i32> of active touch_ids)
    // and send VYOMA_INPUT:touch:cancelled:<id> to each affected app.
    let mut targets = ACTIVE_TOUCH_TARGETS.lock().unwrap();
    for (app_name, ids) in targets.iter() {
        if let Some(hit) = lookup_hit_by_name(app_name, &APPS_MAP) {
            for &id in ids {
                let line = format!("VYOMA_INPUT:touch:cancelled:{id}\n");
                let _ = hit.stdin.lock().unwrap().write_all(line.as_bytes());
            }
        }
    }
    targets.clear();
}

// Maintained by deliver_native_touch:
pub static ACTIVE_TOUCH_TARGETS: Mutex<HashMap<String, Vec<i32>>> = Mutex::new(HashMap::new());
// Updated: Began → insert id; Ended/Cancelled → remove id
```

---

## 9. Gesture Recognition for Touch

Two-finger and multi-finger gestures are handled by a separate `GestureStateRef`
(same `GestureState` type from R32's `gesture.rs`) owned by `touch-dispatch`. The gesture
state machine recognises scroll, pinch, and swipe — identical to R32 but fed from finger
touch slots rather than trackpad slots.

SwipeFour and SwipeThree are intercepted by chrome (never delivered to apps) — same as R32.

---

## 10. WIT Interface `vyoma:touch@1.0.0`

```wit
package vyoma:touch@1.0.0;

interface touch-events {
    poll-touch: func() -> option<touch-event>;

    record touch-event {
        touch-id: s32, phase: touch-phase, touch-type: touch-type,
        x: s32, y: s32, radius: float32, pressure: float32,
    }
    enum touch-phase  { began, moved, ended, cancelled }
    enum touch-type   { finger, stylus, eraser }
}

interface stylus-events {
    poll-stylus: func() -> option<stylus-event>;

    record stylus-event {
        x: s32, y: s32, pressure: float32,
        tilt-x: float32, tilt-y: float32, twist: float32,
        barrel: bool, hovering: bool,
    }
}

world touch { import touch-events; import stylus-events; }
```

---

## 11. Threading Model (B5 Fix)

```
Thread: touch-evdev
  Reads EV_ABS (MT slots, stylus) + EV_KEY (BTN_TOOL_*, BTN_TOUCH, BTN_STYLUS)
  On SYN_REPORT: flush_touch_frame (with &mut slots for just_began/just_ended clearing)
    → pushes TouchEvent to TouchEventQueue
    → pushes synthesized MouseEvent to MouseEventQueue (for touch-to-mouse fallback)
  NEVER calls route_touch, deliver_native_touch, or reads APPS_MAP

Thread: touch-dispatch (sole consumer of TouchEventQueue)
  Drains TouchEventQueue every 2ms
  Calls route_touch for each event
  For finger touches: hit_test (single APPS_MAP lock) → deliver_native_touch
  For stylus: lookup_hit_by_name(FOCUSED_APP) → deliver_native_touch
  For touch-to-mouse: NOT done here — touch-evdev pushed to MouseEventQueue

Thread: mouse-dispatch (R32, unchanged)
  Processes MouseEventQueue including synthesized MouseSource::Touch events
  Owns LAST_HOVER_TARGET, MOUSE_DRAG_CAPTURE, CURRENT_BUTTONS exclusively
```

**Key invariant**: `touch-dispatch` NEVER writes `MOUSE_DRAG_CAPTURE`, `LAST_HOVER_TARGET`,
or `CURRENT_BUTTONS`. Touch-to-mouse synthesis pushes to `MouseEventQueue` so `mouse-dispatch`
maintains single-threaded ownership of all mouse state.

---

## 12. Platform Matrix

| Platform | Touch | Stylus | Notes |
|----------|-------|--------|-------|
| `mobile` | yes (primary) | yes (optional hw) | Full pipeline; virtual kbd integration; gesture recognition |
| `desktop-full` | optional | optional | Detected at runtime; thread starts only if device found |
| `server-headless` | no | no | Threads not started |
| `iot-edge` | optional | no | `run_touch_evdev` started if config flag set; no gesture recognition |
| `robotics-rt` | no | no | No input threads |
| `mcu-minimal` | no | no | Subsystem not compiled |

---

## 13. Capability Summary

| Field | Type | Grants |
|-------|------|--------|
| `touch` | bool | Receive `VYOMA_INPUT:touch:` events + `vyoma:touch@1.0.0` WIT |
| `stylus` | bool | Receive `VYOMA_INPUT:stylus:` events (requires `touch = true`) |

---

## 14. File Layout

```
supervisor/src/input_touch.rs  — TouchEvent, TouchPhase, TouchType, TouchSlot (+ just_ended B1),
                                  StylusState (+ btn_touch/prev_btn_touch B2), STYLUS_TOUCH_ID,
                                  StylusTool, TouchEventQueue, run_touch_evdev,
                                  flush_touch_frame (B1+B2), route_touch (B3+B5),
                                  lookup_hit_by_name (B3), deliver_native_touch,
                                  synthesize_mouse_event_into_queue (B4+B5),
                                  maybe_trigger_virtual_kbd, calibrate_x/y,
                                  TOUCH_CALIBRATION, ACTIVE_TOUCH_TARGETS,
                                  cancel_all_active_touches

supervisor/src/input_mouse.rs  — on_input_lock_rise extended to call cancel_all_active_touches

supervisor/src/manifest.rs     — add touch: bool, stylus: bool to capabilities

supervisor/src/wit/touch.wit   — vyoma:touch@1.0.0

supervisor/tests/
├── touch_phases.rs            — Began/Moved/Ended/Cancelled sequence correctness (B1)
├── touch_stylus.rs            — stylus Began/Ended via btn_touch (B2)
├── touch_stylus_routing.rs    — stylus → FOCUSED_APP not hit_test (B3)
├── touch_mouse_synthesis.rs   — touch→mouse phase mapping correctness (B4)
└── touch_mouse_ownership.rs   — single-thread ownership of MOUSE_DRAG_CAPTURE (B5)
```

---

## 15. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `flush_touch_frame` never emits `Ended` events; `just_began` never cleared | `just_ended: bool` added to `TouchSlot`; set by `end_tracking`; `flush_touch_frame` takes `&mut slots` to clear `just_began` after first emission and `just_ended` after `Ended` event |
| B2: Stylus has no `Began`/`Ended` phase; `BTN_TOUCH` not mapped for stylus | `btn_touch: bool` + `prev_btn_touch: bool` on `StylusState`; `BTN_TOUCH` routed to stylus when `tool != None`; `prev/current btn_touch` comparison emits `Began`/`Moved`/`Ended` in `flush_touch_frame`; `STYLUS_TOUCH_ID = i32::MIN` sentinel |
| B3: `route_touch` calls `hit_test` for stylus events but spec says deliver to `FOCUSED_APP` — contradicted by position-based routing dropping hover-outside-window stylus events | Stylus events bypass `hit_test`; `lookup_hit_by_name(FOCUSED_APP, &APPS_MAP)` acquires `APPS_MAP` once to clone `HitResult` for the focused app; stylus hover events delivered even when pen is outside window bounds |
| B4: Touch-to-mouse synthesis undefined; `Began` must map to `Press`, not `Move`; `CURRENT_BUTTONS` must be updated | `synthesize_mouse_event_into_queue` fully defined: `Began→Press`, `Moved→Move`, `Ended/Cancelled→Release`; does NOT update `CURRENT_BUTTONS` — deferred to `mouse-dispatch` (B5 fix) |
| B5: `touch-dispatch` and `mouse-dispatch` both write `LAST_HOVER_TARGET`, `MOUSE_DRAG_CAPTURE`, `CURRENT_BUTTONS` concurrently — ArcSwap stores race | Touch-to-mouse synthesis pushes `MouseEvent(source=Touch)` to `MouseEventQueue`; `mouse-dispatch` is the sole writer of all mouse state; `touch-dispatch` never writes those globals |
