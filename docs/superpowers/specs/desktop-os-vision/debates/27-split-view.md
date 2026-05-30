# Spec: Full Screen & Split View (Round 27)

**Subsystem**: Full Screen & Split View  
**macOS Analogue**: Full Screen / Split View / Tile Window (macOS 13+)  
**Depends on**: R21 (WM spaces, PerSpaceZ, pending_resize), R22 (lifecycle), R23 (chrome),
R24 (dock), R25 (MC, INPUT_LOCK_LEVEL, surface_quiesced), R26 (Stage Manager, blit_clipped,
stage_offscreen, anim_x_override, anim_alpha)  
**Status**: DRAFT — pending critique resolution

---

## 1. Overview

Full Screen and Split View are window-level presentation modes that expand one or two apps
to fill the entire screen region below chrome and above the dock. They interact with the
Space model (R21), Mission Control (R25), Stage Manager (R26), and the compositor
(blit_clipped, R26).

**Three modes defined in this round**:

| Mode | Apps | Area occupied |
|------|------|---------------|
| Full Screen | 1 | entire logical screen (0,0) to (logical_w, logical_h) — chrome and dock hidden |
| Split View | 2 | both apps share the full screen with an adjustable split ratio; chrome and dock hidden |
| Tile Window | 1 | single app occupies one half of the content area; opposite half is the normal WM space; chrome and dock visible |

**What is deferred to R32**:
- Mouse-driven drag to move the split handle
- Cursor change when hovering the split handle
- Full Screen enter/exit via green traffic-light button hover/click
- Dock reveal via cursor hover at screen edge while full-screen

---

## 2. State Additions to AppState

```rust
// supervisor/src/display.rs — additions to AppState

pub struct AppState {
    // ... all existing fields from R21–R26 ...

    // --- R27: Full Screen / Split View ---
    pub fs_mode: FullScreenMode,          // current presentation mode for this app
    pub fs_space: Option<u8>,             // dedicated FS space number (FullScreen/SplitView only)
    pub split_role: SplitRole,            // None, Left/Top, Right/Bottom
    pub split_ratio: Option<f32>,         // 0.1–0.9; only set on the "primary" member
    pub split_axis: SplitAxis,            // Horizontal or Vertical (default Horizontal)
    pub fs_pre_x: u32,                    // win_x before FS/Split enter (restore point)
    pub fs_pre_y: u32,                    // win_y before FS/Split enter
    pub fs_pre_w: u32,                    // win_w before FS/Split enter
    pub fs_pre_h: u32,                    // win_h before FS/Split enter
    pub fs_pre_space: u8,                 // win_space before FS/Split enter (restore point)
    pub tile_side: TileSide,              // None, Left, Right (Tile Window only)
    pub fs_chrome_hidden: bool,           // true while chrome/dock are suppressed
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum FullScreenMode {
    Normal,       // default — regular windowed mode
    FullScreen,   // single-app full screen (chrome + dock hidden)
    SplitView,    // two-app split (chrome + dock hidden)
    TileWindow,   // single-app half-screen tile (chrome + dock visible)
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SplitRole {
    None,
    Primary,      // owns the split_ratio; positioned first
    Secondary,    // receives the complementary ratio
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SplitAxis {
    Horizontal,   // left | right split (default)
    Vertical,     // top | bottom split
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum TileSide {
    None,
    Left,
    Right,
}
```

**Default values** (on `AppState::new`):
```rust
fs_mode: FullScreenMode::Normal,
fs_space: None,
split_role: SplitRole::None,
split_ratio: None,
split_axis: SplitAxis::Horizontal,
fs_pre_x: 0, fs_pre_y: 0, fs_pre_w: 0, fs_pre_h: 0, fs_pre_space: 1,
tile_side: TileSide::None,
fs_chrome_hidden: false,
```

---

## 3. Supervisor-Level FS Registry

A single `FsRegistry` tracks all active full-screen and split-view pairs so the compositor
and MC space-strip can query them efficiently.

```rust
// supervisor/src/fs/registry.rs

pub struct FsPair {
    pub primary: String,        // app name
    pub secondary: String,      // app name (SplitView only; empty for FullScreen)
    pub space: u8,              // dedicated space number
    pub axis: SplitAxis,
    pub ratio: f32,             // 0.1–0.9 (default 0.5)
}

pub struct FsRegistry {
    /// One entry per active full-screen space
    pub pairs: Vec<FsPair>,
    /// Dedicated FS spaces (1-9 bucket; allocated via SpaceRegistry)
    pub fs_spaces: Vec<u8>,
}

impl FsRegistry {
    pub fn find_by_app(&self, app: &str) -> Option<&FsPair> {
        self.pairs.iter().find(|p| p.primary == app || p.secondary == app)
    }
    pub fn find_by_space(&self, space: u8) -> Option<&FsPair> {
        self.pairs.iter().find(|p| p.space == space)
    }
}
```

`FsRegistry` is owned by `SupervisorState` alongside `StageRegistry` and `SpaceRegistry`.

---

## 4. Full Screen Mode

### 4.1 Enter Full Screen Sequence

Triggered by: IPC command `@supervisor: fullscreen_enter <app_name>` (or
`VYOMA_WM:fullscreen:<app_id>` from the WM protocol — R21). The chrome app may also
issue this on behalf of the user via a window button press (deferred to R32 for mouse;
keyboard shortcut is Ctrl-Shift-F in this round).

```
Step 1: Save restore point
  app.fs_pre_x = app.win_x
  app.fs_pre_y = app.win_y
  app.fs_pre_w = app.win_w
  app.fs_pre_h = app.win_h
  app.fs_pre_space = app.win_space

Step 2: Allocate a dedicated FS space
  new_space = space_registry.allocate_next()  (see §4.2)
  app.fs_space = Some(new_space)

Step 3: Move app to FS space and resize
  app.win_space = new_space
  app.win_x = 0
  app.win_y = 0
  app.pending_resize = Some((logical_w, logical_h))   // R21 drain path
  app.fs_mode = FullScreenMode::FullScreen
  app.fs_chrome_hidden = true

Step 4: Hide chrome + dock surfaces (§4.3)

Step 5: Switch to the new FS space
  PENDING_SPACE_SWITCH.store(new_space, Ordering::Release)

Step 6: Notify app via VYOMA_FS: protocol (§10)
  send to app stdin: "VYOMA_FS:entered_fullscreen\n"
```

All steps happen on the IPC handler thread. The compositor drains `pending_resize` on
the next tick under `vsync_lock.write()` (R21 §6.3 mechanism — unchanged).

### 4.2 Dedicated FS Space Allocation

Full-screen apps get their own dedicated space so they appear as separate spaces in
Mission Control (§8). Space allocation:

```rust
// supervisor/src/fs/registry.rs

impl FsRegistry {
    pub fn allocate_fs_space(space_reg: &mut SpaceRegistry) -> Option<u8> {
        if space_reg.count >= 9 { return None; }   // hard cap
        space_reg.count += 1;
        let s = space_reg.count;
        Some(s)
    }

    pub fn release_fs_space(space: u8, space_reg: &mut SpaceRegistry,
                             apps: &mut HashMap<String, AppState>) {
        // Move any remaining apps out of this space first
        for app in apps.values_mut().filter(|a| a.win_space == space) {
            app.win_space = 1;   // fallback to default space
        }
        // Shrink space count if this was the last space
        if space == space_reg.count && space_reg.count > 1 {
            space_reg.count -= 1;
        }
        // Remove from fs_spaces list
    }
}
```

**Space numbering**: FS spaces are appended at the high end of the current space list.
If spaces 1..3 already exist, the first FS app gets space 4. MC space-strip shows this
as a new card to the right with a "full-screen" badge icon.

### 4.3 Chrome and Dock Suppression

Chrome and dock are space=0 apps (always composited). They cannot simply be "moved off-
screen" for a single space. Instead, suppression is communicated via a new supervisor
global:

```rust
// supervisor/src/fs/suppression.rs

/// Bitmask: bit N set means the app named by FS_SUPPRESSED_APPS[N] is suppressed
/// while the active space is a FS space. Checked in blit_clipped.
pub static FS_CHROME_SUPPRESSED: AtomicBool = AtomicBool::new(false);
pub static FS_DOCK_SUPPRESSED: AtomicBool = AtomicBool::new(false);
```

`blit_clipped` adds a suppression check for space=0 apps before blitting:

```rust
// supervisor/src/compositor.rs — in blit_clipped (after R26 Step 0)

// FS suppression: skip chrome/dock when a FS space is active
if snap.is_space0 {
    if snap.app_name == "chrome" && FS_CHROME_SUPPRESSED.load(Ordering::Acquire) {
        return;
    }
    if snap.app_name == "dock" && FS_DOCK_SUPPRESSED.load(Ordering::Acquire) {
        return;
    }
}
```

**Set on space switch** (in the compositor tick, after `PENDING_SPACE_SWITCH` is applied):

```rust
// supervisor/src/compositor.rs — vsync_tick(), after space switch block

let is_fs_space = fs_registry.find_by_space(spaces.active).is_some();
FS_CHROME_SUPPRESSED.store(is_fs_space, Ordering::Release);
FS_DOCK_SUPPRESSED.store(is_fs_space, Ordering::Release);
```

This means chrome and dock are still rendered (pass 2 iterates them) but the per-app
suppression check short-circuits before any pixels are written. No surface destruction
or lifecycle change occurs.

**Animation**: On FS enter, chrome and dock fade out over 8 frames (133ms at 60fps)
before the space switch. A `fs_chrome_fade_out` animation state is tracked in
compositor state (similar to `StageAnimation` from R26):

```rust
// supervisor/src/fs/animation.rs

pub struct FsEnterAnimation {
    pub app: String,
    pub t: f32,            // 0.0 → 1.0
    pub dt: f32,           // 1.0 / ANIM_FRAMES
}

pub const FS_ANIM_FRAMES: u32 = 8;

// chrome and dock global_alpha set to (1.0 - t) during fade-out
// app surface global_alpha set to t during fade-in from 0→full
```

For simplicity: `app.anim_alpha` (R26) is reused for the FS app's fade-in. Chrome and
dock use a separate `fs_overlay_alpha: f32` stored in `FsEnterAnimation`; `blit_clipped`
reads this for space=0 apps during the animation.

---

## 5. Exit Full Screen

### 5.1 Exit Triggers

1. **Keyboard**: Escape key when `INPUT_LOCK_LEVEL == None` and active space is an FS space.
   The IPC handler calls `handle_fullscreen_exit`.
2. **IPC command**: `@supervisor: fullscreen_exit <app_name>` (e.g., from chrome or the
   app itself via `VYOMA_FS:` protocol).
3. **App process exits**: `on_app_exited` checks if app was in FS mode; calls
   `handle_fullscreen_exit` to clean up.
4. **Space switch to non-FS space**: FS is not automatically exited — the FS space remains.
   Only explicit exit collapses the FS space.

### 5.2 Exit Sequence

```
Step 1: Restore geometry
  app.win_x = app.fs_pre_x
  app.win_y = app.fs_pre_y
  app.pending_resize = Some((app.fs_pre_w, app.fs_pre_h))   // R21 drain path
  app.win_space = app.fs_pre_space

Step 2: Release FS space
  fs_registry.release_fs_space(app.fs_space.take(), &mut space_reg, &mut apps)
  app.fs_mode = FullScreenMode::Normal
  app.fs_chrome_hidden = false

Step 3: Clear suppression flags (will be recomputed on next space switch)
  FS_CHROME_SUPPRESSED.store(false, Ordering::Release)
  FS_DOCK_SUPPRESSED.store(false, Ordering::Release)

Step 4: Switch back to original space
  PENDING_SPACE_SWITCH.store(app.fs_pre_space, Ordering::Release)

Step 5: Animate chrome/dock fade-in (8 frames, alpha 0→1)

Step 6: Notify app
  send to app stdin: "VYOMA_FS:exited_fullscreen\n"
```

### 5.3 Escape Key Routing for FS Exit

The Escape key is intercepted in the TTY input thread before routing to the focused app:

```rust
// supervisor/src/main.rs — process_key_event (extends R25 logic)

fn process_key_event(ev: KeyEvent, state: &SupervisorState) {
    // Highest priority: existing INPUT_LOCK_LEVEL routing (R25, R26)
    if ev.key == Key::Escape {
        let active = state.spaces.active;
        if fs_registry.find_by_space(active).is_some()
            && INPUT_LOCK_LEVEL.load(Ordering::Acquire) == LockLevel::None as u8
        {
            // Exit fullscreen for the app in this FS space
            if let Some(pair) = fs_registry.find_by_space(active) {
                handle_fullscreen_exit(&pair.primary.clone(), state);
            }
            return;
        }
    }
    // Normal routing
    route_by_lock_level(ev, state);
}
```

---

## 6. Full Screen and Spaces

### 6.1 Dedicated Space per Full-Screen App

Each full-screen app occupies its own dedicated space. This mirrors macOS behavior:
in Mission Control, full-screen spaces appear as separate cards in the space strip.

**Invariant**: A dedicated FS space always has exactly one app in `FullScreen` mode or
exactly two apps in `SplitView` mode. No other apps are assigned to it.

**Space numbering scheme**: The `FsRegistry.fs_spaces` list tracks which space numbers
are FS-dedicated. The human-facing MC space-strip shows them with a badge:
- Full Screen spaces: single-app thumbnail with full-bleed
- Split View spaces: two-thumbnail split card

### 6.2 Space Switching to/from FS Spaces

Switching to a FS space (via MC or keyboard shortcut) triggers the standard
`PENDING_SPACE_SWITCH` mechanism. The compositor applies the switch; the
`FS_CHROME_SUPPRESSED` / `FS_DOCK_SUPPRESSED` flags are updated based on
`fs_registry.find_by_space(new_active)`.

Switching away from a FS space does NOT exit full screen — it simply activates a
different space (normal or another FS space). The FS app continues running in its
dedicated space but chrome/dock are restored for the now-active non-FS space.

### 6.3 Space Registry Limits

Max 9 spaces total (R21 hard limit). With 8 normal spaces + 1 FS app, no more FS spaces
can be allocated until one is released. The supervisor logs a warning and refuses
`fullscreen_enter` if `space_reg.count >= 9`:

```rust
if space_reg.count >= 9 {
    send_to_app_stdin(app_name, "VYOMA_FS:error:no_space_available\n");
    return;
}
```

---

## 7. Split View

### 7.1 Enter Split View Sequence

Split View requires two apps. The canonical entry path is:

1. App A enters Full Screen (§4) — gets a dedicated space.
2. User selects App B to share the space (via MC in this round; R32 adds drag-to-pair).
3. Supervisor promotes the space from FullScreen to SplitView.

IPC commands:
- `@supervisor: splitview_enter <app_a> <app_b>` — enters SplitView directly (both apps
  assigned to the same dedicated space at 50/50 ratio).
- `@supervisor: splitview_add_to_fullscreen <app_b>` — adds app_b to the current FS space
  (converts from FullScreen to SplitView mode for the active space).

**Enter Split View sequence (direct)**:

```
Step 1: Save restore points for both apps
  For each of app_a, app_b:
    save fs_pre_x/y/w/h/space (same as FS enter §4.1 Step 1)

Step 2: Allocate dedicated space
  new_space = fs_registry.allocate_fs_space(&mut space_reg)

Step 3: Assign both apps to new_space
  app_a.win_space = new_space;  app_a.fs_mode = SplitView;  app_a.split_role = Primary;
  app_a.split_ratio = Some(0.5);  app_a.split_axis = SplitAxis::Horizontal;
  app_b.win_space = new_space;  app_b.fs_mode = SplitView;  app_b.split_role = Secondary;

Step 4: Register pair
  fs_registry.pairs.push(FsPair {
      primary: app_a.name.clone(), secondary: app_b.name.clone(),
      space: new_space, axis: SplitAxis::Horizontal, ratio: 0.5
  });

Step 5: Compute and apply geometry (§7.2)

Step 6: Suppress chrome + dock (same as FS, §4.3)

Step 7: Switch to new_space via PENDING_SPACE_SWITCH

Step 8: Notify both apps
  "VYOMA_FS:entered_splitview:primary\n"   → app_a
  "VYOMA_FS:entered_splitview:secondary\n" → app_b
```

### 7.2 Split View Geometry Computation

```rust
// supervisor/src/fs/layout.rs

pub fn compute_split_rects(
    axis: SplitAxis,
    ratio: f32,
    logical_w: u32,
    logical_h: u32,
) -> (Rect, Rect) {
    let ratio = ratio.clamp(0.1, 0.9);
    match axis {
        SplitAxis::Horizontal => {
            let w_primary = ((logical_w as f32) * ratio) as u32;
            let w_secondary = logical_w - w_primary;
            (
                Rect { x: 0, y: 0, w: w_primary, h: logical_h },
                Rect { x: w_primary, y: 0, w: w_secondary, h: logical_h },
            )
        }
        SplitAxis::Vertical => {
            let h_primary = ((logical_h as f32) * ratio) as u32;
            let h_secondary = logical_h - h_primary;
            (
                Rect { x: 0, y: 0, w: logical_w, h: h_primary },
                Rect { x: 0, y: h_primary, w: logical_w, h: h_secondary },
            )
        }
    }
}

pub fn apply_split_geometry(
    app_a: &mut AppState, app_b: &mut AppState,
    pair: &FsPair, config: &DisplayConfig,
) {
    let (rect_a, rect_b) = compute_split_rects(pair.axis, pair.ratio,
                                                config.logical_width, config.logical_height);
    app_a.win_x = rect_a.x; app_a.win_y = rect_a.y;
    app_a.pending_resize = Some((rect_a.w, rect_a.h));
    app_b.win_x = rect_b.x; app_b.win_y = rect_b.y;
    app_b.pending_resize = Some((rect_b.w, rect_b.h));
}
```

Both `pending_resize` values are drained by the compositor on the next tick (R21 §6.3
— unchanged mechanism).

### 7.3 Exit Split View

Either app can exit Split View; the result is that the exiting app returns to normal
windowed mode and the remaining app converts to Full Screen mode in the same dedicated
space:

```
@supervisor: splitview_exit_both             ← both apps return to Normal
@supervisor: splitview_exit_app <app_name>   ← named app exits; other converts to FS
```

**`splitview_exit_app` sequence**:

```
Step 1: Restore exiting app geometry
  exiting_app: restore fs_pre_x/y/w/h/space; set fs_mode = Normal; clear split_role

Step 2: Promote remaining app to FullScreen
  remaining.win_x = 0; remaining.win_y = 0;
  remaining.pending_resize = Some((logical_w, logical_h))
  remaining.fs_mode = FullScreen; remaining.split_role = None;
  remaining.split_ratio = None

Step 3: Update FsPair in FsRegistry
  pair.secondary = ""; (marks as single-app FS)
  pair.ratio = 1.0;   (unused but cleared)

Step 4: Send VYOMA_FS notifications
  "VYOMA_FS:exited_splitview\n"     → exiting app
  "VYOMA_FS:promoted_to_fullscreen\n" → remaining app
```

**`splitview_exit_both` sequence**:
Both apps follow the FS exit sequence (§5.2) simultaneously. The dedicated space is
released. `PENDING_SPACE_SWITCH` is set to the primary app's `fs_pre_space`.

---

## 8. Split Resizing

### 8.1 State

The split handle position is encoded entirely by `split_ratio: Option<f32>` on the
primary app's `AppState` and mirrored in `FsPair.ratio`. There is no separate handle
widget surface — the supervisor computes the divider pixel position from the ratio.

### 8.2 IPC Command for Ratio Update

Mouse-driven drag is deferred to R32. For this round, the split ratio can be changed
only via IPC:

```
@supervisor: split_ratio <app_name> <ratio>
```

Where `<ratio>` is a float string `0.10`–`0.90`. The supervisor validates, updates
`FsPair.ratio` and both apps' geometries:

```rust
// supervisor/src/fs/commands.rs

pub fn handle_split_ratio(app_name: &str, ratio_str: &str,
                           state: &mut SupervisorState) {
    let ratio: f32 = match ratio_str.parse() {
        Ok(r) if (0.1..=0.9).contains(&r) => r,
        _ => return,
    };
    let pair = match state.fs_registry.pairs.iter_mut()
        .find(|p| p.primary == app_name || p.secondary == app_name)
    {
        Some(p) => p,
        None => return,
    };
    pair.ratio = ratio;
    let (app_a_name, app_b_name) = (pair.primary.clone(), pair.secondary.clone());
    let (rect_a, rect_b) = compute_split_rects(pair.axis, ratio,
                                                state.config.logical_width,
                                                state.config.logical_height);
    if let Some(a) = state.apps.get_mut(&app_a_name) {
        a.win_x = rect_a.x; a.win_y = rect_a.y;
        a.pending_resize = Some((rect_a.w, rect_a.h));
        a.split_ratio = Some(ratio);
    }
    if let Some(b) = state.apps.get_mut(&app_b_name) {
        b.win_x = rect_b.x; b.win_y = rect_b.y;
        b.pending_resize = Some((rect_b.w, rect_b.h));
    }
    // Notify both apps
    send_to_app_stdin(&app_a_name, &format!("VYOMA_FS:split_ratio_changed:{ratio:.3}\n"));
    send_to_app_stdin(&app_b_name, &format!("VYOMA_FS:split_ratio_changed:{:.3}\n", 1.0 - ratio));
}
```

### 8.3 R32 Hook: Split Handle Drag

The state required by R32 for mouse-driven split resizing:
- `FsPair.ratio: f32` — updated live as the handle is dragged
- `FsPair.axis: SplitAxis` — determines whether drag is horizontal or vertical
- `split_handle_dragging: bool` in `SupervisorState` — set while a drag is in progress
  (blocks other WM input)
- The `handle_split_ratio` function in `fs/commands.rs` is called on each `mousemove`
  event during drag with the new computed ratio (R32 calls this directly)

The handle bounding rect (for R32 hit-testing) is computed from:
```
handle_x = (FsPair.ratio * logical_w) * scale_factor  (Horizontal axis)
handle_y = (FsPair.ratio * logical_h) * scale_factor  (Vertical axis)
handle_thickness_px = 8 * scale_factor
```

---

## 9. Tile Window

### 9.1 Overview

Tile Window pins a single app to the left or right half of the content area
(between chrome and dock). The other half remains available for normal windowed apps.
Chrome and dock remain visible (unlike Full Screen / Split View).

### 9.2 Enter Tile Window

IPC command: `@supervisor: tile_window <app_name> <left|right>`

```
Step 1: Save restore point (same fields as FS: fs_pre_x/y/w/h/space)

Step 2: Compute tile rect (§9.3)

Step 3: Apply geometry
  app.win_x = tile_rect.x;  app.win_y = tile_rect.y;
  app.pending_resize = Some((tile_rect.w, tile_rect.h))
  app.fs_mode = TileWindow;
  app.tile_side = side;
  app.win_manual_layout = true   // disables auto-tiling for this space (R21 §7.1)

Step 4: Notify app
  "VYOMA_FS:tiled:left\n"  or  "VYOMA_FS:tiled:right\n"
```

No dedicated space is allocated. The app stays in its current space.

### 9.3 Tile Window Geometry

```rust
// supervisor/src/fs/layout.rs

const CHROME_H_PTS: u32 = 24;   // R23 constant
const DOCK_H_PTS: u32   = 48;   // R24 constant

pub fn compute_tile_rect(side: TileSide, config: &DisplayConfig) -> Rect {
    let content_y = CHROME_H_PTS;
    let content_h = config.logical_height
        .saturating_sub(CHROME_H_PTS + DOCK_H_PTS);
    let half_w = config.logical_width / 2;
    match side {
        TileSide::Left  => Rect { x: 0,      y: content_y, w: half_w, h: content_h },
        TileSide::Right => Rect { x: half_w, y: content_y, w: config.logical_width - half_w, h: content_h },
        TileSide::None  => unreachable!(),
    }
}
```

Tile ratio is fixed at 50/50 in this round. Dynamic tile ratio is deferred to R32.

### 9.4 Exit Tile Window

```
@supervisor: tile_window_exit <app_name>
```

Sequence:
```
Step 1: Restore geometry
  app.win_x = fs_pre_x; app.win_y = fs_pre_y;
  app.pending_resize = Some((fs_pre_w, fs_pre_h));
  app.fs_mode = Normal;  app.tile_side = None;
  app.win_manual_layout = false  // re-enables auto-tiling if applicable

Step 2: Notify
  "VYOMA_FS:untiled\n"
```

### 9.5 Tile Window and Stage Manager

If Stage Manager is active (`SpaceStages.enabled = true`), tiling is within the main
content area already constrained by the 140pt strip (R26 §6). The tile geometry must
account for the strip:

```rust
// If SM enabled, adjust left tile to start at STRIP_W + MAIN_MARGIN:
if sm_enabled && side == TileSide::Left {
    rect.x = STRIP_W + MAIN_MARGIN;
    rect.w = (config.logical_width / 2).saturating_sub(STRIP_W + MAIN_MARGIN);
}
```

Right tile is unaffected by the strip.

---

## 10. Interaction with Stage Manager

### 10.1 Full Screen Apps and Stage Manager

A full-screen app occupies its own dedicated space. Stage Manager is per-space (R26 §3).
The FS dedicated space has Stage Manager disabled (`SpaceStages.enabled = false` for
that space) — a full-screen space has no stage concept.

When a user enters Full Screen from a Stage Manager-enabled space:
- The app is removed from its stage's `members` (via `on_app_exited_stage` equivalent).
- If the stage becomes empty, normal orphan GC applies (R26 §7).
- The app joins a new dedicated space with SM disabled.

### 10.2 Split View Apps and Stage Manager

Same rule: the dedicated split-view space has `SpaceStages.enabled = false`. Both apps
are removed from their respective stages before entering the split-view space.

### 10.3 Can a Stage Contain a Full-Screen or Split App?

No. An app in `FullScreenMode::FullScreen` or `FullScreenMode::SplitView` has
`win_space = fs_space` (dedicated FS space) where SM is disabled. The app is not a
member of any stage for the duration of its FS/SplitView mode.

On exit from FS/SplitView, the app is returned to its `fs_pre_space`. If that space
has SM enabled and a stage exists, the app is re-added to the active stage's `members`:

```rust
// supervisor/src/fs/commands.rs — after restoring app to fs_pre_space

if let Some(ss) = stage_reg.spaces.get_mut(&app.fs_pre_space) {
    if ss.enabled {
        if let Some(stage) = ss.stages.iter_mut().find(|s| s.id == ss.active) {
            stage.members.push(app.name.clone());
        }
    }
}
```

### 10.4 Tile Window and Stage Manager

A tiled app stays in its current space. Stage Manager remains active. The tiled app
occupies half the main content area; the strip continues to show the stage thumbnails.
Stage switching (selecting a different stage card) exits tile mode for the tiled app:

```rust
// supervisor/src/stage/activation.rs — handle_stage_select (before animation starts)

for app_name in &incoming_members {
    if let Some(app) = apps.get_mut(app_name) {
        if app.fs_mode == FullScreenMode::TileWindow {
            // Exit tile mode before animating into main area
            handle_tile_exit(app_name, apps, config);
        }
    }
}
```

---

## 11. Interaction with Mission Control

### 11.1 Full-Screen Spaces in the Space Strip

MC's space strip (R25 §8) is extended to show FS and SplitView dedicated spaces with
visual differentiation:

**MC `VYOMA_MC:space_type` push notification** (new protocol line added to §13.1):
```
VYOMA_MC:space_type:<space_n>,<type>
```
Where `<type>` is one of: `normal`, `fullscreen`, `splitview`.

MC app renders FS spaces with a different card style (full-bleed thumbnail, no inter-
window gap). The `+` space add button is suppressed next to FS space cards.

### 11.2 Entering Full Screen from Mission Control

MC shows all running apps as thumbnails. Right-clicking (deferred to R32) or pressing
`F` on a selected thumbnail triggers `fullscreen_enter`. For this round, MC sends:

```
@supervisor: fullscreen_enter <app_name>
@supervisor: mc_exit
```

### 11.3 Exiting Full Screen via Mission Control

In MC, FS space cards show an exit button. Pressing Enter on a selected FS space card
triggers space switch only (no exit). The Escape shortcut within a selected FS card
triggers exit:

MC sends: `@supervisor: fullscreen_exit <app_name>`.

### 11.4 Split View Pairing via Mission Control

When a full-screen space card is selected in MC, MC enters a "choose split partner"
sub-mode where the normal space apps are shown. Selecting a second app sends:

```
@supervisor: splitview_add_to_fullscreen <app_b>
@supervisor: mc_exit
```

This converts the existing FS space to SplitView.

### 11.5 Thumbnail Capture for FS Apps

FS apps are regular apps with surfaces. The existing `mc_capture.rs` capture thread
(R25 §5) works unchanged — `CaptureKind::MissionControl { all_spaces: true }` captures
all apps including those in FS spaces.

---

## 12. Interaction with Dock

### 12.1 Dock Auto-Hide in Full Screen

When a Full Screen or Split View space is active, the dock is suppressed
(`FS_DOCK_SUPPRESSED = true`, §4.3). The dock surface is not blitted.

**Dock reveal**: Deferred to R32 (cursor hover at bottom edge). In this round the dock
remains invisible while in a FS/SplitView space.

### 12.2 Dock in Tile Window Mode

The dock remains visible in Tile Window mode (chrome+dock suppression is NOT activated).
Tile geometry accounts for the dock height (§9.3).

### 12.3 Dock Badge Updates

Dock badges (`mru_list`, R24) continue to update for FS/SplitView apps — lifecycle
transitions (Running/Background) fire normally. The dock just doesn't paint while
suppressed.

---

## 13. `VYOMA_FS:` Protocol

### 13.1 Supervisor → App (stdin push)

All messages are line-terminated (`\n`). Apps parse these on their stdin read loop.

```
VYOMA_FS:entered_fullscreen                        ← app entered full screen
VYOMA_FS:exited_fullscreen                         ← app exited full screen
VYOMA_FS:entered_splitview:primary                 ← app entered split view as primary
VYOMA_FS:entered_splitview:secondary               ← app entered split view as secondary
VYOMA_FS:exited_splitview                          ← app left split view
VYOMA_FS:promoted_to_fullscreen                    ← was secondary, partner exited
VYOMA_FS:split_ratio_changed:<ratio>               ← ratio updated (0.000–1.000, 3dp)
VYOMA_FS:tiled:left                                ← entered tile window, left side
VYOMA_FS:tiled:right                               ← entered tile window, right side
VYOMA_FS:untiled                                   ← exited tile window
VYOMA_FS:error:no_space_available                  ← FS enter refused (space cap)
VYOMA_FS:error:invalid_partner:<app>               ← splitview_enter: named app not found
```

### 13.2 App → Supervisor (stdout / IPC)

Apps may request mode changes via stdout IPC:

```
@supervisor: fullscreen_enter <app_name>           ← self or other app
@supervisor: fullscreen_exit <app_name>
@supervisor: splitview_enter <app_a> <app_b>
@supervisor: splitview_exit_both
@supervisor: splitview_exit_app <app_name>
@supervisor: splitview_add_to_fullscreen <app_b>
@supervisor: split_ratio <app_name> <ratio>
@supervisor: tile_window <app_name> <left|right>
@supervisor: tile_window_exit <app_name>
```

### 13.3 WM Protocol Extension

R21 `VYOMA_WM:` protocol is extended:

```
VYOMA_WM:fullscreen:<app_id>
VYOMA_WM:tile:<app_id>,<left|right>
```

These are aliases for the IPC commands above but addressable by `app_id` integer (for
WIT callers). The supervisor translates `app_id` → `app_name` before dispatching.

---

## 14. Compositor Integration

### 14.1 `blit_clipped` — FS Mode Adjustments

No changes to the R26 7-step formula for per-pixel blitting. The FS adjustments are
upstream of `blit_clipped`: apps in FS/SplitView mode have `win_x=0, win_y=0` and
`surface_width_px/height_px` equal to the full physical resolution. `blit_clipped` clips
against chrome_h and dock_clip_y as usual; but:

- `chrome_h_px = 0` when `FS_CHROME_SUPPRESSED` is true (no top clip loss)
- `dock_clip_y_px = physical_height` when `FS_DOCK_SUPPRESSED` is true (no bottom clip loss)

These override values are computed once per compositor tick:

```rust
// supervisor/src/compositor.rs — vsync_tick(), before blit loop

let chrome_clip_px = if FS_CHROME_SUPPRESSED.load(Ordering::Acquire) { 0 }
                      else { (CHROME_H_PTS * config.scale_factor as u32) };
let dock_clip_px   = if FS_DOCK_SUPPRESSED.load(Ordering::Acquire)   { config.physical_height }
                      else { config.physical_height.saturating_sub(DOCK_H_PTS * config.scale_factor as u32) };
```

These replace the fixed constants previously passed to `blit_clipped` calls.

### 14.2 Z-Order for Split View Apps

Both SplitView apps occupy the same dedicated space. They are composited in ascending z
order (R21 two-pass Model A). The primary app gets lower z, secondary gets higher z (so
secondary appears on top where their surfaces would overlap — but with correct geometry
they never overlap):

```rust
// supervisor/src/fs/commands.rs — after apply_split_geometry

app_a.win_z.set(new_space, 10);   // primary: lower z
app_b.win_z.set(new_space, 20);   // secondary: higher z
```

### 14.3 FS Enter/Exit Animations

```rust
// supervisor/src/fs/animation.rs

pub struct FsEnterAnimation {
    pub app_name: String,
    pub mode: FsAnimMode,       // Enter or Exit
    pub t: f32,
    pub dt: f32,                // 1.0 / FS_ANIM_FRAMES (= 1/8)
}

pub enum FsAnimMode { Enter, Exit }

/// Called each compositor tick if Some(anim) is in CompositorState
pub fn advance_fs_anim(
    anim: &mut FsEnterAnimation,
    apps: &mut HashMap<String, AppState>,
    chrome_app: &mut AppState,
    dock_app: &mut AppState,
) {
    anim.t = (anim.t + anim.dt).min(1.0);
    let t = anim.t;
    match anim.mode {
        FsAnimMode::Enter => {
            // FS app fades in from transparent; chrome/dock fade out
            if let Some(app) = apps.get_mut(&anim.app_name) {
                app.anim_alpha = Some(t);
            }
            chrome_app.anim_alpha = Some(1.0 - t);
            dock_app.anim_alpha   = Some(1.0 - t);
        }
        FsAnimMode::Exit => {
            // FS app fades out; chrome/dock fade in
            if let Some(app) = apps.get_mut(&anim.app_name) {
                app.anim_alpha = Some(1.0 - t);
            }
            chrome_app.anim_alpha = Some(t);
            dock_app.anim_alpha   = Some(t);
        }
    }
    if anim.t >= 1.0 {
        // Clear animation overrides
        if let Some(app) = apps.get_mut(&anim.app_name) {
            app.anim_alpha = None;
        }
        chrome_app.anim_alpha = None;
        dock_app.anim_alpha   = None;
    }
}
```

`anim_alpha` is already supported by `blit_clipped` (R26 §13, Step 7):
```rust
blit_surface(... snap.anim_alpha.unwrap_or(snap.global_alpha))
```
No changes needed to `blit_surface` or `blit_clipped` for animation.

---

## 15. WIT Interface `vyoma:fullscreen@1.0.0`

```wit
package vyoma:fullscreen@1.0.0;

enum split-axis { horizontal, vertical }
enum tile-side  { left, right }

interface fullscreen {
    enter-fullscreen:       func(app-id: u32) -> result<_, string>;
    exit-fullscreen:        func(app-id: u32) -> result<_, string>;
    enter-splitview:        func(primary: u32, secondary: u32, axis: split-axis)
                                -> result<_, string>;
    exit-splitview-both:    func(primary: u32) -> result<_, string>;
    exit-splitview-app:     func(app-id: u32)  -> result<_, string>;
    add-to-fullscreen:      func(app-id: u32, target-fs-app: u32) -> result<_, string>;
    set-split-ratio:        func(app-id: u32, ratio: float32) -> result<_, string>;
    tile-window:            func(app-id: u32, side: tile-side) -> result<_, string>;
    untile-window:          func(app-id: u32) -> result<_, string>;
    get-fs-mode:            func(app-id: u32) -> result<string, string>;
}

world fullscreen-world {
    import fullscreen;
}
```

`init_linker_for_app` registers `vyoma:fullscreen` only for apps that declare
`display = true` in vyoma.toml (conditional registration — R18 WIT pattern).

---

## 16. Lifecycle Integration

### 16.1 App Process Exit While in FS/SplitView

When an app in `FullScreenMode::FullScreen` exits:

```rust
// supervisor/src/lifecycle.rs — on_app_process_exited()

pub fn on_app_process_exited(app_name: &str, state: &mut SupervisorState) {
    let fs_mode = state.apps.get(app_name).map(|a| a.fs_mode);
    match fs_mode {
        Some(FullScreenMode::FullScreen) => {
            handle_fullscreen_exit(app_name, state);
        }
        Some(FullScreenMode::SplitView) => {
            // Partner app gets promoted to FullScreen (same as splitview_exit_app)
            handle_splitview_exit_app(app_name, state);
        }
        Some(FullScreenMode::TileWindow) => {
            handle_tile_exit(app_name, &mut state.apps, &state.config);
            // App removed via normal lifecycle path
        }
        _ => { /* Normal exit path */ }
    }
}
```

### 16.2 App Restart Policy

If an app with `restart = "always"` exits while in FS mode, the supervisor:
1. Calls `on_app_process_exited` (cleans up FS state, releases space).
2. Restarts the app normally in its `fs_pre_space`.
3. Does NOT automatically re-enter FS mode — the app starts in Normal mode.

---

## 17. INPUT_LOCK_LEVEL Integration

Full Screen and Split View do NOT add new lock levels. They interact with existing levels:

- While `FsAnimMode::Enter` is active (8-frame fade): `INPUT_LOCK_LEVEL` is set to
  `LockLevel::StageSwitch (3)` to swallow keys during the transition.
- After animation completes: lock cleared to `None (0)`.
- Escape key routing (§5.3) checks `INPUT_LOCK_LEVEL == None` before triggering FS exit.

```rust
// supervisor/src/fs/commands.rs — handle_fullscreen_enter()

// Reuse StageSwitch lock level for animation period
INPUT_LOCK_LEVEL.store(LockLevel::StageSwitch as u8, Ordering::Release);
state.fs_anim = Some(FsEnterAnimation {
    app_name: app_name.to_string(),
    mode: FsAnimMode::Enter,
    t: 0.0,
    dt: 1.0 / FS_ANIM_FRAMES as f32,
});
// Lock is cleared in advance_fs_anim when t >= 1.0
```

---

## 18. Platform Matrix

| Feature | desktop-full | mobile | server-headless | iot-edge | others |
|---------|-------------|--------|----------------|----------|--------|
| Full Screen | yes | default (all apps are FS) | no | no | no |
| Split View | yes | yes (iPad-style) | no | no | no |
| Tile Window | yes | no | no | no | no |
| FS dedicated spaces | yes | no (single space model) | no | no | no |
| Chrome/dock suppression | yes | n/a | n/a | n/a | n/a |
| `VYOMA_FS:` protocol | yes | yes | no | no | no |
| Split ratio IPC | yes | yes | no | no | no |
| FS enter/exit animation | yes | yes | no | no | no |

**Mobile specifics**: All apps are full-screen by default (R21 §11). `FullScreenMode::FullScreen`
is the initial state of every app on mobile. `splitview_enter` maps to iPad-style multi-
tasking: two apps side by side; chrome/dock are replaced by mobile equivalents (deferred to
R28). `TileWindow` is not applicable on mobile (no partial-screen float concept).

**server-headless**: No display capability. All FS/SplitView/Tile commands are rejected
with `VYOMA_FS:error:no_display`.

---

## 19. File Layout

```
supervisor/src/
├── fs/
│   ├── mod.rs          (re-exports; FullScreenMode, SplitRole, SplitAxis, TileSide enums)
│   ├── registry.rs     (FsRegistry, FsPair, allocate_fs_space, release_fs_space)
│   ├── commands.rs     (handle_fullscreen_enter/exit, handle_splitview_enter/exit,
│   │                    handle_split_ratio, handle_tile_window, handle_tile_exit)
│   ├── layout.rs       (compute_split_rects, apply_split_geometry, compute_tile_rect)
│   ├── animation.rs    (FsEnterAnimation, FsAnimMode, advance_fs_anim, FS_ANIM_FRAMES)
│   └── suppression.rs  (FS_CHROME_SUPPRESSED, FS_DOCK_SUPPRESSED AtomicBool globals)
│
├── display.rs          (AppState: fs_mode, fs_space, split_role, split_ratio,
│                        split_axis, fs_pre_*, tile_side, fs_chrome_hidden)
├── compositor.rs       (chrome_clip_px / dock_clip_px computed per tick;
│                        advance_fs_anim called per tick; FS_SUPPRESSED check in blit_clipped)
├── lifecycle.rs        (on_app_process_exited: FS/SplitView/Tile cleanup)
├── main.rs             (Escape key FS exit interception; fullscreen_enter/exit IPC dispatch)
├── ipc_handlers.rs     (splitview_enter/exit, split_ratio, tile_window IPC command routing)
└── wit_handlers.rs     (vyoma:fullscreen@1.0.0 WIT closures)

apps/
(no new apps — FS/SplitView/Tile are pure supervisor-side; VYOMA_FS: delivered as stdin)
```

---

## 20. IPC Command Summary

| Command | Arguments | Effect |
|---------|-----------|--------|
| `fullscreen_enter` | `<app>` | Enter full screen mode |
| `fullscreen_exit` | `<app>` | Exit full screen mode |
| `splitview_enter` | `<app_a> <app_b>` | Enter split view |
| `splitview_exit_both` | — | Both apps exit split view |
| `splitview_exit_app` | `<app>` | Named app exits; partner promotes to FS |
| `splitview_add_to_fullscreen` | `<app_b>` | Add second app to current FS space |
| `split_ratio` | `<app> <ratio>` | Update split ratio (0.10–0.90) |
| `tile_window` | `<app> <left\|right>` | Enter tile window mode |
| `tile_window_exit` | `<app>` | Exit tile window mode |

---

## 21. Open Questions (Deferred)

1. **Green traffic-light button**: Click enters FS mode; hover shows FS/tile options.
   Deferred to R32 (Mouse & Gestures).
2. **Drag-to-resize split handle**: Mouse drag on handle updates `split_ratio` live.
   Deferred to R32.
3. **Dock hover-reveal in FS**: Moving cursor to bottom edge reveals dock temporarily.
   Deferred to R32.
4. **Split View with Stage Manager active**: When both SM and SplitView are used, the
   dedicated FS space has SM disabled but the transition animation should match SM's
   slide-in style. Visual alignment deferred to R30 (Animation System).
5. **`split_ratio` persistence**: Should split ratios survive app restart? Deferred to
   R33 (Settings / Persistence).
6. **Vertical Split View**: The `SplitAxis::Vertical` path is spec'd (§7.2) but no UI
   entry point exists in R27. Deferred to R32 (gesture or keyboard toggle).
7. **Three-app layouts**: macOS does not support 3-way split; VyomaOS does not either
   in R27. Deferred to a future round if demand exists.
