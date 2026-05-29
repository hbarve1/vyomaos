# Architect Spec: Touch & Stylus Input (Round 33)

**Subsystem**: Touch & Stylus Input  
**macOS Analogue**: `UITouch` / `Apple Pencil` / `PencilKit`  
**Depends on**: R21 (apps_map, AppState), R25 (INPUT_LOCK_LEVEL), R28 (FOCUSED_APP), R31 (route_keyboard kbd_request_virtual, ModStateRef), R32 (MouseEventQueue pattern, HitResult, INPUT_EPOCH, hit_test, CURRENT_BUTTONS, on_input_lock_rise/fall)  
**Status**: DRAFT — Round 33

---

## 1. Overview

The touch/stylus subsystem reads Linux multi-touch type B events and Apple Pencil-class
stylus events from `/dev/input/touchN`, structures them into `TouchEvent` values, performs
hit testing, and delivers to apps via `VYOMA_INPUT:touch:` line protocol or the
`vyoma:touch@1.0.0` WIT interface.

On `mobile`, touch is the primary input modality. On `desktop-full`, touchscreen hardware
is optional. On all other platforms the subsystem is absent.

Three delivery paths:
1. **Touch-native apps** (`touch = true`): receive `VYOMA_INPUT:touch:` with full
   multi-touch data including phase, radius, pressure.
2. **Mouse-only apps** (`mouse = true`, `touch = false`): single-finger touch is
   synthesized as mouse events using `MouseSource::Touch`; multi-finger suppressed.
3. **Stylus apps** (`stylus = true`): receive `VYOMA_INPUT:stylus:` with tilt, twist,
   force data in addition to position.

---

## 2. Event Acquisition

### 2.1 touch-evdev Thread

```rust
// supervisor/src/input_touch.rs  (new, ≤ 500 lines)

pub type TouchEventQueue = Arc<Mutex<Vec<TouchEvent>>>;

#[cfg(target_os = "linux")]
pub fn run_touch_evdev(tq: TouchEventQueue) {
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
            EV_SYN => flush_touch_frame(&slots, &mut stylus, &tq),
            EV_ABS => match code {
                ABS_MT_SLOT        => { active_slot = (value as usize).min(MAX_TOUCH_SLOTS - 1); }
                ABS_MT_TRACKING_ID => {
                    if value < 0 { slots[active_slot].end_tracking(); }
                    else { slots[active_slot].begin_tracking(value); }
                }
                ABS_MT_POSITION_X => slots[active_slot].x = value,
                ABS_MT_POSITION_Y => slots[active_slot].y = value,
                ABS_MT_PRESSURE   => slots[active_slot].pressure = value,
                ABS_MT_TOUCH_MAJOR=> slots[active_slot].radius = value,
                // Single-touch (type A) or stylus
                ABS_X => stylus.x = value,
                ABS_Y => stylus.y = value,
                ABS_PRESSURE      => stylus.pressure = value,
                ABS_TILT_X        => stylus.tilt_x = value,
                ABS_TILT_Y        => stylus.tilt_y = value,
                ABS_Z             => stylus.twist = value,    // rotation angle
                _ => {}
            },
            EV_KEY => match code {
                BTN_TOUCH         => slots[0].btn_touch = value != 0,
                BTN_TOOL_PEN      => stylus.tool = StylusTool::Pen,
                BTN_TOOL_RUBBER   => stylus.tool = StylusTool::Eraser,
                BTN_TOOL_PENCIL   => stylus.tool = StylusTool::Pencil,
                BTN_STYLUS        => stylus.barrel_btn = value != 0,
                BTN_STYLUS2       => stylus.barrel_btn2 = value != 0,
                _ => {}
            },
            _ => {}
        }
    }
}
```

`open_touch_device` scans `/dev/input/event0..15` for a device with `ABS_MT_POSITION_X`
capability (distinguishes multitouch screen from trackpad, which is handled by R32).

### 2.2 TouchSlot and StylusState

```rust
const MAX_TOUCH_SLOTS: usize = 10;

#[derive(Clone, Copy, Default)]
struct TouchSlot {
    active:      bool,
    just_began:  bool,    // set on tracking_id assignment, cleared after flush
    tracking_id: i32,
    x:           i32,
    y:           i32,
    pressure:    i32,    // ABS_MT_PRESSURE, 0–255 typical
    radius:      i32,    // ABS_MT_TOUCH_MAJOR, contact size
    btn_touch:   bool,
}

impl TouchSlot {
    fn begin_tracking(&mut self, id: i32) {
        self.active = true; self.just_began = true; self.tracking_id = id;
    }
    fn end_tracking(&mut self) {
        self.active = false;
    }
}

#[derive(Clone, Copy, Default)]
struct StylusState {
    x: i32, y: i32, pressure: i32,
    tilt_x: i32, tilt_y: i32,   // degrees × 100
    twist: i32,                  // 0–35999 (centidegrees)
    tool: StylusTool,
    barrel_btn: bool, barrel_btn2: bool,
    active: bool,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum StylusTool { #[default] None, Pen, Eraser, Pencil }
```

---

## 3. TouchEvent Struct

```rust
// supervisor/src/input_touch.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TouchPhase { Began=0, Moved=1, Ended=2, Cancelled=3 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TouchType { Finger=0, Stylus=1, Eraser=2 }

#[derive(Debug, Clone, Copy)]
pub struct TouchEvent {
    pub touch_id:   i32,         // Linux tracking_id; unique per contact lifetime
    pub phase:      TouchPhase,
    pub touch_type: TouchType,
    pub x:          i32,         // absolute screen coords (calibrated)
    pub y:          i32,
    pub radius:     f32,         // contact radius in logical pixels
    pub pressure:   f32,         // 0.0–1.0 normalised
    // Stylus-specific (zero for finger touches)
    pub tilt_x:     f32,         // X tilt in degrees (-90..90)
    pub tilt_y:     f32,         // Y tilt in degrees (-90..90)
    pub twist:      f32,         // rotation 0..360 degrees
    pub barrel_btn: bool,
    pub barrel_btn2:bool,
    pub epoch:      u8,          // INPUT_EPOCH at creation (B3 from R32 pattern)
}
```

### 3.1 flush_touch_frame

```rust
fn flush_touch_frame(slots: &[TouchSlot; MAX_TOUCH_SLOTS], stylus: &mut StylusState, tq: &TouchEventQueue) {
    let epoch = INPUT_EPOCH.load(Ordering::Acquire);
    let mut events: Vec<TouchEvent> = Vec::new();

    // Emit touch events per changed slot
    for slot in slots.iter() {
        if !slot.active && !slot.just_began {
            // Was just ended (tracking_id → -1)
            // (ended slots are pushed with phase=Ended before clearing)
        }
        if slot.active {
            let phase = if slot.just_began { TouchPhase::Began } else { TouchPhase::Moved };
            events.push(TouchEvent {
                touch_id: slot.tracking_id,
                phase,
                touch_type: TouchType::Finger,
                x: calibrate_x(slot.x), y: calibrate_y(slot.y),
                radius: slot.radius as f32 / RADIUS_SCALE,
                pressure: (slot.pressure as f32 / 255.0).clamp(0.0, 1.0),
                tilt_x: 0.0, tilt_y: 0.0, twist: 0.0,
                barrel_btn: false, barrel_btn2: false,
                epoch,
            });
        }
    }

    // Emit stylus event if active
    if stylus.tool != StylusTool::None && stylus.active {
        events.push(TouchEvent {
            touch_id: -1,   // stylus has no MT tracking_id
            phase:    TouchPhase::Moved,
            touch_type: if stylus.tool == StylusTool::Eraser { TouchType::Eraser } else { TouchType::Stylus },
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
    }

    if !events.is_empty() {
        tq.lock().unwrap().extend(events);
    }
}
```

---

## 4. Coordinate Calibration

Touch screens report raw ADC values. Calibration converts to logical screen pixels using
a linear map stored in platform profile TOML:

```toml
# supervisor/src/profile/profiles/mobile.toml
[touch_calibration]
x_min = 0; x_max = 4095; y_min = 0; y_max = 4095
screen_w = 1080; screen_h = 2340
```

```rust
// supervisor/src/input_touch.rs
fn calibrate_x(raw: i32) -> i32 {
    let cal = &TOUCH_CALIBRATION;
    ((raw - cal.x_min) as i64 * cal.screen_w as i64
        / (cal.x_max - cal.x_min) as i64) as i32
}
```

Calibration data is loaded at startup via `platform_profile` (spec-043 subsystem).

---

## 5. Dispatch Pipeline (route_touch)

### 5.1 touch-dispatch Thread

A dedicated `touch-dispatch` thread drains `TouchEventQueue` every 2 ms and calls
`route_touch` for each event — same pattern as R32's `mouse-dispatch`.

```rust
// supervisor/src/input_touch.rs

pub fn route_touch(ev: &TouchEvent) {
    // (a) Lock-level gate
    let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
    match lock {
        LL_FS_TRANSITION => return,
        LL_MC | LL_STAGE_SWITCH => {
            // Chrome-only; touch doesn't navigate MC (unlike mouse) — suppress
            return;
        }
        _ => {}
    }

    // (b) Drop stale epoch events (same as R32 B3 fix)
    if ev.epoch != INPUT_EPOCH.load(Ordering::Acquire) { return; }

    // (c) Hit test — single APPS_MAP lock (B1: same HitResult pattern as R32)
    let hit = hit_test_touch(ev.x, ev.y);
    let Some(hit) = hit else { return; };

    // (d) Deliver
    deliver_touch(&hit, ev);

    // (e) Virtual keyboard trigger for finger taps in text fields (R31 §10.3)
    if ev.phase == TouchPhase::Ended && ev.touch_type == TouchType::Finger {
        maybe_trigger_virtual_kbd(&hit);
    }
}
```

### 5.2 hit_test_touch

Reuses the same `hit_test(x, y, &APPS_MAP)` function from R32. Returns `Option<HitResult>`.
Touch events use window-local coordinates just like mouse events.

### 5.3 Touch-to-Mouse Fallback

For apps with `mouse = true` but not `touch = true`, single-finger touch is synthesized
as mouse events:

```rust
fn deliver_touch(hit: &HitResult, ev: &TouchEvent) {
    let has_touch = app_has_touch_capability(&hit.name);
    let has_mouse = app_has_mouse_capability(&hit.name);

    if has_touch {
        let line = format_touch_line(ev, ev.x - hit.win_x, ev.y - hit.win_y);
        // Per-app stdin write via HitResult.stdin Arc (no APPS_MAP re-lock)
        let _ = hit.stdin.lock().unwrap().write_all(line.as_bytes());
    } else if has_mouse && ev.touch_type == TouchType::Finger {
        // Only single-finger is mapped to mouse; multi-touch suppressed
        synthesize_mouse_from_touch(hit, ev);
    }
}
```

If an app has **both** `touch = true` and `mouse = true`, it receives **only** touch events —
not duplicate mouse events. The mouse fallback is skipped when `has_touch = true`.

---

## 6. VYOMA_INPUT:touch: Protocol

### 6.1 Wire Format

```
VYOMA_INPUT:touch:began:<touch_id>,<x>,<y>,<radius_x1000>,<pressure_x1000>
VYOMA_INPUT:touch:moved:<touch_id>,<x>,<y>,<radius_x1000>,<pressure_x1000>
VYOMA_INPUT:touch:ended:<touch_id>,<x>,<y>
VYOMA_INPUT:touch:cancelled:<touch_id>
```

### 6.2 Stylus Lines (stylus = true required)

```
VYOMA_INPUT:stylus:moved:<x>,<y>,<pressure_x1000>,<tilt_x_x1000>,<tilt_y_x1000>,<twist_x1000>,<barrel>
VYOMA_INPUT:stylus:eraser:<x>,<y>,<pressure_x1000>
```

Where `<barrel>` is `0` or `1` (barrel button state). Stylus events are always delivered
to the focused app only (not hit-tested by position) — stylus hover can be off-screen in some
hardware configurations.

### 6.3 Coordinate System

All coordinates are window-local (relative to `HitResult.win_x`, `HitResult.win_y`).
This is consistent with R32's mouse coordinate convention.

### 6.4 Touch Sequence Invariants

1. Every `began:<id>` is followed by exactly one `ended:<id>` or `cancelled:<id>`.
2. `moved:<id>` only arrives between `began:<id>` and `ended/<cancelled>:<id>`.
3. Multiple simultaneous touches have distinct `touch_id` values within any window's lifetime.
4. A `cancelled` is emitted for all active touches when `INPUT_LOCK_LEVEL` rises.

---

## 7. Virtual Keyboard Integration (R31 §10.3)

When a finger tap ends on a text field (AX role `TextField` or `TextArea`, R30), the
focused app should be prompted to show the virtual keyboard:

```rust
fn maybe_trigger_virtual_kbd(hit: &HitResult) {
    // Only on mobile; on desktop-full this is a no-op
    if PLATFORM_PROFILE.virtual_kbd_enabled {
        // Notify focused app — it may or may not call @supervisor: kbd_request_virtual
        // Apps are responsible for requesting virtual keyboard from their own AX tree
        log_info!(Subsystem::Input, Some(&hit.name), "touch tap ended — app may request vkbd");
    }
}
```

The virtual keyboard appears/hides via the existing R31 `VYOMA_SYSTEM:virtual_kbd:show/hide`
broadcast mechanism. Touch-end is just the trigger hint; apps drive the actual show/hide.

---

## 8. Gesture Recognition for Touch

Two-finger and multi-finger gestures on touch screens are handled separately from trackpad
gestures (R32 §7):

- **Two-finger scroll**: detected by `touch-dispatch` monitoring two simultaneous `Moved`
  events with parallel direction; emits `VYOMA_INPUT:gesture:scroll_*` to the target app.
- **Pinch-zoom**: same two-finger disambiguation as R32 §7.3, but applied to touch slots.
- **Three/four-finger swipe**: routed to chrome (same as R32 SwipeThree/SwipeFour).

The gesture state machine reuses `GestureState` from `gesture.rs` (R32). `touch-dispatch`
maintains its own `GestureStateRef` distinct from `trackpad-evdev`'s.

---

## 9. WIT Interface `vyoma:touch@1.0.0`

```wit
package vyoma:touch@1.0.0;

interface touch-events {
    poll-touch: func() -> option<touch-event>;

    record touch-event {
        touch-id:   s32,
        phase:      touch-phase,
        touch-type: touch-type,
        x: s32, y: s32,
        radius:   float32,
        pressure: float32,
    }

    enum touch-phase   { began, moved, ended, cancelled }
    enum touch-type    { finger, stylus, eraser }
}

interface stylus-events {
    poll-stylus: func() -> option<stylus-event>;

    record stylus-event {
        x: s32, y: s32, pressure: float32,
        tilt-x: float32, tilt-y: float32, twist: float32,
        barrel: bool,
    }
}

world touch { import touch-events; import stylus-events; }
```

---

## 10. Platform Matrix

| Platform | Touch | Stylus | Notes |
|----------|-------|--------|-------|
| `mobile` | yes (primary) | yes (optional hw) | Full pipeline; virtual kbd integration |
| `desktop-full` | optional | optional | Touchscreen detected at runtime; gesture recognition enabled if present |
| `server-headless` | no | no | Threads not started |
| `iot-edge` | optional | no | touch-evdev may run if config flag set; no gesture recognition |
| `robotics-rt` | no | no | No input threads |
| `mcu-minimal` | no | no | Subsystem not compiled |

---

## 11. Threading Model

```
Thread: touch-evdev
  Reads EV_ABS (MT slot + stylus) + EV_KEY (BTN_TOOL_*) events
  On SYN_REPORT: flush_touch_frame → push TouchEvent to TouchEventQueue
  NEVER calls route_touch or reads APPS_MAP

Thread: touch-dispatch (sole consumer)
  Drains TouchEventQueue every 2ms
  Calls route_touch for each event
  Reads APPS_MAP once per event via hit_test_touch (HitResult pattern)
  Writes per-app stdin via HitResult.stdin Arc
```

---

## 12. File Layout

```
supervisor/src/
├── input_touch.rs     — TouchEvent, TouchPhase, TouchType, TouchSlot, StylusState,
│                         StylusTool, TouchEventQueue, run_touch_evdev, flush_touch_frame,
│                         route_touch, hit_test_touch, deliver_touch, synthesize_mouse_from_touch,
│                         format_touch_line, maybe_trigger_virtual_kbd,
│                         calibrate_x/y, TOUCH_CALIBRATION
└── manifest.rs        — add touch: bool, stylus: bool to capabilities
```

---

## 13. Capability Summary

| Field | Type | Grants |
|-------|------|--------|
| `touch` | bool | Receive `VYOMA_INPUT:touch:` events + `vyoma:touch@1.0.0` WIT |
| `stylus` | bool | Receive `VYOMA_INPUT:stylus:` events (requires `touch = true`) |
