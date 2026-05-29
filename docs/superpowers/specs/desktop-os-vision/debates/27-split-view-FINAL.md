# FINAL Spec: Full Screen & Split View (Round 27)

**Subsystem**: Full Screen & Split View  
**macOS Analogue**: Full Screen / Split View / Tile Window (macOS 13+)  
**Depends on**: R21 (WM spaces, PerSpaceZ, pending_resize), R25 (pending_focus, INPUT_LOCK_LEVEL), R26 (Stage Manager, blit_clipped 7-step)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

Three window expansion modes:
- **FullScreen**: single app occupies entire screen, chrome/dock suppressed
- **SplitView**: two-app pair share a dedicated space side-by-side or stacked
- **TileWindow**: single app occupies one half of the content area; normal WM in the other half; chrome/dock remain visible

All modes use the R21 `pending_resize` drain path for surface allocation. No new resize mechanism needed.

---

## 2. Data Model

```rust
// supervisor/src/fs/registry.rs

#[derive(Clone, Copy, PartialEq)]
pub enum FullScreenMode {
    Normal,
    FullScreen,
    SplitView,
    TileWindow,
}

#[derive(Clone, Copy)]
pub enum TileSide { Left, Right, None }

#[derive(Clone, Copy)]
pub enum SplitDirection { Horizontal, Vertical }

#[derive(Clone, Copy)]
pub enum SplitRole { Primary, Secondary }

pub struct FsEntry {
    pub app_name: String,
    pub fs_space: u8,            // dedicated space allocated for this FS/SplitView
    pub fs_pre_space: u8,        // original space before FS entry
    pub fs_pre_x: u32, pub fs_pre_y: u32, pub fs_pre_w: u32, pub fs_pre_h: u32,
}

pub struct FsPair {
    pub space: u8,
    pub primary: String,
    pub secondary: String,
    pub direction: SplitDirection,
    pub ratio: f32,              // 0.1–0.9, primary share
}

pub struct FsRegistry {
    pub entries: Vec<FsEntry>,
    pub pairs:   Vec<FsPair>,
}
```

AppState additions:
```rust
pub fs_mode:    FullScreenMode,
pub tile_side:  TileSide,
pub split_role: Option<SplitRole>,
pub split_ratio: Option<f32>,
```

---

## 3. Space Allocation — Free List (B2 Fix)

```rust
// supervisor/src/wm/spaces.rs

pub struct SpaceRegistry {
    pub active:    u8,
    pub count:     u8,           // high-water mark (1-9)
    pub free_list: Vec<u8>,      // released spaces available for reuse
}

impl SpaceRegistry {
    pub fn allocate_next(&mut self) -> Option<u8> {
        if let Some(s) = self.free_list.pop() { return Some(s); }
        if self.count >= 9 { return None; }
        self.count += 1;
        Some(self.count)
    }

    pub fn release(&mut self, space: u8) {
        if space == self.count && self.count > 1 {
            self.count -= 1;
            // Also drain free_list tail entries that are now above count
            while self.free_list.last() == Some(&self.count) {
                self.free_list.pop();
                if self.count > 1 { self.count -= 1; }
            }
        } else if space >= 1 && space <= self.count && !self.free_list.contains(&space) {
            self.free_list.push(space);
        }
    }
}
```

`fullscreen_enter` calls `space_reg.allocate_next()` — returns `None` if capped at 9.
`release_fs_space` calls `space_reg.release(space)`.

MC space-strip and `PENDING_SPACE_SWITCH` validation skip spaces in `free_list`.

---

## 4. FS Suppression Flags — Atomic with Space Switch (B1 Fix)

```rust
// supervisor/src/fs/suppression.rs

pub static FS_CHROME_SUPPRESSED: AtomicBool = AtomicBool::new(false);
pub static FS_DOCK_SUPPRESSED:   AtomicBool = AtomicBool::new(false);
```

**Flags are updated inside `vsync_lock.write()` during the space-switch block** —
before `pending_resize` drain and before the blit loop reads them:

```rust
// supervisor/src/compositor.rs — vsync_tick(), space switch block

let pending = PENDING_SPACE_SWITCH.swap(0, Ordering::AcqRel);
if pending != 0 {
    let _write = vsync_lock.write();
    spaces.active = pending;
    fb.fill(BACKGROUND_COLOR);
    mark_new_space_dirty(&mut apps, pending);
    compact_z_order(&mut apps, pending);

    // Compute FS suppression atomically with space switch (B1 fix)
    let is_fs_space = fs_registry.entries.iter().any(|e| e.fs_space == pending)
        || fs_registry.pairs.iter().any(|p| p.space == pending);
    FS_CHROME_SUPPRESSED.store(is_fs_space, Ordering::Release);
    FS_DOCK_SUPPRESSED.store(is_fs_space, Ordering::Release);
    // write-lock drops here
}
```

`chrome_clip_px` and `dock_clip_px` are computed under `vsync_lock.read()` after the
write-lock drops — reading freshly updated atomics:

```rust
let _read = vsync_lock.read();
let chrome_clip_px = if FS_CHROME_SUPPRESSED.load(Ordering::Acquire) { 0 }
                      else { CHROME_H_PTS * config.scale_factor as u32 };
let dock_clip_px   = if FS_DOCK_SUPPRESSED.load(Ordering::Acquire) {
                         config.physical_height
                      } else {
                         config.physical_height - DOCK_H_PTS * config.scale_factor as u32
                      };
// blit loop follows
```

This eliminates the one-frame chrome/dock visibility glitch on FS space switches.

---

## 5. LockLevel Extension — `FsTransition = 4` (B3 Fix)

```rust
// supervisor/src/input.rs

pub enum LockLevel {
    None           = 0,
    MissionControl = 1,   // R25 — unchanged
    ChromeConsent  = 2,   // R25 — unchanged
    StageSwitch    = 3,   // R26 — stage animation swallow
    FsTransition   = 4,   // R27 — full-screen enter/exit swallow
}
```

No existing R25/R26 `compare_exchange` call sites are affected. `FsTransition(4)` does
not renumber any existing constant.

```rust
fn route_keyboard(ev: KeyEvent, focused_app: &str) {
    match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        2 => send_key_to_app("chrome", ev),
        1 => send_key_to_app("mission-control", ev),
        3 | 4 => { /* swallow: animation in progress */ }
        _ => send_key_to_app(focused_app, ev),
    }
}
```

Auto-clear on app process exit (crash during FS transition):
```rust
INPUT_LOCK_LEVEL.compare_exchange(
    LockLevel::FsTransition as u8,
    LockLevel::None as u8,
    Ordering::AcqRel, Ordering::Relaxed,
).ok();
```

---

## 6. Full Screen

### 6.1 Enter Sequence

```
IPC: @supervisor: fullscreen_enter

1. space_reg.allocate_next() → fs_space (or error VYOMA_FS:error:no_space_available)
2. Save restore point on AppState:
     fs_pre_space = app.win_space; fs_pre_x/y/w/h = current geometry
3. app.win_space = fs_space
4. app.pending_resize = Some((logical_w, logical_h))
5. PENDING_SPACE_SWITCH.store(fs_space, Ordering::Release)
6. INPUT_LOCK_LEVEL.store(LockLevel::FsTransition as u8, Ordering::Release)
7. Start FsEnterAnimation (t=0, dt=1/12, fade chrome/dock out)
8. app.fs_mode = FullScreen
9. fs_registry.entries.push(FsEntry { app_name, fs_space, fs_pre_space, ... })
10. Send VYOMA_FS:entering:<logical_w>,<logical_h>
```

### 6.2 Exit Sequence

```
IPC: @supervisor: fullscreen_exit  (or Escape key when focused app has fs_mode = FullScreen)

1. Restore geometry:
     app.win_x = app.fs_pre_x; app.win_y = app.fs_pre_y;
     app.pending_resize = Some((app.fs_pre_w, app.fs_pre_h))
     app.win_space = app.fs_pre_space
     app.fs_mode = Normal
2. compact_z_order for fs_pre_space (app returns to its old space's z-order)
3. PENDING_SPACE_SWITCH.store(fs_pre_space, Ordering::Release)
4. state.pending_focus = Some(app.name.clone())   (R25 deferred focus)
5. fs_registry.entries.retain(|e| e.app_name != app.name)
6. space_reg.release(fs_space)
7. INPUT_LOCK_LEVEL.store(LockLevel::FsTransition as u8, Ordering::Release)
8. Start FsExitAnimation (fade chrome/dock back in)
9. Send VYOMA_FS:exiting
```

---

## 7. Split View

### 7.1 Enter

```
IPC: @supervisor: splitview_enter <other_app>

1. space_reg.allocate_next() → sv_space
2. Save restore points for both apps
3. Assign both apps win_space = sv_space
4. direction: Horizontal (side-by-side) unless logical_h > logical_w (Vertical)
5. ratio = 0.5 (equal split)
6. Compute rects via compute_split_rects()
7. pending_resize for both apps
8. PENDING_SPACE_SWITCH → sv_space
9. fs_registry.pairs.push(FsPair { space: sv_space, primary, secondary, direction, ratio })
10. app.fs_mode = SplitView for both
11. Send VYOMA_FS:splitview_enter:<role>,<w>,<h> to both apps
```

### 7.2 Split Layout

```rust
// supervisor/src/fs/layout.rs

pub fn compute_split_rects(
    pair: &FsPair,
    config: &DisplayConfig,
) -> (Rect, Rect) {
    let content_y = CHROME_H_PTS;
    let content_h = config.logical_height.saturating_sub(CHROME_H_PTS + DOCK_H_PTS);
    // SplitView has its own dedicated space — no chrome/dock shown → full logical_h
    // But spec reserves CHROME_H/DOCK_H for potential future modes; keep consistent
    let full_w = config.logical_width;
    match pair.direction {
        SplitDirection::Horizontal => {
            let left_w = (full_w as f32 * pair.ratio) as u32;
            let right_w = full_w.saturating_sub(left_w);
            (Rect { x: 0,      y: 0, w: left_w,  h: config.logical_height },
             Rect { x: left_w, y: 0, w: right_w, h: config.logical_height })
        }
        SplitDirection::Vertical => {
            let top_h = (config.logical_height as f32 * pair.ratio) as u32;
            let bot_h = config.logical_height.saturating_sub(top_h);
            (Rect { x: 0, y: 0,     w: full_w, h: top_h },
             Rect { x: 0, y: top_h, w: full_w, h: bot_h })
        }
    }
}
```

### 7.3 Split Ratio Adjustment

```
IPC: @supervisor: split_ratio <0.1–0.9>

1. Clamp ratio to [0.1, 0.9]
2. pair.ratio = ratio
3. Recompute rects
4. pending_resize for both apps
5. Send VYOMA_FS:split_ratio_changed:<w>,<h> to both apps
```

R32 hook: `split_handle_dragging: bool` in SupervisorState; `handle_split_ratio` callable
from mouse event path with continuous ratio updates.

### 7.4 Exit One App from SplitView (B4 Fix)

```
IPC: @supervisor: splitview_exit_app  (sent by the exiting app)

Step 1: Restore exiting app geometry
  app.win_x = app.fs_pre_x; app.win_y = app.fs_pre_y
  app.pending_resize = Some((app.fs_pre_w, app.fs_pre_h))
  app.win_space = app.fs_pre_space
  app.fs_mode = Normal; app.split_role = None; app.split_ratio = None

Step 1a: Re-assign z-order in restored space
  app.win_z.set(app.fs_pre_space, 0)     // will be compacted
  wm::zorder::compact_z_order(&mut apps, app.fs_pre_space)

Step 1b: Issue deferred space switch + focus (R25 pattern)
  PENDING_SPACE_SWITCH.store(app.fs_pre_space, Ordering::Release)
  state.pending_focus = Some(app.name.clone())

Step 1c: Re-integrate into stage (AFTER compact, BEFORE space switch applies)
  if let Some(ss) = stage_reg.spaces.get_mut(&app.fs_pre_space) {
      if ss.enabled {
          if let Some(stage) = ss.stages.iter_mut().find(|s| s.id == ss.active) {
              stage.members.push(app.name.clone());
          }
      }
  }

Step 2: Promote remaining app to FullScreen
  Reuse fullscreen_enter sequence for remaining_app (same fs_space)
  remaining_app.pending_resize = Some((logical_w, logical_h))

Step 3: Update FsPair
  fs_registry.pairs.retain(|p| p.space != fs_space)
  // new FsEntry added for remaining_app in fullscreen_enter above

Step 4: Notifications
  Send VYOMA_FS:splitview_exited to exiting app
  Send VYOMA_FS:entering:<logical_w>,<logical_h> to remaining app
```

---

## 8. Tile Window

### 8.1 Enter

```
IPC: @supervisor: tile_left  OR  @supervisor: tile_right

1. app.tile_side = TileSide::Left or Right
2. sm_enabled = stage_reg.spaces.get(&app.win_space).map_or(false, |ss| ss.enabled)
3. rect = compute_tile_rect(tile_side, config, sm_enabled)
4. app.win_x = rect.x; app.win_y = rect.y
5. app.pending_resize = Some((rect.w, rect.h))
6. app.win_manual_layout = true   (R21: disables auto-tiling for this space)
7. app.fs_mode = TileWindow
8. Send VYOMA_FS:tiled:<side>,<x>,<y>,<w>,<h>
```

No dedicated space — tile stays in current space. Chrome and dock remain visible.

### 8.2 `compute_tile_rect` with SM Awareness (B5 Fix)

```rust
// supervisor/src/fs/layout.rs

pub fn compute_tile_rect(side: TileSide, config: &DisplayConfig, sm_enabled: bool) -> Rect {
    let content_y = CHROME_H_PTS;
    let content_h = config.logical_height.saturating_sub(CHROME_H_PTS + DOCK_H_PTS);
    let strip_offset: u32 = if sm_enabled { STRIP_W + MAIN_MARGIN } else { 0 };
    let usable_w = config.logical_width.saturating_sub(strip_offset);
    let half_w = usable_w / 2;
    match side {
        TileSide::Left  => Rect { x: strip_offset,          y: content_y, w: half_w, h: content_h },
        TileSide::Right => Rect { x: strip_offset + half_w, y: content_y,
                                   w: usable_w - half_w,    h: content_h },
        TileSide::None  => unreachable!(),
    }
}
```

Surface dimensions sent via `pending_resize` match the `Rect.w × Rect.h` exactly —
app allocates the right size, blit position is correct.

### 8.3 SM Toggle Hook — Recompute Tiled Apps (B5 Fix)

```rust
// supervisor/src/stage/events.rs — on_stage_manager_toggled()

pub fn on_stage_manager_toggled(
    enabled: bool,
    apps: &mut HashMap<String, AppState>,
    config: &DisplayConfig,
    space: u8,
) {
    for app in apps.values_mut()
        .filter(|a| a.win_space == space && a.fs_mode == FullScreenMode::TileWindow)
    {
        let new_rect = compute_tile_rect(app.tile_side, config, enabled);
        app.win_x = new_rect.x;
        app.win_y = new_rect.y;
        app.pending_resize = Some((new_rect.w, new_rect.h));
        send_to_app_stdin(&app.name, &format!(
            "VYOMA_FS:tile_geometry_changed:{},{},{},{}\n",
            new_rect.x, new_rect.y, new_rect.w, new_rect.h,
        ));
    }
}
```

Called at the end of SM enable and disable sequences (R26 §10.1 and §10.2).

---

## 9. FS Animation

Enter/exit animations use `anim_alpha` on chrome/dock AppSnapshot. Duration: 12 frames
(~200ms at 60fps). `LockLevel::FsTransition = 4` swallows keys during transition.
Auto-cleared when animation completes or on app lifecycle change away from Running.

```rust
pub struct FsAnimation {
    pub kind: FsAnimKind,   // Enter or Exit
    pub t: f32,
    pub dt: f32,            // 1.0 / 12.0
}

pub enum FsAnimKind { Enter, Exit }

pub fn advance_fs_anim(anim: &mut FsAnimation) -> bool {
    anim.t = (anim.t + anim.dt).min(1.0);
    let done = anim.t >= 1.0;
    if done {
        INPUT_LOCK_LEVEL.compare_exchange(
            LockLevel::FsTransition as u8,
            LockLevel::None as u8,
            Ordering::AcqRel, Ordering::Relaxed,
        ).ok();
    }
    done
}
```

---

## 10. VYOMA_FS: Protocol

### 10.1 Supervisor → App (stdin push)

```
VYOMA_FS:entering:<logical_w>,<logical_h>           ← FS mode active
VYOMA_FS:exiting                                     ← about to restore normal mode
VYOMA_FS:splitview_enter:<role>,<w>,<h>             ← SplitView; role = primary|secondary
VYOMA_FS:splitview_exited                            ← this app exited SplitView
VYOMA_FS:split_ratio_changed:<w>,<h>                ← resize event
VYOMA_FS:tiled:<side>,<x>,<y>,<w>,<h>              ← Tile mode; side = left|right
VYOMA_FS:tile_geometry_changed:<x>,<y>,<w>,<h>     ← SM toggle recomputed tile
VYOMA_FS:error:no_space_available                   ← FS enter refused (cap)
```

### 10.2 App → Supervisor (stdout IPC)

```
@supervisor: fullscreen_enter
@supervisor: fullscreen_exit
@supervisor: splitview_enter <other_app>
@supervisor: splitview_exit_app
@supervisor: split_ratio <0.1–0.9>
@supervisor: tile_left
@supervisor: tile_right
@supervisor: tile_exit
```

---

## 11. Interaction with Stage Manager

- FS/SplitView entry: app removed from its current stage. SM disabled for the FS dedicated space.
- FS/SplitView exit: app re-integrated into `fs_pre_space`'s active stage (Step 1c above).
- TileWindow: stays in current space; SM active. `compute_tile_rect` accounts for strip width.
- SM toggle: `on_stage_manager_toggled` recomputes tile geometry for all tiled apps.

---

## 12. Interaction with Mission Control

FS apps appear as distinct space cards in the MC space strip. `VYOMA_MC:space_type:<n>,<type>`
push line (type = `normal | fullscreen | splitview`) allows MC to badge FS space cards.
FS space strip cards show the app name + FS icon.

SplitView initiation from MC: MC sends `@supervisor: splitview_add_to_fullscreen <app>`.

---

## 13. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Full Screen | yes | default (always FS) | no | no |
| Split View | yes | yes | no | no |
| Tile Window | yes | no | no | no |
| FS animation | yes | yes | no | no |
| Dedicated FS space | yes | no (only 1 space on mobile) | no | no |

Mobile: full screen is the default window mode. Split View is side-by-side 50/50.
No Tile Window (mobile has no partial-screen concept). Chrome and dock heights differ.

---

## 14. File Layout

```
supervisor/src/
├── fs/
│   ├── mod.rs          (re-exports)
│   ├── registry.rs     (FsRegistry, FsEntry, FsPair, FsAnimation, FullScreenMode)
│   ├── commands.rs     (fullscreen_enter/exit, splitview_enter/exit, tile_left/right/exit)
│   ├── layout.rs       (compute_split_rects, compute_tile_rect — sm_enabled param)
│   ├── animation.rs    (FsAnimation, advance_fs_anim, LockLevel::FsTransition=4)
│   └── suppression.rs  (FS_CHROME_SUPPRESSED, FS_DOCK_SUPPRESSED atomics)
├── wm/
│   └── spaces.rs       (SpaceRegistry: allocate_next, release — free_list)
├── compositor.rs       (space switch block: suppression flags set atomically)
└── input.rs            (LockLevel: FsTransition=4; route_keyboard: 3|4 swallow)
```

---

## 15. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Suppression flag race with space switch | Flags updated inside `vsync_lock.write()` space-switch block; `chrome_clip_px`/`dock_clip_px` read under `vsync_lock.read()` after write-lock drops — no stale one-frame blit |
| B2: Space free-list leak | `free_list: Vec<u8>` on `SpaceRegistry`; `allocate_next()` pops free-list before incrementing high-water mark; `release()` pushes non-tail releases to free-list; MC and PENDING_SPACE_SWITCH skip free-list entries |
| B3: FsTransition reuses LockLevel::StageSwitch | `LockLevel::FsTransition = 4`; `route_keyboard` swallows on `3 \| 4`; no existing R25/R26 call sites affected |
| B4: splitview_exit_app skips z-order and space switch for exiting app | Steps 1a/1b/1c: compact z-order in `fs_pre_space`, issue `PENDING_SPACE_SWITCH` + `pending_focus` (R25 pattern), then stage re-integration — focus applied after space switch commit |
| B5: `compute_tile_rect` ignores SM strip; SM toggle doesn't recompute | `compute_tile_rect` takes `sm_enabled: bool`; `strip_offset` applied to x and w; `on_stage_manager_toggled` hook recomputes all tiled apps and sends `VYOMA_FS:tile_geometry_changed` |
