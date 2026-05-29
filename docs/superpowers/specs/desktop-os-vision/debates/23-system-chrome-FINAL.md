# FINAL Spec: Menu Bar & System Chrome (Round 23)

**Subsystem**: Menu Bar & System Chrome  
**macOS Analogue**: `NSMenuBar`, SystemUIServer  
**Depends on**: R11 (Surface, compositor), R20 (DisplayConfig), R21 (space-0 always-on-top), R22 (lifecycle)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

System chrome is implemented as a privileged WASM app (`chrome`) running in space=0
(always on top per R21). It draws using standard `VYOMA_DRAW:` commands and communicates
via the `VYOMA_CHROME:` line protocol. The supervisor clips all non-chrome app draws to
`y >= CHROME_HEIGHT_PX`, providing defense-in-depth.

### Non-Goals (v1)

- Clickable menu items (deferred to R32 mouse input)
- Real battery / wifi status (deferred to R7 power, R55 wifi)
- Per-space menu bars (single bar only)
- Notification banners (reserved Surface region; R73)

---

## 2. Chrome App Architecture

### 2.1 Chrome as space=0 WASM App

```toml
# apps/chrome/vyoma.toml
[app]
name    = "chrome"
version = "1.0.0"
wasm    = "chrome.wasm"

[capabilities]
display = true
stdio   = true
shell   = true    # @supervisor: IPC commands
```

`win_space = 0` is set by the supervisor unconditionally for the `chrome` app — it is
not user-configurable. Space-0 ensures chrome is always composited last (on top).

**Invariant**: The WM space-switch handler includes an assertion:
```rust
assert!(app.win_space != 0 || app.name == "chrome", "only chrome may use space=0");
```
This ensures no other app can be placed in space=0 and no space-switch event can
transition chrome to Suspended.

### 2.2 Chrome Startup Fallback (B1 fix)

The supervisor owns a `chrome_surface_ready: bool` flag (false at boot). Before the
`apps_sorted` blit loop in `flush_pass`, the compositor checks this flag:

```rust
// supervisor/src/compositor.rs — flush_pass(), before apps_sorted blit loop

if !state.chrome_surface_ready {
    // Fallback: fill top CHROME_HEIGHT_PX rows with menu bar background
    let chrome_h_px = (CHROME_HEIGHT_PTS as u32).saturating_mul(config.scale_factor as u32);
    fb.fill_rows(0, chrome_h_px, MENU_BAR_BG_COLOR);
}
```

`MENU_BAR_BG_COLOR = 0x1E1E2EFF`. `chrome_surface_ready` is set to `true` on chrome's
first `VYOMA_DRAW:flush` and reset to `false` on chrome exit (lifecycle Terminated).

This eliminates the visual glitch where the top 24px shows uninitialized framebuffer
content during the boot window.

---

## 3. Menu Bar Layout

- Height: 24 logical points
- y range: 0–23 (logical points) = 0–47 physical pixels at 2x
- Left section (x=0..logical_w/2): focused app name + menu items
- Right section (x=logical_w/2..logical_w): clock, status icons
- Background: `MENU_BAR_BG_COLOR = 0x1E1E2EFF`

The chrome app redraws its full Surface on focus change, tick, or consent state change.

---

## 4. VYOMA_CHROME: Protocol

### 4.1 Supervisor → Chrome (stdin push)

```
VYOMA_CHROME:focus_changed:<app_name>
VYOMA_CHROME:tick:<seconds_since_epoch>
VYOMA_CHROME:menu_set:<app_name>,<item1>|<item2>|...
VYOMA_CHROME:menu_clear:<app_name>
VYOMA_CHROME:consent_request:<type>,<app_name>,<detail>
VYOMA_CHROME:consent_revoke:<app_name>        ← from R18/R19 consent revoke
VYOMA_CHROME:app_launched:<app_name>
VYOMA_CHROME:app_exited:<app_name>
```

### 4.2 Chrome → Supervisor (stdout)

```
@supervisor: consent_capture_grant:<app_name>
@supervisor: consent_capture_deny:<app_name>
@supervisor: consent_vdisp_grant:<app_name>
@supervisor: consent_vdisp_deny:<app_name>
@supervisor: input_lock_chrome
@supervisor: input_unlock_chrome
```

### 4.3 Any App → Chrome via Supervisor

Apps write to stdout; supervisor routes `VYOMA_CHROME:` prefixed lines to chrome's stdin:

```
VYOMA_CHROME:menu_set:<item1>|<item2>|...    ← supervisor fills app_name field
VYOMA_CHROME:menu_clear
```

### 4.4 Chrome Queue with Drop-Oldest Policy (B2 fix)

```rust
// supervisor/src/chrome_router.rs

const CHROME_QUEUE_CAPACITY: usize = 64;

pub struct ChromeRouter {
    queue: VecDeque<String>,
    chrome_running: bool,
}

impl ChromeRouter {
    pub fn route(&mut self, msg: String, chrome_tx: &Sender<String>) {
        if self.chrome_running {
            let _ = chrome_tx.send(msg);
            return;
        }
        // Queue for later delivery; drop-oldest on overflow
        if self.queue.len() >= CHROME_QUEUE_CAPACITY {
            self.queue.pop_front();  // drop oldest — last-writer-wins semantics
        }
        self.queue.push_back(msg);
    }

    pub fn on_chrome_running(&mut self, chrome_tx: &Sender<String>) {
        self.chrome_running = true;
        while let Some(msg) = self.queue.pop_front() {
            let _ = chrome_tx.send(msg);
        }
    }

    pub fn on_chrome_not_running(&mut self) {
        self.chrome_running = false;
        // Do NOT clear the queue — chrome may restart soon
    }

    pub fn on_chrome_terminated_no_restart(&mut self) {
        // Chrome is dead and will not restart — discard all queued messages
        self.queue.clear();
        self.chrome_running = false;
    }
}
```

**Drop-oldest overflow policy**: the most recent `menu_set` supersedes all older ones;
the most recent `tick` supersedes older ticks. Dropping oldest is correct.

**Terminated + no restart**: if chrome's restart_total ≥ MAX_RESTARTS_PER_HOUR (R22),
the router discards all incoming `VYOMA_CHROME:` lines immediately (no queuing).

**Chrome space=0 invariant**: chrome cannot be Suspended via space-switch because
space=0 apps are never affected by space-switch events (R21 compositor only switches
space-N apps). This means the only path to chrome-not-running is crash (Terminated).

---

## 5. Clock Tick Mechanism (B5 fix)

### 5.1 Tick State in SupervisorState

```rust
pub struct SupervisorState {
    // ...
    /// Initialized to SystemTime::now() at startup — NOT UNIX_EPOCH (B5 fix).
    pub last_chrome_tick: SystemTime,
    pub chrome_surface_ready: bool,
    pub chrome_input_locked: Arc<AtomicBool>,
}
```

### 5.2 Tick Check in Main Event Loop

```rust
// supervisor/src/main.rs — inside supervisor_tick()

fn maybe_send_chrome_tick(state: &mut SupervisorState, now: SystemTime) {
    let elapsed = now.duration_since(state.last_chrome_tick).unwrap_or_default();
    if elapsed >= Duration::from_secs(1) {
        let ts = now.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        state.chrome_router.route(
            format!("VYOMA_CHROME:tick:{}\n", ts),
            &state.chrome_stdin_tx,
        );
        // Set to now — absorb any overage to prevent burst delivery (B5 fix)
        // last_chrome_tick = now, NOT last_chrome_tick + 1s
        state.last_chrome_tick = now;
    }
}
```

**Single emission per tick**: `last_chrome_tick = now` absorbs any elapsed overage. If
the event loop is blocked for 3 seconds, exactly one tick is emitted on unblock. No
multi-tick burst.

**Initialization**: `last_chrome_tick = SystemTime::now()` at supervisor startup ensures
the first tick is emitted ~1 second after boot, not immediately.

---

## 6. Y-Clip in flush_pass (B3 fix)

### 6.1 Clip Site

The y-clip belongs in `flush_pass` (not inside `blit_surface`), because only `flush_pass`
has access to `scale_factor` and the chrome app name.

```rust
// supervisor/src/compositor.rs — flush_pass()

const CHROME_HEIGHT_PTS: u32 = 24;

fn flush_pass(fb: &mut FrameBuffer, apps_sorted: &[AppSnapshot], config: &DisplayConfig) {
    let chrome_h_px = CHROME_HEIGHT_PTS.saturating_mul(config.scale_factor as u32);

    for snap in apps_sorted {
        let win_y_px = snap.win_y_pts.saturating_mul(config.scale_factor as u32);

        let effective_dest_y = if snap.app_name == "chrome" {
            win_y_px
        } else {
            win_y_px.max(chrome_h_px)
        };

        // Skip source rows that correspond to y < effective_dest_y
        // Prevents blitting app's topmost rows into the chrome strip (B3 fix)
        let src_y_offset = effective_dest_y.saturating_sub(win_y_px);

        if src_y_offset >= snap.surface_height_px {
            continue;  // entire Surface is clipped
        }

        blit_surface(
            fb,
            &snap.surface,
            snap.win_x_pts.saturating_mul(config.scale_factor as u32),
            effective_dest_y,
            snap.surface_width_px,
            snap.surface_height_px.saturating_sub(src_y_offset),
            src_y_offset,  // first source row to blit
        );
    }
}
```

`blit_surface` signature updated to accept `src_y_offset: u32`:
```rust
fn blit_surface(fb: &mut FrameBuffer, surface: &[u8], dest_x: u32, dest_y: u32,
                width: u32, height: u32, src_y_offset: u32)
```

Without `src_y_offset`, a window at logical y=0 clipped to physical y=48 (2x) would blit
its topmost rows (title bar, etc.) at y=48 — visually wrong. With `src_y_offset=48`, those
rows are skipped and rows at physical y=48 in the app's Surface are blitted correctly.

---

## 7. Consent Dialog & Input Lock (B4 fix)

### 7.1 Input Lock Model

```rust
// supervisor/src/main.rs — SupervisorState

pub chrome_input_locked: Arc<AtomicBool>,  // independent of apps_map
```

TTY input thread uses atomic load — no RwLock contention:

```rust
// supervisor/src/input.rs — TTY input thread

fn route_keyboard(state: &InputState, key: KeyEvent) {
    let target = if state.chrome_input_locked.load(Ordering::Acquire) {
        "chrome".to_string()
    } else {
        state.focused_app.load().clone()
    };
    send_key_to_app(&target, key);
}
```

### 7.2 Auto-Unlock on Chrome State Change

Whenever chrome's lifecycle transitions away from `Running` (to Terminating, Terminated,
Suspended), the supervisor auto-releases the input lock:

```rust
// supervisor/src/lifecycle/events.rs — on_chrome_lifecycle_change()

pub fn on_chrome_lifecycle_change(new_state: LifecycleState, lock: &Arc<AtomicBool>) {
    if new_state != LifecycleState::Running {
        lock.store(false, Ordering::Release);
    }
}
```

This ensures keyboard input never stays locked to a dead or suspended chrome app.
The consent modal is purely visual when input is locked — no app-side mechanism prevents
other apps from receiving input if the lock is released (e.g. if chrome crashes mid-dialog).
This is documented: "consent modal locking is best-effort; chrome crash releases the lock."

### 7.3 Consent Flow

```
App calls capture WIT → supervisor sends VYOMA_CHROME:consent_request:capture,<app>,<detail>
Chrome draws modal overlay
Chrome: @supervisor: input_lock_chrome
User selects Yes/No (future R32 mouse input or keyboard shortcut)
Chrome: @supervisor: consent_capture_grant:<app> OR consent_capture_deny:<app>
Chrome: @supervisor: input_unlock_chrome
Supervisor updates CapturePermission and responds to waiting WIT call
```

---

## 8. App Menu Protocol

### 8.1 App Sets Menu

```
VYOMA_CHROME:menu_set:<item1>|<item2>|<item3>
```

Supervisor routes this from the app's stdout to chrome's stdin, prepending app_name:
```
VYOMA_CHROME:menu_set:<app_name>,<item1>|<item2>|<item3>
```

### 8.2 User Selects Menu Item (future R32)

Chrome sends to originating app's stdin:
```
VYOMA_CHROME_SELECTED:<item>
```

---

## 9. WIT Interface `vyoma:chrome@1.0.0`

For the chrome app only (not general apps):

```wit
package vyoma:chrome@1.0.0;

interface chrome {
    /// Set menu items for the focused app.
    set-menu-items: func(items: list<string>) -> result<_, string>;

    /// Get the currently focused app's name.
    get-focused-app: func() -> result<string, string>;

    /// Send consent response (grant or deny).
    send-consent-response: func(app-name: string, consent-type: string, granted: bool)
        -> result<_, string>;

    /// Request keyboard input lock (all keys → chrome).
    lock-input: func() -> result<_, string>;

    /// Release keyboard input lock.
    unlock-input: func() -> result<_, string>;
}
```

Capability gate: only the app named `"chrome"` in boot.toml can access this interface.
Any other app calling these functions receives `Err("chrome-only interface")`.

---

## 10. `VYOMA_DRAW:resize_surface` Command

Chrome needs to extend its Surface from 24pts to 72pts during notification banners (R73).
New draw command added to `draw_cmd.rs`:

```
VYOMA_DRAW:resize_surface:<w>,<h>
```

- `w` and `h` are in logical points
- Only valid from an app with `display = true`
- Supervisor queues as `PendingResize` per R21 pattern (drained between compositor frames)
- Chrome uses this to expand from 24pts to 72pts for banners

---

## 11. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Menu bar | yes | no | no | no |
| Status bar | yes | yes (bottom, 24pts) | no | no |
| Consent dialogs | yes | yes | no | no |
| `chrome` app | yes | yes (simplified) | no | no |
| System tray | yes | no | no | no |
| Input lock | yes | yes | no | no |

Mobile chrome: status bar at bottom (y = logical_h - 24). No menu items, only clock and
battery. Clip is `y > logical_h - 24` for non-chrome apps.

---

## 12. File Layout

```
apps/chrome/src/main.rs        (Chrome WASM app — draws menu bar, handles stdin)
apps/chrome/vyoma.toml         (win_space=0 enforced by supervisor)

supervisor/src/chrome_router.rs (ChromeRouter with drop-oldest queue)
supervisor/src/compositor.rs   (flush_pass: chrome_surface_ready fallback fill, y-clip)
supervisor/src/input.rs        (TTY thread: chrome_input_locked atomic check)
supervisor/src/lifecycle/events.rs (on_chrome_lifecycle_change: auto-unlock)
supervisor/src/draw_cmd.rs     (resize_surface command added)
supervisor/src/wit_handlers.rs (chrome WIT closures)
```

---

## 13. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Chrome startup race | `chrome_surface_ready: bool` flag; compositor fills top 24px with `MENU_BAR_BG_COLOR` until chrome's first flush |
| B2: Queue on Suspended/Terminated | Drop-oldest ring buffer; `on_chrome_terminated_no_restart` clears queue; chrome space=0 invariant prevents Suspended via space-switch |
| B3: Y-clip `src_y_offset` missing | Clip in `flush_pass` (not `blit_surface`); `effective_dest_y` + `src_y_offset = effective - win_y_px` passed to `blit_surface`; prevents topmost rows blitting at y=chrome_h |
| B4: Input lock not threadsafe | `Arc<AtomicBool>` for `chrome_input_locked` (no RwLock contention); auto-store-false on any chrome lifecycle ≠ Running |
| B5: Clock burst on unblock | `last_chrome_tick = now` (not `+= 1s`) absorbs overage; one tick per event-loop iteration max; initialized to `SystemTime::now()` at startup |
