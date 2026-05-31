# FINAL Spec: Controller & Gamepad Input (Round 37)

**Subsystem**: Controller & Gamepad Input  
**macOS Analogue**: GameController framework / MFi controllers  
**Depends on**: R31 (INPUT_LOCK_LEVEL, INPUT_EPOCH), R32 (route_mouse, on_input_lock_rise), R35 (SPSC delivery channels)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

The supervisor owns a **controller hub thread** that polls evdev fds via epoll,
normalizes events into a `ControllerState` model, and delivers
`VYOMA_INPUT:controller:*` lines via the R35 per-app SPSC channel pattern.

Up to 4 controllers bind to player slots P1–P4. P1 follows `FOCUSED_APP` by default;
P2–P4 may be claimed by apps declaring `controller_max_players > 1`.

Key invariants:
- Hub thread NEVER holds `ChildStdin` mutex — SPSC delivery only (R35 pattern).
- Events emitted only on `SYN_REPORT`, not per-axis (B2 fix).
- Disconnect delivered to `last_routed_app[slot]`, bypassing lock gate (B1 fix).
- `on_input_lock_rise()` emits all-zero synthetic state for held buttons (B3 fix).
- Each controller device opened **twice** — read fd (hub epoll) + write fd (rumble writer) (B4 fix).
- `release_controller_claims(app)` called from process-exit hook (B5 fix).

---

## 2. Controller Discovery

**Mechanism**: epoll on `/dev/input/event*` (opened read+write for read fd; second open
for write fd). Runtime discovery via `/dev/input` inotify watch (CREATE/DELETE).

**Identification**: `EVIOCGBIT` checks for `EV_KEY + EV_ABS + BTN_GAMEPAD`. `EVIOCGID`
provides `(bus, vendor, product, version)` for slot persistence.

```rust
// supervisor/src/controller/discovery.rs

struct ControllerDevice {
    read_fd:          RawFd,           // B4: hub epoll uses this
    write_fd:         RawFd,           // B4: rumble writer uses this (separate open)
    devnode:          PathBuf,
    vendor:           u16,
    product:          u16,
    name:             String,
    player:           PlayerSlot,
    rumble_effect_id: Option<i16>,     // lazy FF_RUMBLE upload
    rumble_tx:        mpsc::SyncSender<RumbleCmd>,  // to per-controller writer task
    rumble_shutdown:  oneshot::Sender<()>,
}
```

**Slot persistence**: `CONTROLLER_SLOT_MEMORY: Mutex<HashMap<(u16,u16,PathBuf), PlayerSlot>>`
keyed by `(vendor, product, devnode)`. Same slot returned on reconnect.

---

## 3. Input State Model

```rust
// supervisor/src/controller/state.rs

#[derive(Clone, Default)]
pub struct ControllerState {
    pub player:    PlayerSlot,
    pub buttons:   u32,      // ControllerButton bitmask
    pub lx: i16, pub ly: i16,  // left stick, -32768..32767 after deadzone
    pub rx: i16, pub ry: i16,  // right stick
    pub lt: u8,  pub rt: u8,   // triggers, 0..255 after deadzone
    pub seq:      u64,         // monotonic per-controller sequence
    pub epoch:    u8,          // INPUT_EPOCH at emit time
}

#[repr(u32)]
pub enum ControllerButton {
    A=1<<0, B=1<<1, X=1<<2,  Y=1<<3,
    LB=1<<4, RB=1<<5,
    Back=1<<6, Start=1<<7, Guide=1<<8,
    LStick=1<<9, RStick=1<<10,
    DUp=1<<11, DDown=1<<12, DLeft=1<<13, DRight=1<<14,
}
```

**Normalization** (applied only on SYN_REPORT — B2 fix):
- Sticks: radial deadzone 8% (~2621/32767), linear rescale from deadzone edge to max.
- Triggers: floor deadzone 4% (~10/255), rescale to full 0–255.
- State emitted only when button mask diff OR axis change >256 OR trigger change >4 (hysteresis).

---

## 4. SYN_REPORT Batching (B2 Fix)

```rust
// supervisor/src/controller/state.rs

pub struct ControllerHubEntry {
    scratch:      ControllerState,  // accumulates raw changes since last SYN_REPORT
    published:    ControllerState,  // last emitted state; used for hysteresis
    // ...
}

pub fn on_input_event(entry: &mut ControllerHubEntry, ev: &input_event) {
    match ev.type_ {
        EV_ABS => update_scratch_axis(&mut entry.scratch, ev),
        EV_KEY => update_scratch_buttons(&mut entry.scratch, ev),
        EV_SYN if ev.code == SYN_REPORT => {
            // B2 fix: normalize and emit ONLY at SYN_REPORT boundary
            normalize_axes(&mut entry.scratch);
            if has_significant_change(&entry.scratch, &entry.published) {
                let prev_buttons = entry.published.buttons;
                entry.scratch.seq += 1;
                entry.scratch.epoch = INPUT_EPOCH.load(Ordering::Acquire);
                entry.published = entry.scratch.clone();
                emit_state_event(&entry.published, prev_buttons);
            }
        }
        _ => {}
    }
}
```

---

## 5. Routing

```rust
// supervisor/src/controller/routing.rs

// B1 fix: track last successfully routed app per slot
pub static LAST_ROUTED_APP: [ArcSwap<Option<String>>; 4] = /* ... */;

// B5 fix: claims held per slot
pub static CONTROLLER_CLAIMS: RwLock<HashMap<PlayerSlot, String>> = ...;

pub fn route_controller_event(player: PlayerSlot, state: &ControllerState) -> Option<String> {
    if player == PlayerSlot::P1 && VIRTUAL_CURSOR_ENABLED.load(Ordering::Acquire) {
        translate_to_mouse(state); // → R32 MouseEventQueue
        return None;
    }
    let lock = INPUT_LOCK_LEVEL.load(Ordering::Acquire);
    if lock != LOCK_NONE {
        return None; // dropped during MC/ChromeConsent/StageSwitch/FsTransition
    }
    if let Some(app) = CONTROLLER_CLAIMS.read().unwrap().get(&player).cloned() {
        return Some(app);
    }
    if player == PlayerSlot::P1 {
        let focused = FOCUSED_APP.load().as_ref().clone();
        if app_has_controller_cap(&focused) {
            return Some(focused);
        }
    }
    None
}

fn emit_state_event(state: &ControllerState, prev_buttons: u32) {
    let dest = route_controller_event(state.player, state);
    // B1 fix: update last_routed_app whenever we successfully deliver
    if let Some(ref app) = dest {
        LAST_ROUTED_APP[state.player as usize].store(Arc::new(Some(app.clone())));
        let line = format_state_line(state);
        send_to_app_spsc(app, line);
        // Also emit button edge events
        let pressed  = state.buttons & !prev_buttons;
        let released = prev_buttons & !state.buttons;
        for btn in iter_buttons(pressed) {
            send_to_app_spsc(app, format_button_line(state, btn, true));
        }
        for btn in iter_buttons(released) {
            send_to_app_spsc(app, format_button_line(state, btn, false));
        }
    }
}
```

---

## 6. Disconnect Handling (B1 Fix)

```rust
// supervisor/src/controller/mod.rs

fn on_controller_disconnect(slot: PlayerSlot) {
    // B1 fix: deliver disconnect to LAST_ROUTED_APP, bypassing lock gate and
    // current routing computation entirely
    let last = LAST_ROUTED_APP[slot as usize].load();
    if let Some(ref app) = *last {
        let line = format!("VYOMA_INPUT:controller:disconnect:{}\n", slot as u8);
        send_to_app_spsc(app, line);
    }
    // Clear the last-routed tracking
    LAST_ROUTED_APP[slot as usize].store(Arc::new(None));
    // Drop any claims for this slot (B5)
    CONTROLLER_CLAIMS.write().unwrap().remove(&slot);
    // Shutdown rumble writer (B4)
    if let Some(dev) = HUB.controllers[slot as usize].take() {
        let _ = dev.rumble_shutdown.send(());
        unsafe { libc::close(dev.write_fd); }
        unsafe { libc::close(dev.read_fd); }
    }
}

fn on_controller_connect(slot: PlayerSlot, dev: &ControllerDevice) {
    // Connection events bypass lock gate — deliver to current routing destination
    let dest = route_controller_event(slot, &ControllerState::idle(slot));
    if let Some(ref app) = dest {
        let line = format_connect_line(slot, dev);
        send_to_app_spsc(app, line);
        LAST_ROUTED_APP[slot as usize].store(Arc::new(Some(app.clone())));
    }
}
```

---

## 7. Lock-Rise Button Flush (B3 Fix)

`on_input_lock_rise()` (established R32) is extended:

```rust
// supervisor/src/controller/routing.rs

pub fn flush_controller_state_on_lock_rise() {
    // B3 fix: emit all-zero synthetic state so apps release held buttons
    let epoch = INPUT_EPOCH.load(Ordering::Acquire); // already bumped by caller
    for slot in [PlayerSlot::P1, PlayerSlot::P2, PlayerSlot::P3, PlayerSlot::P4] {
        let last = LAST_ROUTED_APP[slot as usize].load();
        if let Some(ref app) = *last {
            // Bump seq so pre-lock buffered events are recognizable as stale
            let seq = HUB.entries[slot as usize].published.seq + 1;
            let line = format!(
                "VYOMA_INPUT:controller:state:{}:{}:{}:00000000:0:0:0:0:0:0\n",
                slot as u8, seq, epoch
            );
            send_to_app_spsc(app, line);
        }
    }
}
```

Called from `on_input_lock_rise()` after `INPUT_EPOCH` is incremented.

---

## 8. Rumble — Dual Fd (B4 Fix)

```rust
// supervisor/src/controller/rumble.rs

// B4 fix: separate write fd; hub read fd never used for ioctl/write
fn open_controller_fds(devnode: &Path) -> (RawFd, RawFd) {
    let read_fd = libc::open(devnode as *const _, O_RDONLY | O_NONBLOCK);
    let write_fd = libc::open(devnode as *const _, O_RDWR | O_NONBLOCK);
    (read_fd, write_fd) // two independent fds to same device
}

// Per-controller writer task: owns write_fd exclusively
fn rumble_writer_task(write_fd: RawFd, mut rx: mpsc::Receiver<RumbleCmd>, shutdown: oneshot::Receiver<()>) {
    let mut effect_id: Option<i16> = None;
    loop {
        select! {
            cmd = rx.recv() => {
                let RumbleCmd { low, high, duration_ms } = cmd;
                let duration_ms = duration_ms.min(5000); // clamp
                let effect = ff_effect { /* FF_RUMBLE struct */ };
                if effect_id.is_none() {
                    // Lazy upload
                    let id = ioctl_eviocsff(write_fd, &effect);
                    effect_id = Some(id);
                } else {
                    // Re-upload to update magnitudes
                    ioctl_eviocsff_update(write_fd, effect_id.unwrap(), &effect);
                }
                let ev = input_event { type_: EV_FF, code: effect_id.unwrap() as u16, value: 1 };
                write(write_fd, &ev);
            }
            _ = shutdown => break,
        }
    }
    if let Some(id) = effect_id {
        ioctl_eviocrmff(write_fd, id);
    }
    unsafe { libc::close(write_fd); }
}
```

**Capacity**: rumble mpsc bounded to 4. Overflow drops oldest, keeping latest.

---

## 9. App-Exit Claim Cleanup (B5 Fix)

```rust
// supervisor/src/process.rs — process exit hook

pub fn on_app_exit(app_name: &str) {
    // ... existing SPSC cleanup, window state, ACTIVE_IME, hotkeys ...

    // B5 fix: release all controller claims held by this app
    release_controller_claims(app_name);
}

// supervisor/src/controller/routing.rs
pub fn release_controller_claims(app: &str) {
    let released: Vec<PlayerSlot> = {
        let mut claims = CONTROLLER_CLAIMS.write().unwrap();
        let released: Vec<_> = claims.iter()
            .filter(|(_, v)| v.as_str() == app)
            .map(|(k, _)| *k)
            .collect();
        for slot in &released { claims.remove(slot); }
        released
    };
    // For restart=always: re-emit connect to the newly-respawned app on restart
    // (handled by the restart path: after spawn, call on_controller_connect for each
    //  previously-claimed slot that is still connected). Documented in §3 restart policy.
    log_info!(Subsystem::Controller, None,
        "released {} controller claims from exited app={}", released.len(), app);
}
```

**Restart policy**: for `restart = always` apps, after re-spawn the process manager calls
`on_controller_connect` for each slot that was claimed by the crashed app and is still
physically connected. This re-assigns the slot to the new incarnation without requiring the
app to re-issue `controller_claim`.

---

## 10. Capability Gate

```toml
# apps/my-game/vyoma.toml
[capabilities]
controller            = true   # receive VYOMA_INPUT:controller:*
controller_max_players = 4     # allow claiming up to 4 slots
rumble                = true   # may emit VYOMA_RUMBLE:
```

Validation: `controller_max_players > 1` requires `controller = true`; `rumble = true`
requires `controller = true`. Rejected at manifest parse with `Error::InvalidCapabilities`.

---

## 11. Wire Protocol

**Supervisor → App**:
```
VYOMA_INPUT:controller:connect:<player>:<vendor:04x>:<product:04x>:<name>
VYOMA_INPUT:controller:disconnect:<player>
VYOMA_INPUT:controller:state:<player>:<seq>:<epoch>:<buttons:08x>:<lx>:<ly>:<rx>:<ry>:<lt>:<rt>
VYOMA_INPUT:controller:button:<player>:<seq>:<epoch>:<button_name>:<down|up>
```

**App → Supervisor**:
```
VYOMA_RUMBLE:<player>:<low:0-255>:<high:0-255>:<duration_ms:0-5000>
@supervisor: controller_claim <player>
@supervisor: controller_release <player>
@supervisor: virtual_cursor on|off
```

`controller_claim` requires `shell = true` + `controller_max_players >= player+1`.
`controller_release` no-ops if caller doesn't hold the slot.

---

## 12. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Disconnect event lost when routing changed or lock active | `LAST_ROUTED_APP[4]: [ArcSwap<Option<String>>; 4]` updated on every successful delivery; disconnect delivered directly to `last_routed_app[slot]`, bypassing lock gate and routing recomputation |
| B2: Axis normalization before SYN_REPORT produces split-axis emissions | Scratch `ControllerState` accumulates raw events; normalization + hysteresis + emission runs only at `SYN_REPORT` boundary |
| B3: Held buttons "stick" across INPUT_LOCK_LEVEL rise | `flush_controller_state_on_lock_rise()` emits all-zero synthetic state with bumped epoch+seq to all `LAST_ROUTED_APP` targets; called from `on_input_lock_rise()` after epoch increment |
| B4: Rumble writer task blocks hub epoll fd (shared fd) | Controller opened twice: `read_fd` (hub epoll, input reads) and `write_fd` (rumble writer task, ioctl+write exclusively); no fd sharing; bounded mpsc capacity=4 coalesces rapid rumble |
| B5: Controller claims leaked when claiming app exits | `release_controller_claims(app)` called from `on_app_exit` hook in `process.rs`; for `restart=always`, process manager re-emits `on_controller_connect` for previously-claimed connected slots after re-spawn |

---

## 13. File Layout

```
supervisor/src/
  controller/
    mod.rs              (~120 lines: ControllerHub, LAST_ROUTED_APP, init)
    discovery.rs        (~200 lines: evdev open×2 (B4), inotify, slot persistence)
    state.rs            (~180 lines: ControllerState, SYN_REPORT batching (B2), normalization)
    routing.rs          (~160 lines: route_controller_event, CONTROLLER_CLAIMS,
                                     flush_controller_state_on_lock_rise (B3),
                                     release_controller_claims (B5))
    rumble.rs           (~150 lines: dual-fd pattern (B4), writer task, FF_RUMBLE ioctl)
    virtual_cursor.rs   (~110 lines: left-stick → MouseEventQueue)
```
