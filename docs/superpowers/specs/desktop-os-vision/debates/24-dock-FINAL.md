# FINAL Spec: Dock & App Switcher (Round 24)

**Subsystem**: Dock & App Switcher  
**macOS Analogue**: Dock (bottom bar) + Cmd-Tab app switcher  
**Depends on**: R21 (WM space-0, PerSpaceZ), R22 (lifecycle), R23 (chrome, y-clip, resize_surface)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

The Dock is a second privileged WASM app (`dock`) in space=0, positioned at the bottom of
the screen. It renders app icon slots (colored rectangles + labels) and running indicators.
The Ctrl-Tab switcher is an overlay drawn by dock when triggered.

---

## 2. Architecture

### 2.1 Dock as space=0 WASM App

```toml
# apps/dock/vyoma.toml
[app]
name = "dock"
version = "1.0.0"
wasm = "dock.wasm"

[capabilities]
display = true
stdio   = true
shell   = true   # @supervisor: focus <app>
```

`win_space = 0`, `win_y = logical_h - 48`, `win_w = logical_w`, `win_h = 48`.
Z-index: 65534. Chrome Z-index: 65535 (chrome blitted last, above dock).

### 2.2 Compositor Model A — Explicit Two-Pass Blit (B1 fix)

R21 introduced space-0 as always-on-top. With two space-0 apps (chrome + dock), the
compositor must use **Model A** (separate passes):

```rust
// supervisor/src/compositor.rs — flush_pass() — REQUIRED change for R24

fn flush_pass(fb: &mut FrameBuffer, apps_sorted: &[AppSnapshot], config: &DisplayConfig,
              chrome_surface_ready: bool, dock_surface_ready: bool)
{
    let chrome_h_px = (24u32).saturating_mul(config.scale_factor as u32);
    let dock_clip_y_px = config.physical_height
        .saturating_sub((48u32).saturating_mul(config.scale_factor as u32));

    // Fallback fills if system apps not yet ready
    if !chrome_surface_ready {
        fb.fill_rows(0, chrome_h_px, MENU_BAR_BG_COLOR);
    }
    if !dock_surface_ready {
        fb.fill_rows(dock_clip_y_px, config.physical_height - dock_clip_y_px, MENU_BAR_BG_COLOR);
    }

    // Pass 1: active-space apps (space-N, N = active space)
    let mut space_n: Vec<_> = apps_sorted.iter()
        .filter(|a| a.win_space != 0)
        .collect();
    space_n.sort_by_key(|a| a.z_index);
    for snap in &space_n {
        blit_clipped(fb, snap, config, chrome_h_px, dock_clip_y_px);
    }

    // Pass 2: space-0 apps (always on top, sorted by z ascending)
    let mut space_0: Vec<_> = apps_sorted.iter()
        .filter(|a| a.win_space == 0)
        .collect();
    space_0.sort_by_key(|a| a.z_index);  // dock(65534) before chrome(65535)
    for snap in &space_0 {
        blit_clipped(fb, snap, config, chrome_h_px, dock_clip_y_px);
    }
}
```

**Model A invariant**: All space-0 windows are composited above all space-N windows
regardless of numeric Z values. Within space-0, higher Z = blitted later = on top.
Dock(65534) appears under Chrome(65535). This model is now normative.

### 2.3 Dock Startup Fallback

`dock_surface_ready: bool` flag (false at boot, set true on dock's first flush). When
false, supervisor fills bottom 48px with `MENU_BAR_BG_COLOR = 0x1E1E2EFF`. Mirrors R23's
`chrome_surface_ready` pattern exactly.

---

## 3. Data Model (B4 fix — `mru_rank` removed)

```rust
// apps/dock/src/state.rs

pub struct DockSlot {
    pub app_name: String,
    pub display_name: String,   // truncated to 12 chars for display
    pub lifecycle: LifecycleState,
    pub is_focused: bool,
    // NO mru_rank field — MRU ordering lives in DockState.mru_list only
}

pub struct DockState {
    pub slots: Vec<DockSlot>,         // spawn order; drives dock strip rendering
    pub mru_list: Vec<String>,        // MRU order; drives switcher rendering
    pub switcher_index: usize,        // current selection in mru_list (0 when inactive)
    pub switcher_active: bool,
    pub focused_app: Option<String>,
}
```

**Single source of truth**: `mru_list: Vec<String>` is the only MRU representation.
`DockSlot` contains only per-slot rendering state. No rank fields to maintain.

### 3.1 MRU Update Rules

- App focused: move to index 0 of `mru_list` (remove-and-prepend)
- App launched: append to `mru_list` at end (lowest priority)
- App exited: remove from `mru_list`
- Minimize: does NOT change MRU position (app remains in list)

---

## 4. Dock Supervisor Routing

### 4.1 `DockRouter` — identical pattern to `ChromeRouter` (R23)

```rust
// supervisor/src/dock_router.rs

pub struct DockRouter {
    queue: VecDeque<String>,
    dock_running: bool,
}

// Same API as ChromeRouter: route(), on_dock_running(), on_dock_not_running(),
// on_dock_terminated_no_restart()
// Drop-oldest overflow at capacity 64 (same policy as ChromeRouter)
```

### 4.2 Events Routed to Dock

```
VYOMA_DOCK:focus_changed:<app_name>
VYOMA_DOCK:app_launched:<app_name>,<display_name>
VYOMA_DOCK:app_exited:<app_name>
VYOMA_DOCK:lifecycle_changed:<app_name>,<state>
VYOMA_DOCK:ctrl_tab_pressed           ← from TTY Ctrl-Tab interception
VYOMA_DOCK:ctrl_released              ← synthetic on Ctrl-up OR 500ms timeout
```

---

## 5. Ctrl-Tab Global Shortcut (B2 fix — channel model)

### 5.1 Channel-Based `dock_ctrl_held`

The TTY input thread sends key events to the main event loop via an existing
`crossbeam::channel`. No shared `bool` is needed — state lives entirely in the main
event loop thread:

```rust
// supervisor/src/main.rs — main event loop state (NOT shared with TTY thread)

struct DockSwitcherState {
    ctrl_held: bool,
    last_ctrl_tab_at: Instant,
}
```

The TTY input thread sends raw key events (`KeyEvent { key, ctrl_held, alt_held, ... }`)
over the channel. The main event loop reads these and updates `DockSwitcherState`.

### 5.2 Ctrl-Tab Interception

```rust
// supervisor/src/main.rs — inside key event processing

fn process_key_event(ev: KeyEvent, switcher: &mut DockSwitcherState,
                     dock_router: &DockRouter, dock_tx: &Sender<String>) {
    if ev.key == Key::Tab && ev.ctrl_held {
        switcher.ctrl_held = true;
        switcher.last_ctrl_tab_at = Instant::now();
        dock_router.route("VYOMA_DOCK:ctrl_tab_pressed\n".into(), dock_tx);
        return;  // do NOT forward to focused app
    }
    if !ev.ctrl_held && switcher.ctrl_held {
        // Ctrl was released
        switcher.ctrl_held = false;
        dock_router.route("VYOMA_DOCK:ctrl_released\n".into(), dock_tx);
    }
    if switcher.ctrl_held {
        // Other keys while Ctrl is held — do NOT forward to focused app
        return;
    }
    // Normal key routing to focused app
    forward_key_to_focused_app(ev);
}
```

### 5.3 500ms Timeout Synthetic Ctrl-Release

On each supervisor tick:
```rust
if switcher.ctrl_held && Instant::now() - switcher.last_ctrl_tab_at > Duration::from_millis(500) {
    switcher.ctrl_held = false;
    dock_router.route("VYOMA_DOCK:ctrl_released\n".into(), dock_tx);
}
```

### 5.4 Auto-Clear on Dock Exit

When dock's lifecycle transitions away from Running:
```rust
// supervisor/src/lifecycle/events.rs

pub fn on_dock_lifecycle_change(new_state: LifecycleState, switcher: &mut DockSwitcherState) {
    if new_state != LifecycleState::Running {
        switcher.ctrl_held = false;
    }
}
```

---

## 6. Switcher Overlay and `resize_surface` Ordering (B5 fix)

### 6.1 Open Switcher

```
1. Receive VYOMA_DOCK:ctrl_tab_pressed
2. Build switcher display list from mru_list
3. VYOMA_DRAW:resize_surface:<logical_w>,<logical_h>    ← expand to full screen
4. Draw switcher overlay (full screen BGRA fill + app list)
5. VYOMA_DRAW:flush                                      ← REQUIRED before any further resize
6. Set switcher_active = true
```

### 6.2 Cycle Selection

```
On each subsequent VYOMA_DOCK:ctrl_tab_pressed:
  switcher_index = (switcher_index + 1) % mru_list.len()
  Redraw switcher at current size (no resize_surface needed)
  VYOMA_DRAW:flush
```

### 6.3 Close Switcher (B5 fix)

```
On VYOMA_DOCK:ctrl_released:
  selected = mru_list[switcher_index]
  @supervisor: focus <selected>
  switcher_active = false
  switcher_index = 0
  NOTE: collapse resize_surface is issued ONLY AFTER the flush in step 5 above.
  VYOMA_DRAW:resize_surface:<logical_w>,48               ← collapse; safe after flush
  Redraw dock strip at normal size
  VYOMA_DRAW:flush
```

**Ordering guarantee**: `resize_surface` commands are processed in stdout stream order
by the supervisor's sequential output-reading thread. No coalescing. The flush in step 5
(open) ensures the expanded surface is committed to the compositor before collapse is issued.

---

## 7. Combined Top+Bottom Clip (B3 fix — 6-step formula)

```rust
// supervisor/src/compositor.rs — blit_clipped()

fn blit_clipped(fb: &mut FrameBuffer, snap: &AppSnapshot, config: &DisplayConfig,
                chrome_h_px: u32, dock_clip_y_px: u32)
{
    let win_y_px = snap.win_y_pts.saturating_mul(config.scale_factor as u32);
    let win_x_px = snap.win_x_pts.saturating_mul(config.scale_factor as u32);

    // Step 1: top-edge clip (R23 formula)
    let effective_dest_y = if snap.app_name == "chrome" || snap.app_name == "dock" {
        win_y_px
    } else {
        win_y_px.max(chrome_h_px)
    };

    // Step 2: rows clipped away from top of surface
    let src_y_offset = effective_dest_y.saturating_sub(win_y_px);

    // Step 3: height after top clip
    let blittable_h_after_top_clip = snap.surface_height_px.saturating_sub(src_y_offset);

    // Step 4: bottom of blittable region (before bottom clip)
    let blittable_bottom = (effective_dest_y as i64) + (blittable_h_after_top_clip as i64);

    // Step 5: apply bottom-edge clip
    let clip_bottom = if snap.app_name == "chrome" || snap.app_name == "dock" {
        config.physical_height as i64
    } else {
        dock_clip_y_px as i64
    };
    let clipped_bottom = blittable_bottom.min(clip_bottom);

    // Step 6: final blittable height
    let blittable_h_combined = (clipped_bottom - effective_dest_y as i64).max(0) as u32;

    if blittable_h_combined == 0 { return; }

    blit_surface(fb, &snap.surface, win_x_px, effective_dest_y,
                 snap.surface_width_px, blittable_h_combined, src_y_offset);
}
```

All six steps are explicit. `blittable_h_from_top` (the undefined variable in the
original spec) is replaced by the explicitly computed `blittable_h_after_top_clip`.

---

## 8. VYOMA_DOCK: Protocol

### 8.1 Dock → Supervisor

```
@supervisor: focus <app_name>
@supervisor: dock_ready                 ← dock signals it has drawn initial state
```

### 8.2 Supervisor → Dock (stdin push)

```
VYOMA_DOCK:focus_changed:<app_name>
VYOMA_DOCK:app_launched:<app_name>,<display_name>
VYOMA_DOCK:app_exited:<app_name>
VYOMA_DOCK:lifecycle_changed:<app_name>,<state>
VYOMA_DOCK:ctrl_tab_pressed
VYOMA_DOCK:ctrl_released
```

---

## 9. Dock Drawing

Dock strip layout (48pt height):
- Slot width: `logical_w / max(slots.len(), 1)`, min 60pt, max 120pt
- Each slot: colored rectangle (green=Running, gray=Suspended, dim=Terminated)
- Focused slot: bright border color
- Running indicator: 4pt dot below slot center
- Font: `m` size (8×16 pts), slot label centered

Switcher overlay:
- Full-screen dark overlay (RGBA 0x00000099 blended on top)
- App list centered, `l` font (16×32 pts), selected entry highlighted
- Maximum visible entries: `(logical_h - 128) / 48`

---

## 10. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Dock strip | yes | no | no | no |
| Ctrl-Tab switcher | yes | no | no | no |
| Bottom 48pt clip | yes | no | no | no |
| `dock` WASM app | yes | no | no | no |

---

## 11. File Layout

```
apps/dock/src/
├── main.rs        (event loop: stdin parsing, state updates, draw calls)
├── state.rs       (DockState, DockSlot — no mru_rank)
└── draw.rs        (render_dock_strip, render_switcher_overlay)

supervisor/src/
├── dock_router.rs (DockRouter — mirror of chrome_router.rs)
├── compositor.rs  (flush_pass: Model A two-pass, blit_clipped, dock_surface_ready fallback)
└── main.rs        (DockSwitcherState, Ctrl-Tab channel processing, 500ms timeout)
```

---

## 12. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Two space-0 apps z-order | Model A explicit: Pass 1 = space-N apps (z ascending), Pass 2 = space-0 apps (z ascending). Dock(65534) under Chrome(65535). Normative for all future rounds. |
| B2: `dock_ctrl_held` data race | Channel model: TTY sends `KeyEvent` to main loop; `DockSwitcherState.ctrl_held` lives in event loop only — no shared bool, no AtomicBool. Auto-clears on dock lifecycle ≠ Running. |
| B3: Combined clip missing variable | 6-step explicit formula: `blittable_h_after_top_clip` defined before use; `blittable_h_combined` correctly uses top-clipped height as input to bottom clip. |
| B4: `mru_rank` field undefined | Removed from `DockSlot` entirely. MRU = `mru_list: Vec<String>` in `DockState`. Single source of truth. |
| B5: Double resize_surface unsynchronised | Flush-before-collapse requirement stated explicitly. Step 5 flush in open-switcher must precede any collapse resize_surface. Sequential stdout processing enforces order. |
