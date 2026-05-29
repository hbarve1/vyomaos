# FINAL Spec: Mission Control & Exposé (Round 25)

**Subsystem**: Mission Control & Exposé  
**macOS Analogue**: Mission Control (window overview), Exposé (app window scatter)  
**Depends on**: R21 (WM spaces, PerSpaceZ), R22 (lifecycle, surface_quiesced), R23 (chrome, input_lock), R24 (dock, DockSwitcherState, Model A)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

Mission Control (MC) is a third privileged WASM app (`mission-control`) in space=0
running an overview compositor mode. When active, it shows thumbnails of all open
windows and a space strip across the top. Exposé mode shows only windows of the
currently focused app.

---

## 2. Architecture

### 2.1 Mission Control as space=0 WASM App

```toml
# apps/mission-control/vyoma.toml
[app]
name = "mission-control"
version = "1.0.0"
wasm = "mission-control.wasm"

[capabilities]
display = true
stdio   = true
shell   = true   # @supervisor: focus <app>, space <n>, mc_exit
# NO filesystem = true (B1 fix — thumbnails delivered via stdin)
```

`win_space = 0`, `win_y = 0`, `win_w = logical_w`, `win_h = logical_h`.
Z-index: 65533. Space-0 z-order: dock(65534) and chrome(65535) composite above MC.
MC fills the screen but dock and chrome remain visible on top.

**Space=0 invariant update**: The assertion guarding space=0 is updated:
```rust
assert!(
    app.win_space != 0
        || matches!(app.name.as_str(), "chrome" | "dock" | "mission-control"),
    "only chrome/dock/mission-control may use space=0"
);
```

### 2.2 Compositor Model A — Three space-0 Apps

Pass 2 in `flush_pass` now has three space-0 apps sorted by z ascending:
```
mission-control (65533) → dock (65534) → chrome (65535)
```
No changes to the two-pass structure from R24.

### 2.3 MC Startup Fallback

`mc_surface_ready: bool` flag (false at boot, set true on MC's first flush).
When false: **no fallback fill** — apps below remain visible since MC background is
intentionally transparent-on-inactive. MC is launched only when triggered; until then
`mc_surface_ready` stays false and the MC surface is zero-sized (0×0).

---

## 3. Trigger Mechanism

### 3.1 F3 / Ctrl-Up Interception (TTY Input Thread)

```rust
// supervisor/src/main.rs — inside key event processing

fn process_key_event(ev: KeyEvent, mc_state: &mut McTriggerState,
                     mc_router: &McRouter, mc_tx: &Sender<String>) {
    if ev.key == Key::F3
        || (ev.key == Key::Up && ev.ctrl_held)
    {
        mc_router.route("VYOMA_MC:open\n".into(), mc_tx);
        INPUT_LOCK_LEVEL.store(LockLevel::MissionControl as u8, Ordering::Release);
        return;  // do NOT forward to focused app
    }
    // Normal routing via INPUT_LOCK_LEVEL priority (B2)
    route_by_lock_level(ev);
}
```

### 3.2 Exposé Shortcut

Ctrl-Down or Alt-F3 sends `VYOMA_MC:expose_open` instead of `VYOMA_MC:open`.
MC app renders only windows of `focused_app` with no space strip.

---

## 4. Input Lock Priority (B2 Fix)

### 4.1 Single `AtomicU8` Priority Level

Replace two independent `AtomicBool` flags with one priority-encoded value:

```rust
// supervisor/src/input.rs

pub enum LockLevel {
    None          = 0,
    MissionControl = 1,
    ChromeConsent  = 2,   // consent always wins over MC
}

pub static INPUT_LOCK_LEVEL: AtomicU8 = AtomicU8::new(0);

/// Saved level before raising to ChromeConsent
pub static PREV_LOCK_LEVEL: AtomicU8 = AtomicU8::new(0);
```

TTY routing:
```rust
fn route_keyboard(ev: KeyEvent, focused_app: &str) {
    match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        2 => send_key_to_app("chrome", ev),
        1 => send_key_to_app("mission-control", ev),
        _ => send_key_to_app(focused_app, ev),
    }
}
```

**Raising to ChromeConsent** (consent dialog open):
```rust
PREV_LOCK_LEVEL.store(INPUT_LOCK_LEVEL.load(Ordering::Acquire), Ordering::Release);
INPUT_LOCK_LEVEL.store(LockLevel::ChromeConsent as u8, Ordering::Release);
```

**Restoring after consent closes**:
```rust
INPUT_LOCK_LEVEL.store(PREV_LOCK_LEVEL.load(Ordering::Acquire), Ordering::Release);
```

**Auto-clear on lifecycle change away from Running** (same pattern as R23/R24):
```rust
pub fn on_mc_lifecycle_change(new_state: LifecycleState) {
    if new_state != LifecycleState::Running {
        INPUT_LOCK_LEVEL.compare_exchange(
            LockLevel::MissionControl as u8,
            LockLevel::None as u8,
            Ordering::AcqRel,
            Ordering::Relaxed,
        ).ok();
    }
}

pub fn on_chrome_lifecycle_change(new_state: LifecycleState) {
    if new_state != LifecycleState::Running {
        INPUT_LOCK_LEVEL.compare_exchange(
            LockLevel::ChromeConsent as u8,
            PREV_LOCK_LEVEL.load(Ordering::Acquire),
            Ordering::AcqRel,
            Ordering::Relaxed,
        ).ok();
    }
}
```

**Priority rule**: `ChromeConsent(2)` always wins over `MissionControl(1)`. If consent
dialog opens while MC is active, keys go to chrome. On consent close, keys return to MC
(restored via `PREV_LOCK_LEVEL`). This eliminates the soft-deadlock described in B2.

---

## 5. Thumbnail Capture — Dedicated Capture Thread (B1 + B3 Fix)

### 5.1 No Filesystem Capability (B1 Fix)

Thumbnails are delivered entirely via **stdin push** — no files, no `/data` mount.
The MC app needs only `stdio = true`. The `VYOMA_MC:` protocol carries pixel data
inline:

```
VYOMA_MC:thumb_begin:<app_name>,<w_px>,<h_px>
VYOMA_MC:thumb_data:<base64_chunk>   (one line per 64-byte chunk, repeated)
VYOMA_MC:thumb_end
```

MC app accumulates chunks into an in-memory decoded buffer, keyed by `app_name`.
Rendering uses `VYOMA_DRAW:draw_thumb:<thumb_id>,<x>,<y>,<w>,<h>` — a new draw
command that blits a named in-memory buffer at the given destination rect.

### 5.2 Dedicated Capture Thread (B3 Fix)

```rust
// supervisor/src/mc_capture.rs

pub struct CaptureRequest {
    pub all_spaces: bool,
    pub reply_tx: Sender<Vec<ThumbData>>,
}

pub struct ThumbData {
    pub app_name: String,
    pub width_px: u32,
    pub height_px: u32,
    pub pixels: Vec<u8>,   // BGRA, width_px × height_px × 4 bytes
}

/// Spawned once at supervisor startup
pub fn capture_thread_main(
    req_rx: Receiver<CaptureRequest>,
    apps: Arc<RwLock<AppsMap>>,
    vsync_lock: Arc<RwLock<()>>,
    config: Arc<DisplayConfig>,
) {
    for req in req_rx {
        // One atomic snapshot: hold read-lock for entire copy pass (B3 fix)
        let snapshot: Vec<ThumbData> = {
            let _r = vsync_lock.read();
            snapshot_all_surfaces_locked(
                &apps.read(),
                req.all_spaces,
                &config,
            )
        };
        // vsync_lock released here — all I/O outside lock
        let _ = req.reply_tx.send(snapshot);
    }
}

fn snapshot_all_surfaces_locked(apps: &AppsMap, all_spaces: bool,
                                 config: &DisplayConfig) -> Vec<ThumbData>
{
    apps.values()
        .filter(|a| all_spaces || a.win_space == config.active_space)
        .filter_map(|a| {
            // Use last_flushed_snapshot (B4 fix) for quiesced surfaces
            let pixels = a.last_flushed_snapshot.as_ref()?.as_ref().clone();
            Some(ThumbData {
                app_name: a.name.clone(),
                width_px: a.surface_width_px,
                height_px: a.surface_height_px,
                pixels,
            })
        })
        .collect()
}
```

**Key invariants**:
- `vsync_lock.read()` held for the **entire** multi-app copy — not released and
  re-acquired per app. Eliminates writer starvation window (B3 fix).
- All I/O and base64 encoding happen after `vsync_lock` is released.
- If a second `mc_capture_request` arrives before the first completes, it is dropped
  (MC sees no second `windows_ready` — acceptable; MC can retry on next open).

### 5.3 Thumbnail Delivery Flow

```
1. MC app sends @supervisor: mc_capture_request (or mc_capture_request_all_spaces)
2. Supervisor main loop posts CaptureRequest to capture_thread via tx channel
3. Capture thread takes single vsync_lock.read() snapshot, releases lock, encodes thumbnails
4. Encoded ThumbData sent back to main loop via reply_tx
5. Main loop base64-encodes each thumb, streams:
     VYOMA_MC:thumb_begin:<app_name>,<w>,<h>
     VYOMA_MC:thumb_data:<base64_line>  (repeat)
     VYOMA_MC:thumb_end
   for each captured app
6. After all thumbs: VYOMA_MC:windows_ready:<count>
7. MC app draws thumbnails using in-memory decoded buffers
```

### 5.4 Thumbnail Scale

Thumbnails are downscaled in the capture thread before base64 encoding. Target thumbnail
size: `logical_w/6 × logical_h/6` logical points (≈160×100pt at 960×600 base). Simple
2×2 pixel averaging downscale — no dependency on external libraries.

---

## 6. Surface Quiescence for Suspended Apps (B4 Fix)

### 6.1 `surface_quiesced` Flag

```rust
// supervisor/src/display.rs — AppState

pub struct AppState {
    // ... existing fields ...
    pub surface_quiesced: bool,
    pub last_flushed_snapshot: Option<Arc<Vec<u8>>>,
}
```

### 6.2 Quiescence on Suspend

```rust
// supervisor/src/lifecycle/events.rs

pub fn on_app_suspended(app: &mut AppState) {
    app.lifecycle = LifecycleState::Suspended;
    app.surface_quiesced = true;
    // surface is now stable for capture; last_flushed_snapshot is safe to read
}

pub fn on_app_resumed(app: &mut AppState) {
    app.surface_quiesced = false;
    app.lifecycle = LifecycleState::Running;
}
```

### 6.3 Draw Command Gate

```rust
// supervisor/src/draw_cmd.rs — handle_draw_cmd()

pub fn handle_draw_cmd(app: &mut AppState, cmd: DrawCmd) {
    if app.surface_quiesced {
        return;  // discard all draw commands for quiesced apps
    }
    // ... normal draw handling ...
}
```

### 6.4 `last_flushed_snapshot` Update

On each successful `VYOMA_DRAW:flush` commit:
```rust
// supervisor/src/compositor.rs — in flush handler

app.last_flushed_snapshot = Some(Arc::new(app.surface.clone()));
```

The MC capture thread reads `last_flushed_snapshot` (not live `surface`). This is safe
without `vsync_lock` for `Suspended` apps (quiesced surface = no mutations). For `Running`
apps, the capture thread holds `vsync_lock.read()` which blocks the flush handler's
`vsync_lock.write()` — consistent snapshot guaranteed.

---

## 7. Window Layout — Grid Algorithm

```rust
// apps/mission-control/src/layout.rs

pub fn compute_grid(
    n: usize, area_w: u32, area_h: u32
) -> Vec<Rect> {
    let cols = (n as f32).sqrt().ceil() as u32;
    let rows = (n as u32 + cols - 1) / cols;
    let cell_w = area_w / cols;
    let cell_h = area_h / rows;
    let thumb_w = (cell_w - THUMB_PADDING * 2).min(cell_h * 16 / 10);
    let thumb_h = thumb_w * 10 / 16;
    (0..n).map(|i| {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let x = col * cell_w + (cell_w - thumb_w) / 2;
        let y = row * cell_h + (cell_h - thumb_h) / 2;
        Rect { x, y, w: thumb_w, h: thumb_h }
    }).collect()
}
```

Layout area: `y = SPACE_STRIP_H + MARGIN` to `y = logical_h - DOCK_H - MARGIN`.
`SPACE_STRIP_H = 80`, `DOCK_H = 48`, `MARGIN = 16`, `THUMB_PADDING = 8`.
Window label (app name, truncated to 16 chars) rendered below each thumbnail.

---

## 8. Space Strip

### 8.1 Layout

Top 80pt of MC overlay (`y = 0..80`). Shows one thumbnail card per space plus an
"Add Space" (`+`) button on the right. Each space card: `logical_w / (n_spaces + 1)`
wide (min 80pt, max 200pt), 64pt tall.

### 8.2 Space Commands

MC app sends to supervisor:
```
@supervisor: space_add          ← create new space (right of existing)
@supervisor: space_del <n>      ← delete space n (min 1 space remaining)
@supervisor: space <n>          ← switch to space n
```

These reuse R21 IPC commands verbatim.

### 8.3 Space Thumbnails

Space thumbnails are generated during `mc_capture_request_all_spaces`. Each space
shows the wallpaper color + stacked window thumbnails at reduced opacity.
The active space card has a bright border.

---

## 9. Exposé Mode

Triggered by `VYOMA_MC:expose_open`. Identical to MC open but:
- No space strip (full `logical_h` for window layout area)
- Only windows where `a.win_space == active_space && a.focused_by_app == current_focused`
  are shown
- Keyboard navigation, exit, and thumbnail delivery identical to full MC mode

---

## 10. Keyboard Navigation

```
Arrow keys: move selection highlight among thumbnails
Enter:       select (triggers exit + focus + space switch)
Escape:      cancel (exit MC, restore previous focus, no space change)
```

MC app tracks `selected_index: usize` and redraws highlight on each keypress.
Mouse click navigation deferred to R32.

---

## 11. Deferred Focus After Space Switch (B5 Fix)

### 11.1 `pending_focus: Option<String>` in SupervisorState

```rust
// supervisor/src/main.rs — SupervisorState

pub struct SupervisorState {
    // ...
    pub pending_focus: Option<String>,
}
```

### 11.2 MC Exit Sequence

MC app sends (in this order):
```
1. @supervisor: space <N>          ← set PENDING_SPACE_SWITCH = N (if different space)
2. @supervisor: focus <selected>   ← stored in pending_focus if space switch pending
3. @supervisor: mc_exit            ← MC collapses surface, normal compositing resumes
```

### 11.3 Focus Deferral Logic

```rust
// supervisor/src/ipc_handlers.rs — handle_focus_command()

pub fn handle_focus_command(app_name: &str, state: &mut SupervisorState) {
    let pending_switch = PENDING_SPACE_SWITCH.load(Ordering::Acquire);
    if pending_switch != 0 {
        // Defer until compositor applies space switch (B5 fix)
        state.pending_focus = Some(app_name.to_string());
        return;
    }
    // No space switch pending — apply immediately
    wm::focus::handle_focus(app_name, &mut state.apps, state.spaces.active);
    wm::zorder::compact_z_order(&mut state.apps, state.spaces.active);
}
```

### 11.4 Compositor Applies Deferred Focus

```rust
// supervisor/src/compositor.rs — vsync_tick(), after space switch block

// Apply PENDING_SPACE_SWITCH first
if pending_switch != 0 {
    PENDING_SPACE_SWITCH.store(0, Ordering::Release);
    state.spaces.active = pending_switch;
    fb.fill(BACKGROUND_COLOR);
    mark_all_dirty(&mut state.apps);
    wm::zorder::compact_z_order(&mut state.apps, state.spaces.active);
}

// Then apply deferred focus (always in correct space context now)
if let Some(focus_name) = state.pending_focus.take() {
    wm::focus::handle_focus(&focus_name, &mut state.apps, state.spaces.active);
    wm::zorder::compact_z_order(&mut state.apps, state.spaces.active);
}
```

**Ordering guarantee**: focus is always applied AFTER space switch is committed.
`compact_z_order` runs on the correct `spaces.active`. The selected app is brought
to front in its own space.

---

## 12. MC Exit and Normal Compositing Restore

### 12.1 `@supervisor: mc_exit`

```rust
// supervisor/src/ipc_handlers.rs

pub fn handle_mc_exit(state: &mut SupervisorState, mc_tx: &Sender<String>) {
    INPUT_LOCK_LEVEL.compare_exchange(
        LockLevel::MissionControl as u8,
        LockLevel::None as u8,
        Ordering::AcqRel,
        Ordering::Relaxed,
    ).ok();
    state.mc_router.route("VYOMA_MC:close\n".into(), mc_tx);
}
```

MC app:
```
On @supervisor: mc_exit received:
  1. Run exit animation (~200ms, global_alpha fade 1.0→0.0)
  2. VYOMA_DRAW:resize_surface:0,0    ← collapse surface (invisible)
  3. VYOMA_DRAW:flush
  4. Set mc_surface_ready-equivalent to false
```

Normal compositing resumes immediately — MC's collapsed surface is zero-sized,
so Pass 2 blits only dock and chrome.

### 12.2 MC Cancel (Escape)

Escape key: MC sends `@supervisor: focus <previously_focused_app>` (no space change),
then follows normal mc_exit sequence. `pending_focus` is set if a space change was
pending (rare on cancel — no space change sent, so `PENDING_SPACE_SWITCH == 0`, focus
applied immediately).

---

## 13. VYOMA_MC: Protocol

### 13.1 Supervisor → MC (stdin push)

```
VYOMA_MC:open                                       ← trigger MC (full mode)
VYOMA_MC:expose_open                                ← trigger Exposé mode
VYOMA_MC:close                                      ← supervisor requests close (e.g. on dock swipe)
VYOMA_MC:focus_changed:<app_name>                   ← passive update
VYOMA_MC:space_changed:<n>                          ← passive update
VYOMA_MC:app_launched:<app_name>,<display_name>
VYOMA_MC:app_exited:<app_name>
VYOMA_MC:thumb_begin:<app_name>,<w_px>,<h_px>
VYOMA_MC:thumb_data:<base64_64_chars>               ← repeat until end
VYOMA_MC:thumb_end
VYOMA_MC:windows_ready:<count>                      ← all thumbs delivered
```

### 13.2 MC → Supervisor (stdout)

```
@supervisor: mc_capture_request                     ← active space only
@supervisor: mc_capture_request_all_spaces          ← all spaces (space strip)
@supervisor: space <n>                              ← switch space
@supervisor: space_add
@supervisor: space_del <n>
@supervisor: focus <app_name>
@supervisor: mc_exit
```

### 13.3 `McRouter`

Identical pattern to `ChromeRouter` and `DockRouter` — 64-slot drop-oldest VecDeque.
`on_mc_not_running()` preserves queue; `on_mc_terminated_no_restart()` clears queue.

---

## 14. `VYOMA_DRAW:draw_thumb` Command

New draw command added to `draw_cmd.rs`:

```
VYOMA_DRAW:draw_thumb:<thumb_id>,<x>,<y>,<w>,<h>
```

- `thumb_id`: integer key assigned by MC app after receiving and decoding a `thumb_begin/data/end` sequence
- `x`, `y`, `w`, `h`: destination rect in logical points
- Supervisor blits the named in-memory thumbnail buffer at the given rect, scaling as needed
- Only valid from the `mission-control` app (capability-gated)

The `thumb_id` → pixel buffer mapping lives in the MC app's in-memory state (not on disk).
The supervisor does NOT store thumbnails; MC app is responsible for accumulating and keying them.

---

## 15. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Mission Control | yes | partial (app-switcher) | no | no |
| Exposé | yes | no | no | no |
| Space strip | yes | no | no | no |
| `mission-control` WASM app | yes | yes (simplified) | no | no |
| Ctrl-Tab dock switcher | yes (R24) | no | no | no |

Mobile variant: no space strip, no grid thumbnails. Shows a scrollable list of app names
with `last_flushed_snapshot` mini-previews. Triggered by home-button swipe up (R32 touch).

---

## 16. File Layout

```
apps/mission-control/src/
├── main.rs        (event loop: stdin parsing, capture requests, draw calls)
├── state.rs       (McState: thumbs, selected_index, mode, space_list)
├── layout.rs      (compute_grid, compute_space_strip)
└── draw.rs        (render_mc_overlay, render_space_strip, render_expose)

supervisor/src/
├── mc_router.rs   (McRouter — mirror of chrome_router.rs, dock_router.rs)
├── mc_capture.rs  (CaptureRequest channel, capture_thread_main, snapshot_all_surfaces_locked)
├── compositor.rs  (flush_pass: mc_surface_ready, draw_thumb handler, pending_focus apply)
├── draw_cmd.rs    (draw_thumb command parsing; surface_quiesced gate)
├── input.rs       (INPUT_LOCK_LEVEL: AtomicU8, LockLevel enum, PREV_LOCK_LEVEL)
├── display.rs     (AppState: surface_quiesced, last_flushed_snapshot)
└── main.rs        (McTriggerState, F3/Ctrl-Up interception, mc_exit handler, pending_focus)
```

---

## 17. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Filesystem thumbnail delivery grants `/data` access | Removed filesystem requirement; thumbnails delivered entirely via stdin as base64 `VYOMA_MC:thumb_begin/data/end` framing; `draw_thumb` uses MC-app in-memory buffer |
| B2: `mc_input_locked` + `chrome_input_locked` simultaneously true | Single `Arc<AtomicU8> INPUT_LOCK_LEVEL` with priority levels: None(0), MissionControl(1), ChromeConsent(2). Consent always wins. `PREV_LOCK_LEVEL` saves state for restore. Auto-clear via `compare_exchange` on lifecycle change |
| B3: Per-app vsync_lock release/reacquire causes writer starvation | Dedicated `capture_thread_main`; single `vsync_lock.read()` held for entire multi-app snapshot; released before all I/O; main event loop posts request and returns immediately |
| B4: Suspended app surface mutations during capture | `surface_quiesced: bool` flag on `AppState`; set on suspend, cleared on resume; draw_cmd handler discards all commands when flag set; `last_flushed_snapshot: Option<Arc<Vec<u8>>>` provides stable capture source |
| B5: Focus command applied to wrong space during space switch | `pending_focus: Option<String>` in `SupervisorState`; focus command deferred if `PENDING_SPACE_SWITCH != 0`; compositor applies deferred focus AFTER space switch commit in same tick |
