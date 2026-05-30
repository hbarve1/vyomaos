# Architect Spec: Mouse, Trackpad & Gestures (Round 32)

**Subsystem**: Mouse, Trackpad & Gestures
**macOS Analogue**: `IOHIDFamily` / `NSGestureRecognizer` / `NSEvent` mouse handling
**Depends on**: R11 (compositor two-pass flush), R21 (window manager apps_map, win_x/y/w/h, z-order), R24 (dock space-0 occupants), R25 (Mission Control INPUT_LOCK_LEVEL, FOCUSED_APP), R26 (Stage Manager anim_x_override, stage_offscreen, from_virtual_kbd pattern), R27 (Split View FsTransition=4 lock level), R28 (focus/z-order, pending_focus_notifications, PENDING_COMPACT_SPACES), R31 (keyboard route_keyboard pipeline pattern, ModStateRef)
**Status**: DRAFT — Round 32

---

## 1. Overview

The mouse/trackpad subsystem converts raw `/dev/input/eventN` evdev events (mouse
`EV_REL`/`EV_KEY` and trackpad `EV_ABS` multi-touch slot protocol) into structured
`MouseEvent` values, performs z-order-aware hit testing against the live window
geometry from R21, applies a layered dispatch pipeline that respects
`INPUT_LOCK_LEVEL` from R25, recognises trackpad gestures (two-finger scroll,
pinch, three- and four-finger swipe, tap-to-click, force touch), and delivers
the result to space-0 chrome consumers (Dock hover, MC drag, Stage strip click),
to focused WASM apps via the line-oriented `VYOMA_INPUT:mouse:` protocol, or to
nobody at all (during a lock-level transition).

The design has six concerns, each addressed in its own section:

1. **Event acquisition** — evdev mouse thread (`mouse-evdev`) and trackpad
   thread (`trackpad-evdev`) feeding a shared `MouseEventQueue`.
2. **MouseEvent struct** — typed Rust value containing position (absolute
   screen coordinates), delta, buttons bitmap, scroll deltas, event kind, and
   gesture metadata.
3. **Cursor system** — `CURSOR_X`/`CURSOR_Y` `AtomicI32` pair, software cursor
   sprite composited in R11 pass 2, optional hardware cursor on virtio-gpu.
4. **Hit testing & dispatch** — z-order-aware, space-aware, lock-level-aware
   route from cursor coords to one of: space-0 chrome, focused app, no-op.
5. **Gesture state machine** — `TrackpadState` aggregates multi-touch slots
   across frames, emits gesture events when thresholds cross.
6. **WIT interface** — `vyoma:pointer@1.0.0` lets apps query cursor position
   and set the cursor shape from inside WASM.

---

## 2. Event Acquisition

### 2.1 Dual-Device Model

The supervisor reads from **two evdev devices**:

| Source | Purpose | Thread |
|--------|---------|--------|
| `/dev/input/eventN` (mouse) — `EV_REL` + `EV_KEY` | Relative motion, three buttons, scroll wheel | `mouse-evdev` (new) |
| `/dev/input/eventN` (trackpad) — `EV_ABS` MT slot protocol | Multi-touch positions, tap, gesture detection | `trackpad-evdev` (new) |

Both threads write into the same shared `MouseEventQueue`:

```rust
// supervisor/src/input_mouse.rs

pub type MouseEventQueue = Arc<Mutex<Vec<MouseEvent>>>;
```

Mirroring R31, the dispatch thread (`mouse-dispatch`) is the **only** consumer
of this queue and the **only** caller of `route_mouse`. Producer threads never
call dispatch directly; this keeps lock ordering acyclic
(`MouseEventQueue → CURSOR_X/Y → apps_map → FOCUSED_APP`).

### 2.2 Device Selection

```rust
// supervisor/src/input_mouse.rs

/// Scans /dev/input/event0..15 for a device with EV_REL bits set and at least
/// one of REL_X/REL_Y — distinguishes pointing devices from keyboards.
fn open_mouse_device() -> Option<File> { /* ... */ }

/// Scans for a device with EV_ABS bits and ABS_MT_SLOT capability — selects
/// the trackpad/touchpad (which reports MT positions, not relative motion).
fn open_trackpad_device() -> Option<File> { /* ... */ }
```

Note that some devices (e.g., Apple Magic Mouse) report **both** EV_REL and
ABS_MT. In that case, the device is opened twice and treated as a hybrid: motion
comes from the mouse thread, gesture detection from the trackpad thread reading
its `ABS_MT_*` channels.

### 2.3 mouse-evdev Thread

```rust
// supervisor/src/input_mouse.rs

#[cfg(target_os = "linux")]
pub fn run_mouse_evdev(mq: MouseEventQueue) {
    let Some(mut dev) = open_mouse_device() else {
        log_info!(Subsystem::Input, None, "mouse-evdev: no device, skipping");
        return;
    };
    let mut acc_dx: i32 = 0;
    let mut acc_dy: i32 = 0;
    let mut acc_scroll_v: i32 = 0;
    let mut acc_scroll_h: i32 = 0;
    let mut pending_buttons: u8 = 0;
    let mut current_buttons: u8 = 0;
    let mut buf = [0u8; 24];
    loop {
        if dev.read_exact(&mut buf).is_err() { break; }
        let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
        let code    = u16::from_ne_bytes([buf[18], buf[19]]);
        let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        match ev_type {
            EV_SYN => {
                // SYN_REPORT: flush an aggregated frame
                flush_mouse_frame(
                    &mq, &mut acc_dx, &mut acc_dy,
                    &mut acc_scroll_v, &mut acc_scroll_h,
                    &mut current_buttons, pending_buttons,
                );
                current_buttons = pending_buttons;
            }
            EV_REL => match code {
                REL_X       => acc_dx += value,
                REL_Y       => acc_dy += value,
                REL_WHEEL   => acc_scroll_v += value,
                REL_HWHEEL  => acc_scroll_h += value,
                _ => {}
            },
            EV_KEY => {
                let mask = match code {
                    BTN_LEFT   => 0b0000_0001,
                    BTN_RIGHT  => 0b0000_0010,
                    BTN_MIDDLE => 0b0000_0100,
                    _ => 0,
                };
                if mask != 0 {
                    if value != 0 { pending_buttons |= mask; }
                    else          { pending_buttons &= !mask; }
                }
            }
            _ => {}
        }
    }
}
```

The aggregation across `SYN_REPORT` boundaries matches Linux evdev convention:
a single frame may contain `REL_X`, `REL_Y`, several `EV_KEY` events, and
`REL_WHEEL`. Emitting one `MouseEvent` per `SYN_REPORT` avoids burst-flooding
the queue and matches the source semantics.

### 2.4 trackpad-evdev Thread

The trackpad uses the Linux multi-touch slot protocol (kernel
`Documentation/input/multi-touch-protocol.rst`). Each contact occupies a slot
(0..N); the kernel reports `ABS_MT_SLOT` to switch to a slot, then
`ABS_MT_TRACKING_ID` (-1 = release, ≥0 = unique id per contact),
`ABS_MT_POSITION_X`, `ABS_MT_POSITION_Y`.

```rust
const MAX_SLOTS: usize = 10;

#[derive(Clone, Copy, Default)]
struct TouchSlot {
    active:      bool,
    tracking_id: i32,
    x:           i32,
    y:           i32,
    pressure:    i32,
}

#[cfg(target_os = "linux")]
pub fn run_trackpad_evdev(mq: MouseEventQueue, gs: GestureStateRef) {
    let Some(mut dev) = open_trackpad_device() else {
        log_info!(Subsystem::Input, None, "trackpad-evdev: no device, skipping");
        return;
    };
    let mut slots: [TouchSlot; MAX_SLOTS] = Default::default();
    let mut active_slot: usize = 0;
    let mut buf = [0u8; 24];
    loop {
        if dev.read_exact(&mut buf).is_err() { break; }
        let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
        let code    = u16::from_ne_bytes([buf[18], buf[19]]);
        let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        match ev_type {
            EV_SYN => {
                // SYN_REPORT: hand the current slot snapshot to the gesture
                // recogniser, which decides what (if any) events to emit.
                let snapshot = slots;
                let mut g = gs.lock().unwrap();
                g.feed_frame(&snapshot, &mq);
            }
            EV_ABS => match code {
                ABS_MT_SLOT => {
                    active_slot = (value as usize).min(MAX_SLOTS - 1);
                }
                ABS_MT_TRACKING_ID => {
                    if value < 0 {
                        slots[active_slot].active = false;
                    } else {
                        slots[active_slot].active = true;
                        slots[active_slot].tracking_id = value;
                    }
                }
                ABS_MT_POSITION_X => slots[active_slot].x = value,
                ABS_MT_POSITION_Y => slots[active_slot].y = value,
                ABS_MT_PRESSURE   => slots[active_slot].pressure = value,
                _ => {}
            },
            _ => {}
        }
    }
}
```

The trackpad-evdev thread does **not** push raw move/click events to the queue
itself; it hands the slot snapshot to `GestureStateRef::feed_frame`, which is
the sole producer of all trackpad-derived `MouseEvent` and `GestureEvent`
values. This keeps gesture state machine atomic per frame.

For one-finger drag on the trackpad, `feed_frame` emits the same `MouseMove` /
`MouseButton(Left)` events as the mouse path; downstream code does not
distinguish trackpad from mouse for non-gesture events.

---

## 3. MouseEvent Struct

```rust
// supervisor/src/input_mouse.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MouseKind {
    Move    = 0,
    Press   = 1,
    Release = 2,
    Scroll  = 3,
    Enter   = 4,
    Leave   = 5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MouseButton {
    None   = 0,
    Left   = 1,
    Right  = 2,
    Middle = 3,
}

bitflags::bitflags! {
    #[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ButtonMask: u8 {
        const LEFT   = 0b0000_0001;
        const RIGHT  = 0b0000_0010;
        const MIDDLE = 0b0000_0100;
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub kind:        MouseKind,
    pub x:           i32,        // absolute screen coords (already accumulated)
    pub y:           i32,
    pub dx:          i32,        // delta since previous Move
    pub dy:          i32,
    pub button:      MouseButton, // which button changed (for Press/Release)
    pub button_mask: ButtonMask, // all currently-pressed buttons
    pub scroll_dx:   i32,        // signed; positive = right
    pub scroll_dy:   i32,        // signed; positive = up (natural scroll)
    pub modifiers:   u8,         // snapshot from ModStateRef (R31)
    pub source:      MouseSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MouseSource {
    Mouse        = 0,
    Trackpad     = 1,
    Touch        = 2,   // injected by R33 touch subsystem
    Synthetic    = 3,   // injected by tests or @supervisor: commands
}
```

The `source` field lets downstream code distinguish trackpad from mouse without
peeking at hardware. Touch events (R33) reuse this struct so the dispatch
pipeline is identical.

### 3.1 GestureEvent

Gesture events are emitted in addition to the equivalent `MouseEvent`s (which
may or may not be suppressed depending on the gesture; see §7):

```rust
#[derive(Debug, Clone, Copy)]
pub enum GestureEvent {
    ScrollBegin    { x: i32, y: i32 },
    Scroll         { x: i32, y: i32, dx: i32, dy: i32 },
    ScrollEnd      { x: i32, y: i32 },
    PinchBegin     { x: i32, y: i32 },
    Pinch          { x: i32, y: i32, scale: f32 },
    PinchEnd       { x: i32, y: i32, final_scale: f32 },
    SwipeThree     { dx: i32, dy: i32 },   // released after threshold cross
    SwipeFour      { dx: i32, dy: i32 },
    TapToClick     { x: i32, y: i32, fingers: u8 },
    ForceTouch     { x: i32, y: i32, pressure: f32 },
}
```

`SwipeFour` is the canonical Mission Control / Spaces gesture; it is intercepted
by space-0 chrome before any app sees it. `SwipeThree` is the
between-windows swipe (Stage Manager). See §7.5.

---

## 4. Cursor System

### 4.1 Cursor State

```rust
// supervisor/src/cursor.rs (new file, < 200 lines)

pub static CURSOR_X: AtomicI32 = AtomicI32::new(0);
pub static CURSOR_Y: AtomicI32 = AtomicI32::new(0);
pub static CURSOR_SHAPE: AtomicU8 = AtomicI32::new(0); // CursorShape enum
pub static CURSOR_VISIBLE: AtomicBool = AtomicBool::new(true);

#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CursorShape {
    Arrow     = 0,
    IBeam     = 1,
    Hand      = 2,
    Crosshair = 3,
    ResizeWE  = 4,
    ResizeNS  = 5,
    ResizeNWSE= 6,
    ResizeNESW= 7,
    Wait      = 8,
    NotAllowed= 9,
}
```

Position is two **separate** `AtomicI32` values rather than one packed `u64`
because writers from `mouse-dispatch` only update both atomically with respect
to each other, never with respect to the compositor read; we tolerate one frame
of torn position (max 1 px per axis at 60 Hz) in exchange for lock-free reads.
See §10.3 for the safety argument.

### 4.2 Software Cursor Compositing

Inserted into R11's **pass 2** (post-flush, post-chrome) right before
`fb.flush()`. The cursor is the last thing drawn, so it always appears on top
of every other surface including space-0 chrome:

```rust
// supervisor/src/display.rs::flush() — pass 2 addition

fn flush(&mut self, apps_sorted: &[...]) {
    // ... pass 1: app surfaces in z-order (R11) ...
    // ... pass 2: chrome (dock, MC, etc.) ...

    // Cursor sprite (last layer)
    if CURSOR_VISIBLE.load(Ordering::Relaxed) {
        let cx = CURSOR_X.load(Ordering::Relaxed);
        let cy = CURSOR_Y.load(Ordering::Relaxed);
        let shape = CursorShape::from_u8(CURSOR_SHAPE.load(Ordering::Relaxed));
        draw_cursor_sprite(&mut self.fb, cx, cy, shape);
    }

    self.fb.flush();
}
```

`draw_cursor_sprite` is a 16×16 (or 32×32 at HiDPI) hardcoded bitmap per shape;
no dynamic allocation, no surface management. See §10.3 for the reasoning
about read ordering vs writes from `mouse-dispatch`.

### 4.3 Hardware Cursor (virtio-gpu)

If the framebuffer driver advertises virtio-gpu's `RESOURCE_ATTACH_CURSOR`
capability, the supervisor uses **hardware** cursor instead of software
compositing. The supervisor uploads cursor bitmaps once at startup
(`update_cursor_image`), then `move_cursor` is a single virtio command per
mouse move — cheap, and decoupled from the compositor loop.

Detection is via `Framebuffer::supports_hw_cursor() -> bool`; on `desktop-full`
QEMU it is typically false (Cocoa/SDL backends don't expose it), on real
virtio-gpu hardware it is typically true. The fallback to software cursor is
automatic and transparent.

---

## 5. Hit Testing

### 5.1 The hit_test Function

```rust
// supervisor/src/input_mouse.rs

/// Returns the app name whose surface contains the screen-coordinate point
/// (x, y), respecting z-order and space membership. Space-0 chrome occupants
/// are checked first.
pub fn hit_test(x: i32, y: i32, apps_map: &AppsMap) -> Option<String> {
    let snapshot: Vec<(String, AppState)> = {
        let m = apps_map.lock().unwrap();
        m.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    };
    // 1. Space-0 chrome occupants — always hit-tested first, regardless of
    //    lock level. Ordered by descending z so dock (65534) takes priority
    //    over MC (65533).
    let mut z_sorted: Vec<&(String, AppState)> = snapshot.iter()
        .filter(|(_, s)| s.space == 0)
        .collect();
    z_sorted.sort_by(|a, b| b.1.z.cmp(&a.1.z));
    for (name, state) in &z_sorted {
        if state.win_visible && rect_contains(&state, x, y) {
            return Some(name.to_string());
        }
    }
    // 2. App windows in the active space — sorted by descending z.
    let active_space = ACTIVE_SPACE.load(Ordering::Relaxed);
    let mut app_sorted: Vec<&(String, AppState)> = snapshot.iter()
        .filter(|(_, s)| s.space == active_space && s.win_visible)
        .collect();
    app_sorted.sort_by(|a, b| b.1.z.cmp(&a.1.z));
    for (name, state) in &app_sorted {
        if rect_contains(state, x, y) {
            return Some(name.to_string());
        }
    }
    None
}

fn rect_contains(s: &AppState, x: i32, y: i32) -> bool {
    let x0 = effective_x(s);   // applies anim_x_override / stage_offscreen
    let y0 = s.win_y;
    x >= x0 && x < x0 + s.win_w && y >= y0 && y < y0 + s.win_h
}

fn effective_x(s: &AppState) -> i32 {
    if let Some(ax) = s.anim_x_override { return ax; }
    if s.stage_offscreen { return -10_000; }   // off-screen, never hit
    s.win_x
}
```

### 5.2 Drag Capture

A button press latches the recipient app into `MOUSE_DRAG_CAPTURE`:

```rust
// supervisor/src/input_mouse.rs

pub static MOUSE_DRAG_CAPTURE: ArcSwap<Option<String>> =
    ArcSwap::from_pointee(None);
```

While capture is set, all subsequent `Move`/`Release`/`Scroll` events are
delivered to the captured app **regardless of hit test**, until the
corresponding button release frame is dispatched. This matches macOS NSEvent
semantics: a drag that starts inside a window continues to deliver events to
that window even when the cursor leaves the window or moves off-screen.

Capture is cleared on button release. If the captured app exits during a drag
(detected via missing entry in `apps_map`), capture is cleared on the next
event and the in-flight drag is silently dropped.

---

## 6. Dispatch Pipeline (route_mouse)

```rust
// supervisor/src/input_mouse.rs

/// Single-threaded entry point called only by mouse-dispatch.
pub fn route_mouse(ev: &MouseEvent) {
    // (a) Update cursor position from delta or absolute.
    update_cursor_position(ev);

    // (b) Lock-level gate.
    let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
    if lock == LL_MC || lock == LL_STAGE_SWITCH || lock == LL_FS_TRANSITION {
        // Allow chrome to consume; block apps.
        route_to_chrome_only(ev);
        return;
    }
    if lock == LL_CHROME_CONSENT {
        // Modal dialog open; only consent dialog (its app name) receives.
        if let Some(consent) = CHROME_CONSENT_OWNER.load().as_ref() {
            deliver_to_app(consent, ev);
        }
        return;
    }

    // (c) Drag capture takes precedence over hit test.
    let captured = MOUSE_DRAG_CAPTURE.load().as_ref().cloned();
    let target = if let Some(c) = captured { Some(c) } else {
        let am = APPS_MAP.clone();
        hit_test(ev.x, ev.y, &am)
    };

    // (d) Enter/Leave synthesis vs previous target.
    synth_enter_leave_if_needed(&target);

    // (e) Deliver.
    if let Some(t) = target {
        if is_space0_chrome(&t) {
            deliver_to_chrome(&t, ev);
        } else {
            deliver_to_app(&t, ev);
        }
    }

    // (f) If this is a Press on a non-captured target, set capture.
    if ev.kind == MouseKind::Press && captured.is_none() {
        MOUSE_DRAG_CAPTURE.store(Arc::new(target));
    }
    if ev.kind == MouseKind::Release && ev.button_mask.is_empty() {
        MOUSE_DRAG_CAPTURE.store(Arc::new(None));
    }
}
```

### 6.1 route_to_chrome_only

During `LL_MC`, the Mission Control app needs hover/drag for window thumbnails.
During `LL_STAGE_SWITCH`, the stage strip needs hover. During `LL_FS_TRANSITION`,
nothing receives input.

```rust
fn route_to_chrome_only(ev: &MouseEvent) {
    let owner = match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        LL_MC           => "mc",
        LL_STAGE_SWITCH => "stage-manager",
        LL_FS_TRANSITION => return,    // input freeze
        _ => return,
    };
    // Mouse events only reach owner if it is actually present in apps_map.
    if APPS_MAP.lock().unwrap().contains_key(owner) {
        deliver_to_app(owner, ev);
    }
}
```

### 6.2 Enter/Leave Synthesis

```rust
static LAST_HOVER_TARGET: ArcSwap<Option<String>> = ArcSwap::from_pointee(None);

fn synth_enter_leave_if_needed(target: &Option<String>) {
    let prev = LAST_HOVER_TARGET.load();
    if prev.as_ref().as_ref() == target.as_ref() { return; }
    if let Some(p) = prev.as_ref() {
        deliver_synthetic(p, MouseKind::Leave);
    }
    if let Some(t) = target {
        deliver_synthetic(t, MouseKind::Enter);
    }
    LAST_HOVER_TARGET.store(Arc::new(target.clone()));
}
```

Enter/Leave events are synthesised from the dispatch loop, never from raw
evdev. This means an app sees exactly one Enter for the first event after the
cursor crosses into its bounds, and one Leave when it crosses out.

---

## 7. Gesture Recognition

### 7.1 GestureState

```rust
// supervisor/src/gesture.rs (< 350 lines)

pub type GestureStateRef = Arc<Mutex<GestureState>>;

pub struct GestureState {
    last_slots:        [TouchSlot; MAX_SLOTS],
    in_progress:       Option<GestureKind>,
    scroll_origin:     (i32, i32),
    pinch_initial_d:   i32,
    swipe_start:       (i32, i32),
    tap_start_time_ms: u64,
    tap_max_distance:  i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GestureKind {
    Scroll,
    Pinch,
    SwipeThree,
    SwipeFour,
    Drag,
}
```

### 7.2 feed_frame

```rust
impl GestureState {
    pub fn feed_frame(&mut self, slots: &[TouchSlot; MAX_SLOTS], mq: &MouseEventQueue) {
        let active: Vec<&TouchSlot> = slots.iter().filter(|s| s.active).collect();
        match (active.len(), self.in_progress) {
            (0, Some(g)) => self.end_gesture(g, mq),
            (1, None)    => self.start_drag(active[0], mq),
            (2, None)    => self.start_scroll_or_pinch(&active, mq),
            (3, None)    => self.maybe_start_swipe(GestureKind::SwipeThree, &active),
            (4, None)    => self.maybe_start_swipe(GestureKind::SwipeFour, &active),
            (_, Some(GestureKind::Scroll))    => self.continue_scroll(&active, mq),
            (_, Some(GestureKind::Pinch))     => self.continue_pinch(&active, mq),
            (_, Some(GestureKind::SwipeThree))=> self.continue_swipe(GestureKind::SwipeThree, &active),
            (_, Some(GestureKind::SwipeFour)) => self.continue_swipe(GestureKind::SwipeFour, &active),
            _ => {}
        }
        self.last_slots = *slots;
    }
}
```

### 7.3 Scroll vs Pinch (Two-Finger Disambiguation)

When two fingers touch down, the gesture is ambiguous until enough motion
accumulates. The state machine uses the **rate of change of inter-finger
distance** vs **mean translation**:

- If `|Δdistance| > 2 * |Δmean|` over the first 5 frames → Pinch.
- Otherwise → Scroll.

The first 5 frames are buffered; once classification commits, all buffered
deltas are emitted as the resolved gesture. This prevents spurious
half-pinch-half-scroll outputs at the start of every two-finger touch.

### 7.4 Tap-to-Click

A tap is detected when:
- a single slot becomes active, then inactive, within `TAP_MAX_DURATION_MS`
  (default 200 ms);
- the max distance from start to release is `< TAP_MAX_DISTANCE` (default 10
  px in trackpad units).

The detected tap is converted into synthetic `Press`+`Release` events with
button = Left at the release position. The synthetic events carry
`MouseSource::Trackpad` so apps that want to disable tap-to-click can filter
on source.

For two-finger taps, the synthetic Press carries button = Right (secondary
click). For three-finger taps (Look Up on macOS), a dedicated `GestureEvent::
TapToClick { fingers: 3, .. }` is emitted instead of synthetic mouse events;
apps that opt into gestures handle it explicitly.

### 7.5 Swipe Three / Swipe Four

A three- or four-finger swipe is detected when the **mean** of all active
slots translates by more than `SWIPE_THRESHOLD` (default 80 trackpad units)
in any direction, all fingers move in roughly the same direction (cosine
similarity > 0.7), and no slot has lifted yet.

Once threshold is crossed:
- **SwipeFour** is consumed by space-0 chrome — never sent to an app.
  Horizontal swipe routes to `@stage-manager: switch-space-left/right`;
  vertical-up routes to `@mc: activate`.
- **SwipeThree** is consumed by space-0 chrome for stage strip / app switcher.
  See R26 / R29 for behavioural details.

Both gestures are *atomic*: no intermediate `Pinch` or `Scroll` events are
emitted while the swipe is in progress, and no `MouseEvent` deltas are
delivered to apps.

### 7.6 Force Touch

If `pressure > FORCE_TOUCH_THRESHOLD` (default 0.7 normalised, hardware-
specific calibration in `profile/profiles/<plat>.toml`) on a single-slot
touch that has remained stationary for `FORCE_TOUCH_MIN_MS` (default 150
ms), a `GestureEvent::ForceTouch` is emitted in addition to the underlying
press. Apps that opt into force touch receive it as a `VYOMA_INPUT:gesture:
force_touch:` line; apps that don't see only the standard press.

### 7.7 Gesture Interruption by Lock Level

If `INPUT_LOCK_LEVEL` transitions to a non-zero value while a gesture is in
progress, the gesture is aborted: `end_gesture` is invoked with synthetic
end-event semantics (e.g., `PinchEnd` at the current scale), the in-progress
state is cleared, and no further events from the still-touching fingers are
delivered until **all** slots release. This prevents a half-completed pinch
from leaking into the next app after a Mission Control activation.

See critique B3 for the race detail.

---

## 8. Lock Level Integration

| Lock level | Mouse behaviour |
|-----------|-----------------|
| `LL_NONE` (0) | Normal hit-test + dispatch. |
| `LL_MC` (1) | Only `mc` app receives mouse events. Cursor still moves and is composited. Gestures: SwipeFour-down dismisses MC. |
| `LL_CHROME_CONSENT` (2) | Only `CHROME_CONSENT_OWNER` receives. Cursor still moves. |
| `LL_STAGE_SWITCH` (3) | Only `stage-manager` receives. Cursor still moves. |
| `LL_FS_TRANSITION` (4) | **Input freeze**: no events delivered to any app; cursor position is still updated and rendered so it doesn't appear to lag. |

Note in particular that during `LL_FS_TRANSITION`, the cursor still tracks
physical mouse motion. This is required because the user expects the cursor
to follow their hand even while the screen is animating into a full-screen
state; freezing the cursor during the ~250 ms transition is more disorienting
than letting it move but ignoring clicks.

Space-0 chrome **always** receives hover/leave events for visual feedback (Dock
magnification, MC thumbnail hover) regardless of lock level — except for
`LL_FS_TRANSITION`. This is implemented in `route_to_chrome_only` returning
early when the lock level is `LL_FS_TRANSITION`.

---

## 9. VYOMA_INPUT:mouse: Protocol

### 9.1 Wire Format

Apps with `mouse = true` in `vyoma.toml` receive mouse events as line-oriented
stdin messages from the supervisor:

```
VYOMA_INPUT:mouse:move:<x>,<y>,<dx>,<dy>,<button_mask>,<modifiers>
VYOMA_INPUT:mouse:press:<x>,<y>,<button>,<button_mask>,<modifiers>
VYOMA_INPUT:mouse:release:<x>,<y>,<button>,<button_mask>,<modifiers>
VYOMA_INPUT:mouse:scroll:<x>,<y>,<dx>,<dy>,<modifiers>
VYOMA_INPUT:mouse:enter:<x>,<y>
VYOMA_INPUT:mouse:leave:<x>,<y>
```

Where:
- `<x>`, `<y>` are in **window-local** coordinates (relative to `win_x`,
  `win_y`); apps should never need to know their screen position.
- `<dx>`, `<dy>` are deltas in the same units.
- `<button>` is `0`=none, `1`=left, `2`=right, `3`=middle.
- `<button_mask>` is the OR of currently-pressed buttons as a decimal integer
  (matching `ButtonMask::bits()`).
- `<modifiers>` is `ModifierState::bits()` from R31 (Shift, Ctrl, Alt, Super).

### 9.2 Gesture Lines

Apps with `gestures = true` receive these in addition to `mouse:` lines:

```
VYOMA_INPUT:gesture:scroll_begin:<x>,<y>
VYOMA_INPUT:gesture:scroll:<x>,<y>,<dx>,<dy>
VYOMA_INPUT:gesture:scroll_end:<x>,<y>
VYOMA_INPUT:gesture:pinch_begin:<x>,<y>
VYOMA_INPUT:gesture:pinch:<x>,<y>,<scale_x1000>      # scale is f32 * 1000 as integer
VYOMA_INPUT:gesture:pinch_end:<x>,<y>,<final_scale_x1000>
VYOMA_INPUT:gesture:force_touch:<x>,<y>,<pressure_x1000>
```

Swipe gestures are **never** delivered to apps — they are consumed by chrome.

### 9.3 Ordering Guarantees

Within a single app's stdin stream, mouse events arrive in **strict temporal
order**, with the following invariants:

1. Every `Press` is followed by exactly one `Release` for the same button
   before any other `Press` for that button.
2. Exactly one `Enter` precedes any `Move`/`Press`/`Release`/`Scroll` event
   sequence; the sequence ends with exactly one `Leave`.
3. Drag capture preserves order: even after a drag leaves the window's
   bounds, the app continues receiving `Move`/`Release` events until the
   button releases. No `Leave` is sent until after the corresponding
   `Release`.
4. `gesture:*` lines for a given gesture are ordered `_begin` → repeated → 
   `_end`, with no other gesture lines interleaved.

These guarantees are upheld by the single-threaded dispatch model (only
`mouse-dispatch` calls `route_mouse`).

### 9.4 Setting Cursor Shape (App → Supervisor)

Apps signal cursor shape changes via stdout commands:

```
VYOMA_CURSOR:shape:<name>     # arrow|ibeam|hand|crosshair|resize_we|resize_ns|wait|...
VYOMA_CURSOR:hide
VYOMA_CURSOR:show
```

Cursor shape is **scoped to hover**: when the cursor enters the app's window,
its requested shape is applied; when it leaves, the shape reverts to `arrow`.
This means an app does not need to track Leave events to reset the cursor.

---

## 10. Threading & Lock-Safety

### 10.1 Thread Inventory

| Thread | Owns | Reads | Writes |
|--------|------|-------|--------|
| `mouse-evdev` | EV device fd | — | `MouseEventQueue` |
| `trackpad-evdev` | EV device fd | `GestureState` | `GestureState`, `MouseEventQueue` |
| `mouse-dispatch` | — | `MouseEventQueue`, `APPS_MAP`, `INPUT_LOCK_LEVEL`, `MOUSE_DRAG_CAPTURE`, `LAST_HOVER_TARGET` | `CURSOR_X/Y`, app stdin, `MOUSE_DRAG_CAPTURE`, `LAST_HOVER_TARGET` |
| `compositor` (existing, R11) | framebuffer | `CURSOR_X/Y`, `CURSOR_SHAPE`, `CURSOR_VISIBLE`, `APPS_MAP` | framebuffer |
| `ipc-broker` (existing) | app stdin pipes | — | app stdin pipes |

The dispatch thread is the only consumer of `MouseEventQueue`. The compositor
thread only **reads** cursor atomics; it does not write them. The dispatch
thread only **reads** `APPS_MAP` (under its own lock); it does not modify
window geometry (that is R21's window manager thread).

### 10.2 Lock Ordering

To prevent ABBA deadlock, locks are acquired in this strict global order:

```
INPUT_LOCK_LEVEL  (atomic, no lock)
↓
APPS_MAP          (Mutex<HashMap<String, AppState>>)
↓
MouseEventQueue   (Mutex<Vec<MouseEvent>>)
↓
GestureState      (Mutex<GestureState>)
↓
LAST_HOVER_TARGET (ArcSwap, no lock)
↓
CURSOR_X/Y        (atomics, no lock)
↓
app stdin write   (per-app write half)
```

`mouse-dispatch` never holds `APPS_MAP` across a write to an app stdin pipe;
hit_test takes the lock, copies what it needs into a local snapshot, and
releases it before delivery. This is the same discipline that R11's compositor
flush uses (snapshot under lock, render outside).

### 10.3 Cursor Position Read/Write Race

`mouse-dispatch` writes `CURSOR_X` then `CURSOR_Y` (or vice versa). The
compositor reads both, one after the other. There exists a 60-Hz window where
the compositor reads a half-updated position (new X, old Y). The maximum
displacement is one mouse delta — typically ≤ 5 px on a fast mouse, often 1 px
on a trackpad. This is **acceptable** because:

1. It manifests only as a single-frame visual discontinuity, never a wrong
   click target (clicks are dispatched in the same thread that writes the
   position).
2. It cannot cause a logical error since hit-testing is done in
   `mouse-dispatch` using the local `ev.x, ev.y`, never via the global
   atomics.
3. Tearing for one axis at 60 Hz is invisible to the human eye for typical
   pointer speeds.

The alternative — packing `(x,y)` into a single `AtomicU64` — has been
considered and rejected because it complicates the compositor read path (must
unpack) and offers no observable benefit over the tolerated tear.

### 10.4 GestureState Lock Discipline

`GestureState` is held only inside `trackpad-evdev`'s SYN_REPORT handler.
`feed_frame` may push to `MouseEventQueue` while holding the gesture mutex;
the queue mutex is acquired after the gesture mutex per §10.2.

`mouse-dispatch` **never** locks `GestureState`. It only reads from
`MouseEventQueue`. This means gesture state is invisible to the dispatch
pipeline; it can only see the synthesised mouse/gesture events that gesture
recognition produced.

---

## 11. WIT Interface — vyoma:pointer@1.0.0

```wit
package vyoma:pointer@1.0.0;

interface cursor {
    /// Get the current cursor position in screen coordinates.
    /// Coordinates may be stale by one frame.
    get-position: func() -> tuple<s32, s32>;

    /// Set the cursor shape while the cursor is over this app's window.
    /// Has no effect when the cursor is outside the window.
    set-shape: func(shape: shape);

    /// Hide the cursor entirely while it is over this app's window.
    /// Reverts to default visibility on Leave.
    hide: func();
    show: func();
}

enum shape {
    arrow, ibeam, hand, crosshair,
    resize-we, resize-ns, resize-nwse, resize-nesw,
    wait, not-allowed,
}

interface events {
    /// Read the next mouse event delivered to this app, or none if no event
    /// is currently buffered. This is the WIT alternative to reading the
    /// VYOMA_INPUT:mouse: lines from stdin.
    poll-event: func() -> option<mouse-event>;
}

variant mouse-event {
    move(move-payload),
    press(button-payload),
    release(button-payload),
    scroll(scroll-payload),
    enter,
    leave,
}
```

The WIT interface is **opt-in alternative** to the line protocol. Apps choose
either stdin lines or WIT; mixing both for the same app is unsupported. The
default for new SDK templates is WIT (typed, no parsing overhead). Legacy
apps using stdout/stdin continue working.

The supervisor implements both transports by routing the same `MouseEvent`
through both: when a WIT-enabled app is the target, its event is pushed into a
per-app `VecDeque<MouseEvent>` accessible from the WASM-side `poll-event`
host function; the stdin pipe path is bypassed for that app.

---

## 12. Platform Matrix

| Platform | Mouse hardware | Trackpad hardware | Cursor | Notes |
|----------|----------------|-------------------|--------|-------|
| `desktop-full` | yes (USB/virtio) | yes (laptop) | software or HW | full pipeline; gestures enabled |
| `mobile` | no | no (touch screen, see R33) | invisible | mouse-evdev and trackpad-evdev threads not started; cursor compositor pass elided; VYOMA_INPUT:mouse: events sourced from touch (R33) only on long-press |
| `server-headless` | no | no | invisible | no input threads; no cursor; `vyoma:pointer` returns errors |
| `iot-edge` | optional serial | no | software | mouse-evdev opens `/dev/ttyS0` with `mouse_serial` driver if config flag set; gestures disabled |
| `robotics-rt` | no | no | invisible | no input threads |
| `mcu-minimal` | no | no | n/a | subsystem not present at all (no input crate compiled) |

Platform detection is via `PLATFORM` cfg at build; threads that aren't needed
are not even started, avoiding wasted file descriptors and energy on systems
without the relevant hardware.

For `iot-edge` with optional serial mouse, the `mouse-evdev` thread is
replaced by `mouse-serial`, which speaks the Microsoft serial mouse protocol
(`R/X/Y/L/R` byte tuples) and writes into the same `MouseEventQueue`. The
rest of the pipeline is identical.

---

## 13. File Layout

```
supervisor/src/
├── input_keys.rs           (R31, existing)
├── input_mouse.rs          (new, ≤ 500 lines)
│   ├── MouseEvent / MouseKind / MouseButton / ButtonMask / MouseSource
│   ├── MouseEventQueue type
│   ├── run_mouse_evdev (thread body)
│   ├── run_trackpad_evdev (thread body)
│   ├── route_mouse (dispatch entry)
│   ├── hit_test / rect_contains / effective_x
│   ├── MOUSE_DRAG_CAPTURE / LAST_HOVER_TARGET
│   └── deliver_to_app / deliver_to_chrome / deliver_synthetic
├── gesture.rs              (new, ≤ 350 lines)
│   ├── TouchSlot
│   ├── GestureState / GestureStateRef
│   ├── GestureEvent / GestureKind
│   ├── feed_frame
│   ├── start_scroll_or_pinch / continue_scroll / continue_pinch
│   ├── maybe_start_swipe / continue_swipe
│   └── tap_to_click_check
├── cursor.rs               (new, ≤ 200 lines)
│   ├── CURSOR_X / CURSOR_Y / CURSOR_SHAPE / CURSOR_VISIBLE atomics
│   ├── CursorShape enum
│   ├── draw_cursor_sprite
│   ├── update_cursor_position
│   └── HW cursor virtio-gpu integration (if available)
└── display.rs              (R11, modified: cursor sprite drawn in pass 2)
```

Apps that consume the line protocol need no SDK changes; they parse
`VYOMA_INPUT:mouse:` from stdin as they parse other events. Apps that opt
into WIT get the new `vyoma:pointer@1.0.0` import.

---

## 14. Manifest Capability

```toml
# vyoma.toml
[capabilities]
mouse    = true            # receive VYOMA_INPUT:mouse: events
gestures = true            # additionally receive VYOMA_INPUT:gesture: events
cursor   = true            # may issue VYOMA_CURSOR:shape: commands
```

Default for all three is `false`. Mouse + gestures + cursor together replace
the single `mouse = true` capability from prior phases; older manifests
declaring just `mouse = true` continue working (cursor defaults to false,
gestures to false).

`gestures = true` requires `mouse = true`; the manifest validator rejects the
combination `mouse = false, gestures = true`.

---

## 15. Initialisation Sequence

In `supervisor::main::main`, after R21 window manager and R31 keyboard threads
are started:

```rust
// supervisor/src/main.rs

let mouse_q: MouseEventQueue = Arc::new(Mutex::new(Vec::new()));
let gesture_st: GestureStateRef = Arc::new(Mutex::new(GestureState::default()));

#[cfg(target_os = "linux")]
{
    let mq = mouse_q.clone();
    std::thread::Builder::new().name("mouse-evdev".into()).spawn(move || {
        input_mouse::run_mouse_evdev(mq);
    }).unwrap();

    let mq = mouse_q.clone();
    let gs = gesture_st.clone();
    std::thread::Builder::new().name("trackpad-evdev".into()).spawn(move || {
        input_mouse::run_trackpad_evdev(mq, gs);
    }).unwrap();
}

// Dispatch thread — single consumer.
let mq = mouse_q.clone();
std::thread::Builder::new().name("mouse-dispatch".into()).spawn(move || {
    loop {
        let evs: Vec<MouseEvent> = {
            let mut q = mq.lock().unwrap();
            std::mem::take(&mut *q)
        };
        for ev in evs {
            input_mouse::route_mouse(&ev);
        }
        std::thread::sleep(std::time::Duration::from_millis(2));   // 500 Hz poll
    }
}).unwrap();
```

The 2 ms poll keeps latency low (≤ 2 ms median) while bounding CPU at < 1% on
desktop-full. A condvar-driven wake from the producer threads is a follow-up
optimisation if poll cost becomes measurable.

---

## 16. Invariants & Test Plan

### 16.1 Invariants

1. `route_mouse` is called by **exactly one thread** (`mouse-dispatch`).
2. `hit_test` reads `APPS_MAP` under its lock and releases the lock before
   any I/O.
3. `CURSOR_X` / `CURSOR_Y` are written **only** by `mouse-dispatch`.
4. `MOUSE_DRAG_CAPTURE`, `LAST_HOVER_TARGET`, `CHROME_CONSENT_OWNER` are
   written only by `mouse-dispatch`.
5. An Enter event for app A is never followed by an Enter for app A without
   an intervening Leave.
6. Every Press is paired with exactly one Release.
7. Gestures (pinch, scroll) emit `_begin` / `_end` pairs; no `_end` is
   emitted without a preceding `_begin`.
8. While `INPUT_LOCK_LEVEL == LL_FS_TRANSITION`, no app receives any mouse
   event of any kind.
9. SwipeFour and SwipeThree gestures are never delivered to apps.
10. When a target app exits during a drag, `MOUSE_DRAG_CAPTURE` is cleared
    on the next event and no further events for that drag are delivered.

### 16.2 Unit Tests

- `tests/input_mouse_hit_test.rs`: hit testing with overlapping windows,
  z-order ties, occluded windows.
- `tests/input_mouse_drag_capture.rs`: drag continues across window
  boundaries; capture clears on release.
- `tests/input_mouse_enter_leave.rs`: synth Enter/Leave pairs across
  cursor moves.
- `tests/input_mouse_lock_levels.rs`: each lock level produces the right
  routing.
- `tests/gesture_scroll_pinch.rs`: two-finger disambiguation correctness.
- `tests/gesture_swipe.rs`: three- and four-finger swipe detection.
- `tests/gesture_tap_to_click.rs`: synthetic Press+Release from tap.
- `tests/gesture_interrupt.rs`: lock-level change mid-gesture aborts and
  emits proper end event.
- `tests/cursor_position_atomic.rs`: race-tolerance: cursor compositor
  reads converge to last-write within one frame.

### 16.3 Smoke Test

`make smoke-mouse`: boots VyomaOS in QEMU with a virtual mouse fed by a
test harness, generates 1000 randomised events, verifies no app deadlock,
no panic, and that the cursor reaches the expected final position within
±5 px.

---

## 17. Open Questions (resolved in critique pass)

- Q1: Cursor compositor read race — see §10.3, accepted with rationale.
- Q2: GestureState shared mutability — single-locker in `trackpad-evdev`,
  read-only snapshot pushed via queue.
- Q3: HW vs SW cursor selection — runtime detect, no app-visible difference.
- Q4: WIT vs stdin transport — opt-in per app, both supported.
- Q5: VYOMA_CURSOR shape lifetime — scoped to hover, auto-reset on Leave.

---

## 18. Out of Scope

- **Pen / stylus input**: deferred to R33 (touch); pen events would reuse
  `MouseEventQueue` with `MouseSource::Touch` and an extra pressure channel.
- **Mouse acceleration curves**: ships flat for v1; acceleration profile is
  a `profile/profiles/<plat>.toml` field deferred to Phase 19.
- **Multi-monitor cursor handoff**: deferred to R34 (multi-display); cursor
  coordinates are currently single-display screen-relative.
- **Cursor themes / app-supplied cursor bitmaps**: deferred; only built-in
  shapes for now.
- **Gesture customisation**: SwipeFour is hard-bound to MC and stage; user-
  configurable mappings deferred to Phase 20.
