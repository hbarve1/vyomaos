# FINAL Spec: Mouse, Trackpad & Gestures (Round 32)

**Subsystem**: Mouse, Trackpad & Gestures  
**macOS Analogue**: `IOHIDFamily` / `NSGestureRecognizer` / `NSEvent` mouse handling  
**Depends on**: R11 (compositor flush), R21 (apps_map, win_x/y/w/h, z-order), R24 (space-0), R25 (INPUT_LOCK_LEVEL, FOCUSED_APP), R26 (anim_x_override, stage_offscreen), R27 (FsTransition=4), R28 (focus/z-order), R31 (ModStateRef, route_keyboard pattern)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

The mouse/trackpad subsystem converts raw evdev events into structured `MouseEvent` values,
performs z-order-aware hit testing, applies a layered dispatch pipeline that respects
`INPUT_LOCK_LEVEL`, recognises trackpad gestures, and delivers events to space-0 chrome
consumers or focused WASM apps via `VYOMA_INPUT:mouse:` / `vyoma:pointer@1.0.0` WIT.

Six concerns:
1. **Event acquisition** — `mouse-evdev` + `trackpad-evdev` threads feeding `MouseEventQueue`
2. **Structured types** — `MouseEvent`, `GestureEvent`, `HitResult`
3. **Cursor system** — packed `AtomicU64` position, software/HW cursor compositing
4. **Hit testing & dispatch** — `route_mouse` single-threaded entry; `HitResult` collapses
   all `APPS_MAP` lock acquisition into one shot (B1 fix)
5. **Gesture state machine** — scroll/pinch disambiguation, swipe-3/4, tap-to-click, force touch
6. **WIT interface** — `vyoma:pointer@1.0.0`

---

## 2. Event Acquisition

### 2.1 Dual-Device Model

| Source | Purpose | Thread |
|--------|---------|--------|
| `/dev/input/eventN` (mouse, `EV_REL`+`EV_KEY`) | Relative motion, three buttons, scroll wheel | `mouse-evdev` |
| `/dev/input/eventN` (trackpad, `EV_ABS` MT slot) | Multi-touch positions, tap, gesture detection | `trackpad-evdev` |

Both threads write into:
```rust
pub type MouseEventQueue = Arc<Mutex<Vec<MouseEvent>>>;
```

The `mouse-dispatch` thread is the **only** consumer and the **only** caller of `route_mouse`.
Producer threads never call dispatch directly.

### 2.2 Device Selection

```rust
fn open_mouse_device()    -> Option<File>  { /* first device with EV_REL + REL_X/REL_Y */ }
fn open_trackpad_device() -> Option<File>  { /* first device with EV_ABS + ABS_MT_SLOT */ }
```

Hybrid devices (e.g., Apple Magic Mouse with both EV_REL and ABS_MT) are opened by both
threads; motion from the mouse path, gesture detection from the trackpad path.

### 2.3 mouse-evdev Thread

```rust
#[cfg(target_os = "linux")]
pub fn run_mouse_evdev(mq: MouseEventQueue) {
    let Some(mut dev) = open_mouse_device() else {
        log_info!(Subsystem::Input, None, "mouse-evdev: no device, skipping");
        return;
    };
    let mut acc_dx = 0i32; let mut acc_dy = 0i32;
    let mut acc_sv = 0i32; let mut acc_sh = 0i32;
    let mut pending_btns: u8 = 0;
    let mut current_btns: u8 = 0;
    let mut buf = [0u8; 24];
    loop {
        if dev.read_exact(&mut buf).is_err() { break; }
        let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
        let code    = u16::from_ne_bytes([buf[18], buf[19]]);
        let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        match ev_type {
            EV_SYN => flush_mouse_frame(&mq, &mut acc_dx, &mut acc_dy,
                                         &mut acc_sv, &mut acc_sh,
                                         &mut current_btns, pending_btns),
            EV_REL => match code {
                REL_X     => acc_dx += value,
                REL_Y     => acc_dy += value,
                REL_WHEEL => acc_sv += value,
                REL_HWHEEL=> acc_sh += value,
                _ => {}
            },
            EV_KEY => {
                let mask = match code {
                    BTN_LEFT   => 0b001u8,
                    BTN_RIGHT  => 0b010u8,
                    BTN_MIDDLE => 0b100u8,
                    _ => 0,
                };
                if mask != 0 {
                    if value != 0 { pending_btns |= mask; }
                    else          { pending_btns &= !mask; }
                }
            }
            _ => {}
        }
    }
}
```

`flush_mouse_frame` converts accumulated deltas into screen-clamped absolute coordinates
using `CURSOR_POS` (B4 fix; see §4.1), then pushes one `MouseEvent` per `SYN_REPORT`.
It also updates `CURRENT_BUTTONS` (B5 fix; see §6.4).

### 2.4 trackpad-evdev Thread

```rust
const MAX_SLOTS: usize = 10;

#[derive(Clone, Copy, Default)]
struct TouchSlot { active: bool, tracking_id: i32, x: i32, y: i32, pressure: i32 }

#[cfg(target_os = "linux")]
pub fn run_trackpad_evdev(mq: MouseEventQueue, gs: GestureStateRef) {
    let Some(mut dev) = open_trackpad_device() else {
        log_info!(Subsystem::Input, None, "trackpad-evdev: no device, skipping");
        return;
    };
    let mut slots: [TouchSlot; MAX_SLOTS] = Default::default();
    let mut active_slot = 0usize;
    let mut buf = [0u8; 24];
    loop {
        if dev.read_exact(&mut buf).is_err() { break; }
        let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
        let code    = u16::from_ne_bytes([buf[18], buf[19]]);
        let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);
        match ev_type {
            EV_SYN => {
                let mut g = gs.lock().unwrap();
                g.feed_frame(&slots, &mq);
            }
            EV_ABS => match code {
                ABS_MT_SLOT        => { active_slot = (value as usize).min(MAX_SLOTS - 1); }
                ABS_MT_TRACKING_ID => {
                    if value < 0 { slots[active_slot].active = false; }
                    else { slots[active_slot].active = true; slots[active_slot].tracking_id = value; }
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

`trackpad-evdev` never writes to `APPS_MAP` or calls `route_mouse`. It only pushes events
via `GestureState::feed_frame` → `MouseEventQueue`.

---

## 3. Structured Types

### 3.1 MouseEvent

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MouseKind {
    Move=0, Press=1, Release=2, Scroll=3, Enter=4, Leave=5,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MouseButton { None=0, Left=1, Right=2, Middle=3 }

bitflags::bitflags! {
    #[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
    pub struct ButtonMask: u8 {
        const LEFT   = 0b001;
        const RIGHT  = 0b010;
        const MIDDLE = 0b100;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum MouseSource { Mouse=0, Trackpad=1, Touch=2, Synthetic=3 }

#[derive(Debug, Clone, Copy)]
pub struct MouseEvent {
    pub kind:        MouseKind,
    pub x:           i32,          // absolute screen coords
    pub y:           i32,
    pub dx:          i32,          // delta since last Move
    pub dy:          i32,
    pub button:      MouseButton,
    pub button_mask: ButtonMask,
    pub scroll_dx:   i32,
    pub scroll_dy:   i32,
    pub modifiers:   u8,           // from ModStateRef (R31)
    pub source:      MouseSource,
    pub epoch:       u8,           // matches INPUT_EPOCH at event creation (B3 fix)
}
```

### 3.2 GestureEvent

```rust
#[derive(Debug, Clone, Copy)]
pub enum GestureEvent {
    ScrollBegin    { x: i32, y: i32 },
    Scroll         { x: i32, y: i32, dx: i32, dy: i32 },
    ScrollEnd      { x: i32, y: i32 },
    PinchBegin     { x: i32, y: i32 },
    Pinch          { x: i32, y: i32, scale: f32 },
    PinchEnd       { x: i32, y: i32, final_scale: f32 },
    SwipeThree     { dx: i32, dy: i32 },
    SwipeFour      { dx: i32, dy: i32 },
    TapToClick     { x: i32, y: i32, fingers: u8 },
    ForceTouch     { x: i32, y: i32, pressure: f32 },
}
```

### 3.3 HitResult (B1 Fix)

```rust
// supervisor/src/input_mouse.rs

/// Result of one hit_test call — carries everything needed for dispatch
/// without re-acquiring APPS_MAP.
pub struct HitResult {
    pub name:      String,
    pub stdin:     Arc<Mutex<ChildStdin>>,   // cloned under APPS_MAP lock
    pub is_space0: bool,
    pub win_x:     i32,     // window-local origin for coordinate conversion
    pub win_y:     i32,
    pub wit_inbox: Option<Arc<Mutex<VecDeque<MouseEvent>>>>,
}
```

`hit_test` returns `Option<HitResult>`. The `stdin` and `wit_inbox` arcs are cloned inside
the single `APPS_MAP` lock acquisition. `deliver_to_app` receives a `&HitResult` and
**never touches `APPS_MAP` again** — only the per-app stdin lock. This collapses the three
separate `APPS_MAP` acquisitions in the original spec (hit_test, is_space0_chrome,
deliver_to_app) into exactly one.

---

## 4. Cursor System (B4 Fix)

### 4.1 Packed Atomic Position

Two separate `AtomicI32` values create a torn-read hazard on weakly-ordered architectures
(ARM64 — `iot-edge`, `mobile`, `robotics-rt`): the compositor can observe `X_new, Y_old`
even when the writer wrote them in order, because Relaxed loads can be reordered on ARM.

**Fix**: pack X and Y into a single `AtomicU64`:

```rust
// supervisor/src/cursor.rs

pub static CURSOR_POS:     AtomicU64  = AtomicU64::new(0);
pub static CURSOR_SHAPE:   AtomicU8   = AtomicU8::new(0);
pub static CURSOR_VISIBLE: AtomicBool = AtomicBool::new(true);

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

A single `AtomicU64` store/load is one instruction on both x86_64 and ARM64. The
`Acquire/Release` pairing ensures the compositor always reads a position pair that
was written atomically by `mouse-dispatch`. The "1 px tear is acceptable" rationale
from the architect spec is **deleted** — consistency is free.

### 4.2 Software Cursor Compositing

Added to R11's pass 2 (post-flush, last layer before `fb.flush()`):

```rust
// supervisor/src/display.rs — flush() pass 2 addition

if CURSOR_VISIBLE.load(Ordering::Relaxed) {
    let (cx, cy) = cursor_xy();   // single Acquire load
    let shape = CursorShape::from_u8(CURSOR_SHAPE.load(Ordering::Relaxed));
    draw_cursor_sprite(&mut self.fb, cx, cy, shape);
}
```

`draw_cursor_sprite`: 16×16 hardcoded bitmap per shape; no allocation; no surface
management. HiDPI: 32×32 at `backing_scale = 2.0`.

### 4.3 Hardware Cursor (virtio-gpu)

If `Framebuffer::supports_hw_cursor()` is true, the supervisor uses virtio-gpu's
`RESOURCE_ATTACH_CURSOR` + `move_cursor` instead of software compositing. Detection is
runtime; fallback to software is automatic.

---

## 5. Hit Testing

### 5.1 hit_test — Single Lock, Full HitResult (B1 Fix)

```rust
// supervisor/src/input_mouse.rs

/// Takes APPS_MAP lock once, extracts everything needed for dispatch,
/// returns a HitResult with pre-cloned Arc refs. APPS_MAP is released
/// before any I/O or delivery.
pub fn hit_test(x: i32, y: i32, apps_map: &AppsMap) -> Option<HitResult> {
    let m = apps_map.lock().unwrap();

    // 1. Space-0 chrome (descending z — chrome=65535 first)
    let mut candidates: Vec<(&String, &AppState)> = m.iter()
        .filter(|(_, s)| s.space == 0 && s.win_visible)
        .collect();
    candidates.sort_by(|a, b| b.1.z.cmp(&a.1.z));
    for (name, state) in &candidates {
        if rect_contains(state, x, y) {
            return Some(HitResult {
                name:      name.to_string(),
                stdin:     state.stdin_writer.clone(),
                is_space0: true,
                win_x:     effective_x(state),
                win_y:     state.win_y,
                wit_inbox: state.wit_inbox.clone(),
            });
        }
    }

    // 2. Active-space apps (descending z)
    let active_space = ACTIVE_SPACE.load(Ordering::Relaxed);
    let mut apps: Vec<(&String, &AppState)> = m.iter()
        .filter(|(_, s)| s.space == active_space as u8 && s.win_visible)
        .collect();
    apps.sort_by(|a, b| b.1.z.cmp(&a.1.z));
    for (name, state) in &apps {
        if rect_contains(state, x, y) {
            return Some(HitResult {
                name:      name.to_string(),
                stdin:     state.stdin_writer.clone(),
                is_space0: false,
                win_x:     effective_x(state),
                win_y:     state.win_y,
                wit_inbox: state.wit_inbox.clone(),
            });
        }
    }
    None
    // APPS_MAP lock released here — before any delivery
}

fn rect_contains(s: &AppState, x: i32, y: i32) -> bool {
    let x0 = effective_x(s);
    x >= x0 && x < x0 + s.win_w && y >= s.win_y && y < s.win_y + s.win_h
}

fn effective_x(s: &AppState) -> i32 {
    if let Some(ax) = s.anim_x_override { return ax; }
    if s.stage_offscreen { return -10_000; }
    s.win_x
}
```

### 5.2 Drag Capture

```rust
pub static MOUSE_DRAG_CAPTURE: ArcSwap<Option<String>> = ArcSwap::from_pointee(None);
```

While capture is set, all `Move`/`Release`/`Scroll` events go to the captured app
regardless of hit test. Cleared on button release or on `INPUT_LOCK_LEVEL` rise (B5 fix).

---

## 6. Dispatch Pipeline

### 6.1 INPUT_EPOCH (B3 Fix)

```rust
/// Bumped every time INPUT_LOCK_LEVEL changes. Used to discard stale
/// gesture events that were queued before a lock-level rise.
pub static INPUT_EPOCH: AtomicU8 = AtomicU8::new(0);
```

Every `MouseEvent` carries `epoch: u8` set at event creation time. `route_mouse` discards
gesture events whose epoch does not match the current epoch.

### 6.2 route_mouse — Single Snapshot Dispatch (B1 + B2 Fix)

```rust
// supervisor/src/input_mouse.rs

pub fn route_mouse(ev: &MouseEvent) {
    // (a) Update cursor position atomically.
    let (sx, sy) = accumulate_position(ev);
    set_cursor_xy(sx, sy);

    // (b) Lock-level gate.
    let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
    match lock {
        LL_FS_TRANSITION => return,  // full freeze; cursor moves but no delivery
        LL_MC | LL_STAGE_SWITCH => {
            route_to_chrome_only(ev, lock);
            return;
        }
        LL_CHROME_CONSENT => {
            if let Some(ref owner) = *CHROME_CONSENT_OWNER.load() {
                deliver_to_stored(owner, ev);
            }
            return;
        }
        _ => {}
    }

    // (c) Discard stale gesture events from before the last epoch change (B3).
    let cur_epoch = INPUT_EPOCH.load(Ordering::Acquire);
    if ev.epoch != cur_epoch && is_gesture_kind(ev.kind) {
        return;
    }

    // (d) Single APPS_MAP acquisition for hit + coordinate conversion (B1 + B2).
    // We also snapshot window geometries for Enter/Leave coordinate conversion.
    let captured = MOUSE_DRAG_CAPTURE.load().as_ref().cloned();
    let hit: Option<HitResult> = if let Some(ref c) = captured {
        // Drag capture: still need win_x/win_y from APPS_MAP for coord conversion.
        let m = APPS_MAP.lock().unwrap();
        m.get(c.as_str()).map(|state| HitResult {
            name:      c.clone(),
            stdin:     state.stdin_writer.clone(),
            is_space0: state.space == 0,
            win_x:     effective_x(state),
            win_y:     state.win_y,
            wit_inbox: state.wit_inbox.clone(),
        })
        // APPS_MAP released here
    } else {
        hit_test(ev.x, ev.y, &APPS_MAP)
        // APPS_MAP released inside hit_test
    };

    // (e) Enter/Leave synthesis — gated on LL_NONE to avoid animation storms (B2).
    synth_enter_leave_if_needed(&hit, lock);

    // (f) Deliver using cloned Arc refs from HitResult — no APPS_MAP re-lock.
    if let Some(ref h) = hit {
        deliver_to_hit(h, ev);
    }

    // (g) Update drag capture from result of this event.
    if ev.kind == MouseKind::Press && captured.is_none() {
        if let Some(ref h) = hit {
            MOUSE_DRAG_CAPTURE.store(Arc::new(Some(h.name.clone())));
        }
    }
    if ev.kind == MouseKind::Release && ev.button_mask.is_empty() {
        MOUSE_DRAG_CAPTURE.store(Arc::new(None));
    }
}
```

### 6.3 Enter/Leave Suppression During Animations (B2 Fix)

Window animations (Stage Manager, split-view transitions) move `anim_x_override` multiple
pixels per compositor frame. At 500 Hz mouse poll, 8–10 move events arrive per compositor
frame. If hit_test runs on each, the cursor alternately lands on/off the animating window,
producing an Enter/Leave/Enter/Leave storm of thousands of events during a 250 ms animation.

**Fix**: Freeze hover target at any non-zero lock level. Synthesize one reconciliation
Enter/Leave when lock returns to `LL_NONE`.

```rust
static LAST_HOVER_TARGET: ArcSwap<Option<String>> = ArcSwap::from_pointee(None);

fn synth_enter_leave_if_needed(hit: &Option<HitResult>, lock: u8) {
    if lock != LL_NONE {
        // Hover target frozen during all transitions.
        return;
    }
    let prev = LAST_HOVER_TARGET.load();
    let target_name = hit.as_ref().map(|h| h.name.clone());
    if prev.as_ref().as_ref() == target_name.as_ref() { return; }

    // Emit Leave for old target using stored HitResult (already have stdin Arc).
    if let Some(ref p) = *prev {
        deliver_synthetic_by_name(p, MouseKind::Leave);
    }
    if let Some(ref h) = hit {
        deliver_synthetic(h, MouseKind::Enter);
    }
    LAST_HOVER_TARGET.store(Arc::new(target_name));
}

/// Called when INPUT_LOCK_LEVEL falls to LL_NONE — reconciles hover target.
pub fn on_input_unlock() {
    let target = hit_test_now();   // hit_test at current cursor position
    synth_enter_leave_if_needed(&target, LL_NONE);
}
```

`hit_test_now()` acquires `APPS_MAP` and calls `hit_test` with `cursor_xy()` — called from
the code that writes `INPUT_LOCK_LEVEL`, not from `mouse-dispatch`. Uses the same single-lock
discipline.

### 6.4 Drag Capture on Lock Rise / Fall (B5 Fix)

A `Release` event that arrives while `INPUT_LOCK_LEVEL != LL_NONE` is swallowed by
`route_to_chrome_only` or the lock gate. `MOUSE_DRAG_CAPTURE` is never cleared, and the
captured app never receives its `Release`. On `LL_NONE` re-entry, `route_mouse` still sees
capture set — the captured app receives spurious `Move` events with no button held.

**Fix 1**: Synthesize `Release` on lock-level rise.

```rust
/// Called in the same critical section that writes INPUT_LOCK_LEVEL to any non-zero value.
pub fn on_input_lock_rise() {
    if let Some(ref captured) = *MOUSE_DRAG_CAPTURE.load() {
        let (cx, cy) = cursor_xy();
        let synth = MouseEvent {
            kind:        MouseKind::Release,
            x: cx, y: cy, dx: 0, dy: 0,
            button:      MouseButton::Left,
            button_mask: ButtonMask::empty(),
            scroll_dx: 0, scroll_dy: 0,
            modifiers:   0,
            source:      MouseSource::Synthetic,
            epoch:       INPUT_EPOCH.load(Ordering::Acquire),
        };
        // deliver directly by name — APPS_MAP lock acquired inside, released before return
        deliver_to_app_by_name(captured, &synth);
        MOUSE_DRAG_CAPTURE.store(Arc::new(None));
    }
    INPUT_EPOCH.fetch_add(1, Ordering::Release);  // invalidate pre-rise gesture events
}
```

**Fix 2**: Belt-and-braces clear on lock-level fall if no buttons held.

```rust
/// CURRENT_BUTTONS: AtomicU8 — maintained by flush_mouse_frame from SYN_REPORT button mask.
pub static CURRENT_BUTTONS: AtomicU8 = AtomicU8::new(0);

/// Called when INPUT_LOCK_LEVEL falls to LL_NONE.
pub fn on_input_lock_fall() {
    if MOUSE_DRAG_CAPTURE.load().is_some() {
        if CURRENT_BUTTONS.load(Ordering::Acquire) == 0 {
            MOUSE_DRAG_CAPTURE.store(Arc::new(None));
        }
    }
}
```

`flush_mouse_frame` in `mouse-evdev` always stores `current_btns` into `CURRENT_BUTTONS`
after updating `CURSOR_POS`.

**Updated invariant §16.1 #6**: Every `Press` is paired with exactly one `Release`,
possibly **synthetic** when a lock-level rise interrupts the drag.

**Updated invariant §16.1 #10**: `MOUSE_DRAG_CAPTURE` is cleared on `INPUT_LOCK_LEVEL`
rise (with synthetic Release delivered to the captured app, if any).

### 6.5 deliver_to_hit (B1 Fix)

```rust
fn deliver_to_hit(hit: &HitResult, ev: &MouseEvent) {
    if !app_has_mouse_capability(&hit.name) { return; }

    // Coordinate conversion using win_x/win_y from HitResult (no APPS_MAP re-lock).
    let local_x = ev.x - hit.win_x;
    let local_y = ev.y - hit.win_y;

    if let Some(ref inbox) = hit.wit_inbox {
        inbox.lock().unwrap().push_back(*ev);
        return;
    }
    let line = format_mouse_line(ev.kind, local_x, local_y, ev.dx, ev.dy,
                                  ev.button, ev.button_mask, ev.scroll_dx,
                                  ev.scroll_dy, ev.modifiers);
    // per-app stdin lock — NOT APPS_MAP; no deadlock risk
    let _ = hit.stdin.lock().unwrap().write_all(line.as_bytes());
    // Note: if write blocks > 5ms (stuck app), log_warn and drop the event.
}
```

The per-app `stdin` mutex is acquired here, *after* `APPS_MAP` has been released. Lock order:
`APPS_MAP → per-app stdin` — acyclic, no ABBA risk with the compositor thread.

---

## 7. Gesture Recognition

### 7.1 GestureState (B3 Fix — aborted_ids field added)

```rust
// supervisor/src/gesture.rs

pub type GestureStateRef = Arc<Mutex<GestureState>>;

pub struct GestureState {
    last_slots:       [TouchSlot; MAX_SLOTS],
    in_progress:      Option<GestureKind>,
    aborted_ids:      Vec<i32>,    // tracking_ids to suppress post-abort (B3)
    scroll_origin:    (i32, i32),
    pinch_initial_d:  i32,
    swipe_start:      (i32, i32),
    tap_start_ms:     u64,
    tap_max_distance: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GestureKind { Scroll, Pinch, SwipeThree, SwipeFour, Drag }
```

### 7.2 feed_frame (B3 Fix — lock-level check + aborted_ids filtering)

```rust
impl GestureState {
    pub fn feed_frame(&mut self, slots: &[TouchSlot; MAX_SLOTS], mq: &MouseEventQueue) {
        let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);

        // Abort in-progress gesture on lock-level rise.
        if lock != LL_NONE && self.in_progress.is_some() {
            let end_ev = self.synthesize_end(slots);
            mq.lock().unwrap().push(end_ev);
            // Record all currently-active tracking_ids as aborted.
            for s in slots.iter().filter(|s| s.active) {
                if !self.aborted_ids.contains(&s.tracking_id) {
                    self.aborted_ids.push(s.tracking_id);
                }
            }
            self.in_progress = None;
        }

        // Garbage-collect aborted IDs once the finger lifts.
        self.aborted_ids.retain(|id| slots.iter().any(|s| s.active && s.tracking_id == *id));

        // Filter out aborted fingers before any dispatch.
        let active: Vec<&TouchSlot> = slots.iter()
            .filter(|s| s.active && !self.aborted_ids.contains(&s.tracking_id))
            .collect();

        match (active.len(), self.in_progress) {
            (0, Some(g)) => self.end_gesture(g, mq),
            (1, None)    => self.start_drag(active[0], mq),
            (2, None)    => self.start_scroll_or_pinch(&active, mq),
            (3, None)    => self.maybe_start_swipe(GestureKind::SwipeThree, &active),
            (4, None)    => self.maybe_start_swipe(GestureKind::SwipeFour, &active),
            (_, Some(GestureKind::Scroll))     => self.continue_scroll(&active, mq),
            (_, Some(GestureKind::Pinch))      => self.continue_pinch(&active, mq),
            (_, Some(GestureKind::SwipeThree)) => self.continue_swipe(GestureKind::SwipeThree, &active, mq),
            (_, Some(GestureKind::SwipeFour))  => self.continue_swipe(GestureKind::SwipeFour, &active, mq),
            _ => {}
        }
        self.last_slots = *slots;
    }
}
```

Each `MouseEvent` pushed to `mq` by `feed_frame` gets `epoch: INPUT_EPOCH.load(Ordering::Acquire)`.
In `route_mouse`, gesture events with stale epoch are silently dropped (§6.1).

### 7.3 Scroll vs Pinch (Two-Finger Disambiguation)

When two fingers touch down, buffer the first 5 frames:
- If `|Δdistance| > 2 * |Δmean|` → Pinch
- Otherwise → Scroll

All buffered deltas are emitted as the resolved gesture type. This prevents spurious
half-scroll-half-pinch at the start of every two-finger touch.

### 7.4 Tap-to-Click

Single-slot contact that:
- becomes inactive within `TAP_MAX_DURATION_MS` (default 200 ms)
- max displacement `< TAP_MAX_DISTANCE` (default 10 px trackpad units)

Emits synthetic `Press`+`Release` with `button = Left`, `source = Trackpad`. Two-finger tap
emits `button = Right`. Three-finger tap emits `GestureEvent::TapToClick { fingers: 3 }`.

### 7.5 Swipe Three / Four

Mean of all active slots translates `> SWIPE_THRESHOLD` (default 80 trackpad units) with
cosine similarity > 0.7:

- **SwipeFour**: intercepted by space-0 chrome — horizontal → stage switch, up → MC activate.
- **SwipeThree**: intercepted by space-0 chrome for stage strip / app switcher.

Neither is delivered to apps.

### 7.6 Force Touch

Single stationary slot with `pressure > FORCE_TOUCH_THRESHOLD` (normalised 0.0–1.0) held
for `> FORCE_TOUCH_MIN_MS` (150 ms). Emits `GestureEvent::ForceTouch` + standard press.
Threshold calibrated per-platform in `profile/profiles/<plat>.toml`.

---

## 8. Lock Level Integration

| Level | Mouse behaviour |
|-------|----------------|
| `LL_NONE` (0) | Normal hit-test + dispatch; Enter/Leave synthesis active |
| `LL_MC` (1) | Only `mc` receives; cursor moves; Enter/Leave frozen; gestures aborted on rise |
| `LL_CHROME_CONSENT` (2) | Only consent owner receives; cursor moves |
| `LL_STAGE_SWITCH` (3) | Only `stage-manager` receives; cursor moves; Enter/Leave frozen |
| `LL_FS_TRANSITION` (4) | Full freeze: no delivery; cursor tracks and renders; Enter/Leave frozen |

Cursor position is always updated regardless of lock level (`set_cursor_xy` called before
lock gate in `route_mouse`). This ensures the cursor appears to follow the hand during
all transitions.

---

## 9. VYOMA_INPUT:mouse: Protocol

### 9.1 Wire Format

Apps with `mouse = true` receive:

```
VYOMA_INPUT:mouse:move:<x>,<y>,<dx>,<dy>,<button_mask>,<modifiers>
VYOMA_INPUT:mouse:press:<x>,<y>,<button>,<button_mask>,<modifiers>
VYOMA_INPUT:mouse:release:<x>,<y>,<button>,<button_mask>,<modifiers>
VYOMA_INPUT:mouse:scroll:<x>,<y>,<scroll_dx>,<scroll_dy>,<modifiers>
VYOMA_INPUT:mouse:enter:<x>,<y>
VYOMA_INPUT:mouse:leave:<x>,<y>
```

All coordinates are window-local (relative to `win_x`, `win_y` from `HitResult`).

### 9.2 Gesture Lines (gestures = true)

```
VYOMA_INPUT:gesture:scroll_begin:<x>,<y>
VYOMA_INPUT:gesture:scroll:<x>,<y>,<dx>,<dy>
VYOMA_INPUT:gesture:scroll_end:<x>,<y>
VYOMA_INPUT:gesture:pinch_begin:<x>,<y>
VYOMA_INPUT:gesture:pinch:<x>,<y>,<scale_x1000>
VYOMA_INPUT:gesture:pinch_end:<x>,<y>,<final_scale_x1000>
VYOMA_INPUT:gesture:force_touch:<x>,<y>,<pressure_x1000>
```

Swipe gestures are **never** delivered to apps — consumed by chrome.

### 9.3 Ordering Invariants

1. Every `Press` is paired with exactly one `Release` (possibly synthetic).
2. Exactly one `Enter` precedes any event sequence; ends with exactly one `Leave`.
3. Drag delivers `Move`/`Release` to captured app even outside window bounds; `Leave`
   is sent only after the final `Release`.
4. Gesture events: `_begin` → repeated → `_end`, no interleaving.
5. No `Enter` for app A without an intervening `Leave` since the previous `Enter`.

### 9.4 VYOMA_CURSOR Protocol (App → Supervisor)

```
VYOMA_CURSOR:shape:<name>     # arrow|ibeam|hand|crosshair|resize_we|resize_ns|...
VYOMA_CURSOR:hide
VYOMA_CURSOR:show
```

Shape is scoped to hover; reverts to `arrow` on Leave.

---

## 10. WIT Interface `vyoma:pointer@1.0.0`

```wit
package vyoma:pointer@1.0.0;

interface cursor {
    get-position: func() -> tuple<s32, s32>;   // from CURSOR_POS AtomicU64 (B4)
    set-shape:    func(shape: shape);
    hide:         func();
    show:         func();
}

enum shape {
    arrow, ibeam, hand, crosshair,
    resize-we, resize-ns, resize-nwse, resize-nesw, wait, not-allowed,
}

interface events {
    poll-event: func() -> option<mouse-event>;
}

variant mouse-event {
    move(move-payload), press(button-payload), release(button-payload),
    scroll(scroll-payload), enter, leave,
}

world pointer { import cursor; import events; }
```

WIT is an opt-in alternative to the line protocol. The supervisor routes the same
`MouseEvent` through both: WIT-enabled apps push into a per-app `VecDeque<MouseEvent>`
accessible from `poll-event`; stdin line delivery is bypassed for that app.

---

## 11. Threading Model

```
Thread: mouse-evdev
  Reads EV_REL/EV_KEY from mouse device
  Calls flush_mouse_frame per SYN_REPORT → pushes MouseEvent (with epoch) to MouseEventQueue
  Updates CURRENT_BUTTONS after each frame (B5)
  Updates CURSOR_POS (set_cursor_xy) inside flush_mouse_frame

Thread: trackpad-evdev
  Reads EV_ABS MT slot events from trackpad device
  On SYN_REPORT: GestureState::feed_frame (holds GestureState lock; releases before any I/O)
  feed_frame pushes MouseEvent/GestureEvent to MouseEventQueue
  NEVER calls route_mouse, deliver_to_app, or read APPS_MAP

Thread: mouse-dispatch (sole consumer)
  Drains MouseEventQueue every 2ms
  Calls route_mouse for each event
  Maintains LAST_HOVER_TARGET, MOUSE_DRAG_CAPTURE
  Reads APPS_MAP (single acquisition per event, via hit_test)
  Writes per-app stdin (via cloned Arc from HitResult)

Thread: compositor (R11)
  Reads CURSOR_POS (cursor_xy() — single Acquire load)
  Reads CURSOR_SHAPE, CURSOR_VISIBLE
  Never writes cursor atomics
```

---

## 12. Lock-Safety Analysis (B1 + B4 Fixed)

| Lock | Acquired by | Held during |
|------|------------|-------------|
| `INPUT_LOCK_LEVEL` | Any thread (atomic) | Single load/store |
| `INPUT_EPOCH` | mouse-evdev, on_input_lock_rise (atomic) | Single inc/load |
| `CURSOR_POS` | mouse-dispatch/evdev (atomic u64, Release store) | Single store |
| `APPS_MAP` | mouse-dispatch (read, via hit_test) | Walk of window list — released before any stdin write |
| `GestureState` | trackpad-evdev only | One SYN_REPORT frame; MouseEventQueue acquired after (per §10.2 order) |
| `MouseEventQueue` | mouse-evdev, trackpad-evdev (push); mouse-dispatch (drain) | Single push or full drain |
| `MOUSE_DRAG_CAPTURE` | mouse-dispatch, on_input_lock_rise (ArcSwap) | Wait-free load/store |
| `LAST_HOVER_TARGET` | mouse-dispatch (ArcSwap) | Wait-free load/store |
| per-app stdin | mouse-dispatch (via HitResult.stdin Arc) | One write; acquired **after** APPS_MAP is released |

Lock order: `APPS_MAP → GestureState → MouseEventQueue → per-app stdin`. No ABBA risk.

---

## 13. Platform Matrix

| Platform | Mouse | Trackpad | Cursor | Notes |
|----------|-------|---------|--------|-------|
| `desktop-full` | yes | yes | SW or HW | Full pipeline; gestures enabled |
| `mobile` | no | no | invisible | Threads not started; touch handled by R33 |
| `server-headless` | no | no | invisible | No input threads; `vyoma:pointer` returns errors |
| `iot-edge` | optional serial | no | SW | `mouse-serial` driver replaces `mouse-evdev`; gestures disabled |
| `robotics-rt` | no | no | invisible | No input threads |
| `mcu-minimal` | no | no | n/a | Subsystem not compiled |

`VMIN=0, VTIME=1` does not apply here — evdev reads block natively; `mouse-dispatch` polls
the queue every 2 ms regardless.

---

## 14. Capability Summary

| Field | Type | Grants |
|-------|------|--------|
| `mouse` | bool | Receive `VYOMA_INPUT:mouse:` events + `vyoma:pointer` WIT |
| `gestures` | bool | Receive `VYOMA_INPUT:gesture:` events (requires `mouse = true`) |
| `cursor` | bool | May send `VYOMA_CURSOR:shape/hide/show` commands |

---

## 15. File Layout

```
supervisor/src/
├── input_mouse.rs     — MouseEvent, MouseKind, MouseButton, ButtonMask, MouseSource,
│                         HitResult (B1), MouseEventQueue, run_mouse_evdev,
│                         run_trackpad_evdev, route_mouse, hit_test, MOUSE_DRAG_CAPTURE,
│                         LAST_HOVER_TARGET, CURRENT_BUTTONS (B5), deliver_to_hit,
│                         on_input_lock_rise (B5), on_input_lock_fall (B5),
│                         on_input_unlock (B2), synth_enter_leave_if_needed,
│                         INPUT_EPOCH (B3)
├── gesture.rs         — TouchSlot, GestureState (+ aborted_ids B3), GestureStateRef,
│                         GestureEvent, GestureKind, feed_frame, scroll/pinch/swipe/tap
├── cursor.rs          — CURSOR_POS: AtomicU64 (B4), cursor_xy, set_cursor_xy,
│                         CURSOR_SHAPE, CURSOR_VISIBLE, CursorShape, draw_cursor_sprite,
│                         HW cursor virtio-gpu integration
├── display.rs         — modified: cursor sprite draw in pass 2 using cursor_xy()
└── manifest.rs        — add mouse: bool, gestures: bool, cursor: bool to capabilities

supervisor/src/wit/pointer.wit   — vyoma:pointer@1.0.0

supervisor/tests/
├── input_mouse_hit_test.rs
├── input_mouse_drag_capture.rs      — includes lock-rise synthetic Release (B5)
├── input_mouse_enter_leave.rs       — includes animation suppression (B2)
├── input_mouse_lock_levels.rs
├── gesture_scroll_pinch.rs
├── gesture_swipe.rs
├── gesture_tap_to_click.rs
└── gesture_interrupt.rs             — aborted_ids + epoch stale-event drop (B3)
```

---

## 16. Invariants

1. `route_mouse` called by exactly one thread (`mouse-dispatch`).
2. `APPS_MAP` acquired at most once per `route_mouse` call (inside `hit_test`); released before any per-app stdin write.
3. `CURSOR_POS` is an `AtomicU64`; a single Acquire load always yields a consistent (x, y) pair (B4).
4. `MOUSE_DRAG_CAPTURE` and `LAST_HOVER_TARGET` written only by `mouse-dispatch` or `on_input_lock_rise`.
5. Enter for app A never follows Enter for A without intervening Leave.
6. Every Press paired with exactly one Release (possibly synthetic on lock-level rise) (B5).
7. Gesture events (`Pinch`, `Scroll`) emit `_begin`/`_end` pairs; no `_end` without `_begin`.
8. During `LL_FS_TRANSITION`, no app receives any event; cursor position still updates.
9. `SwipeFour` and `SwipeThree` never delivered to apps.
10. `MOUSE_DRAG_CAPTURE` cleared on `INPUT_LOCK_LEVEL` rise with synthetic Release delivered (B5).
11. During non-zero lock levels, hover target is frozen; one reconciliation Enter/Leave emitted on return to `LL_NONE` (B2).
12. Gesture events with stale `epoch` are silently dropped in `route_mouse` (B3).

---

## 17. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: `deliver_to_app` re-locks `APPS_MAP` during stdin write — violates lock invariant; stuck app stalls compositor | `HitResult` struct returned from `hit_test` with pre-cloned `stdin: Arc<Mutex<ChildStdin>>`, `is_space0: bool`, `win_x/y`; `deliver_to_hit` uses the cloned Arc, never re-acquires `APPS_MAP`; per-app stdin timeout prevents stall |
| B2: Hit-test snapshot stale across `synth_enter_leave_if_needed` — window animations produce Enter/Leave storms (thousands per 250ms) | Single `APPS_MAP` snapshot covers both hit_test and coordinate conversion; `synth_enter_leave_if_needed` returns immediately when `lock != LL_NONE`; `on_input_unlock()` synthesizes one reconciliation event on `LL_NONE` re-entry |
| B3: Gesture abort mid-gesture has two races — queued pre-abort events reach wrong app; lock level checked only per-SYN_REPORT | `aborted_ids: Vec<i32>` in `GestureState` suppresses still-active fingers post-abort; `INPUT_EPOCH: AtomicU8` bumped on every lock-level change; `route_mouse` drops gesture events with stale epoch |
| B4: `CURSOR_X/CURSOR_Y` two separate `AtomicI32` torn-read on ARM64 — compositor can observe `(X_new, Y_old)` pair that never existed | Pack into single `CURSOR_POS: AtomicU64`; `cursor_xy()` / `set_cursor_xy()` with Acquire/Release; one atomic op on both x86_64 and ARM64; "1 px tear is acceptable" rationale deleted |
| B5: `MOUSE_DRAG_CAPTURE` never cleared when Release arrives during lock level — phantom drag persists after MC dismissal | `on_input_lock_rise()` synthesizes Release for captured app + clears capture + bumps `INPUT_EPOCH`; `on_input_lock_fall()` clears capture if `CURRENT_BUTTONS == 0`; `CURRENT_BUTTONS: AtomicU8` maintained by `flush_mouse_frame` |
