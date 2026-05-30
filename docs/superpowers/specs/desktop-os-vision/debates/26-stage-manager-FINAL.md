# FINAL Spec: Stage Manager (Round 26)

**Subsystem**: Stage Manager  
**macOS Analogue**: Stage Manager (macOS 13+ / iPadOS 16+)  
**Depends on**: R21 (WM spaces, PerSpaceZ), R22 (lifecycle), R24 (dock), R25 (MC, INPUT_LOCK_LEVEL, surface_quiesced, last_flushed_snapshot)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

Stage Manager groups windows into "stages." One stage occupies the main content area;
all other stages for the current space appear as thumbnail cards in a 140pt left strip.
Clicking a strip card swaps the active stage. Stage Manager is per-space, toggleable,
and co-exists with dock and chrome.

---

## 2. Architecture

### 2.1 Split Responsibility

**Supervisor** owns all stage geometry — `StageRegistry`, app positioning, animation
frame computation. A thin **`stage-strip` WASM app** (space=0, z=65532) renders the
left strip and reports selection events. This follows the R25 mission-control pattern.

### 2.2 Space-0 Z-Order (Four Apps)

Pass 2 in `flush_pass`:
```
stage-strip (65532) → mission-control (65533) → dock (65534) → chrome (65535)
```
Space=0 assertion updated:
```rust
assert!(
    app.win_space != 0
        || matches!(app.name.as_str(), "chrome"|"dock"|"mission-control"|"stage-strip"),
    "only chrome/dock/mission-control/stage-strip may use space=0"
);
```

### 2.3 `stage-strip` Launch

`stage-strip` is launched at **supervisor boot** with `win_w=0, win_h=0` (invisible until
Stage Manager is first enabled). This avoids the not-running race in B4. On Stage Manager
enable the strip resizes to `140 × (logical_h - CHROME_H - DOCK_H)`.

```toml
# apps/stage-strip/vyoma.toml
[app]
name = "stage-strip"
version = "1.0.0"
wasm = "stage-strip.wasm"
[capabilities]
display = true
stdio   = true
shell   = true
```

### 2.4 Left-Edge Clip `strip_clip_x_px` (B5 Fix — see §13)

When Stage Manager is enabled, `blit_clipped()` receives `strip_clip_x_px = 140 *
config.scale_factor`. Space-N apps are clipped at their left edge against this value.
Space-0 apps skip left-edge clipping (they own their strip region).

---

## 3. Data Model

```rust
// supervisor/src/stage/registry.rs

pub type StageId = u32;

pub struct Stage {
    pub id: StageId,
    pub members: Vec<String>,   // app names in display order
}

pub struct SpaceStages {
    pub stages: Vec<Stage>,
    pub active: StageId,
    pub enabled: bool,
    pub next_id: StageId,
}

pub struct StageRegistry {
    pub spaces: HashMap<u8, SpaceStages>,
}
```

A stage has exactly one active stage per space. `members` is display order (top-to-bottom
in the strip thumbnail). Empty stages are GC'd after 5s (§7).

---

## 4. Off-Screen Placement + Quiescence (B2 Fix)

### 4.1 `stage_offscreen` Flag on AppState

```rust
// supervisor/src/display.rs — AppState additions

pub stage_offscreen: bool,                    // true = app in inactive stage
pub pending_offscreen_resize: Option<(u32, u32)>,  // resize deferred until activation
```

Inactive-stage apps are hidden via `win_x = OFFSCREEN_X` (0xFFFF). They remain Running
but are semi-quiesced.

### 4.2 Draw Command Gate for Off-Screen Apps

```rust
// supervisor/src/draw_cmd.rs — additions to handle_draw_cmd()

DrawCmd::ResizeSurface { w, h } => {
    if app.stage_offscreen {
        app.pending_offscreen_resize = Some((w, h));
        return;   // block resize; apply when stage activates
    }
    // normal resize handling
}
DrawCmd::Flush => {
    if !app.stage_offscreen {
        app.last_flushed_snapshot = Some(Arc::new(app.surface.clone()));
    }
    // normal flush handling continues (surface updated, not composited)
}
```

### 4.3 Drain on Stage Activation

In `handle_stage_select`, before starting animation for incoming members:
```rust
for name in &incoming_members {
    if let Some(app) = apps.get_mut(name) {
        app.stage_offscreen = false;
        if let Some((w, h)) = app.pending_offscreen_resize.take() {
            app.pending_resize = Some((w, h));  // normal R21 drain path
        }
    }
}
```

---

## 5. Stage Activation and Animation

### 5.1 `handle_stage_select`

```rust
// supervisor/src/stage/activation.rs

pub fn handle_stage_select(
    to_id: StageId,
    apps: &mut HashMap<String, AppState>,
    registry: &mut StageRegistry,
    config: &DisplayConfig,
    space: u8,
) {
    let ss = registry.spaces.get_mut(&space).unwrap();
    let from_id = ss.active;
    if from_id == to_id { return; }

    let from_members: Vec<String> = ss.stages.iter()
        .find(|s| s.id == from_id).map(|s| s.members.clone()).unwrap_or_default();
    let to_members: Vec<String> = ss.stages.iter()
        .find(|s| s.id == to_id).map(|s| s.members.clone()).unwrap_or_default();

    // Pre-position incoming members (drain pending_offscreen_resize)
    let main_rects = compute_main_area_layout(to_members.len(), config);
    for (name, rect) in to_members.iter().zip(main_rects.iter()) {
        if let Some(app) = apps.get_mut(name) {
            app.stage_offscreen = false;
            if let Some((w, h)) = app.pending_offscreen_resize.take() {
                app.pending_resize = Some((w, h));
            }
            app.anim_x_override = Some(config.logical_width as i32 + rect.0 as i32);
            app.anim_alpha = Some(0.0);
        }
    }

    // Set animation state
    ss.active = to_id;
    let anim = StageAnimation {
        from_stage_id: from_id,
        to_stage_id: to_id,
        t: 0.0,
        dt: 1.0 / (ANIM_DURATION_FRAMES as f32),
    };
    // stored in compositor state
    INPUT_LOCK_LEVEL.store(LockLevel::StageSwitch as u8, Ordering::Release);
}
```

### 5.2 Animation Frame (Compositor Tick)

```rust
// supervisor/src/stage/animation.rs — advance_frame()

pub fn advance_frame(anim: &mut StageAnimation, apps: &mut HashMap<String, AppState>,
                     registry: &StageRegistry, config: &DisplayConfig, space: u8) {
    anim.t = (anim.t + anim.dt).min(1.0);
    let t = ease_out_quad(anim.t);

    let ss = registry.spaces.get(&space).unwrap();
    let main_rects = compute_main_area_layout(
        ss.stages.iter().find(|s| s.id == anim.to_stage_id)
            .map(|s| s.members.len()).unwrap_or(0), config
    );

    // Animate outgoing stage: slide left → offscreen, fade out
    if let Some(from_stage) = ss.stages.iter().find(|s| s.id == anim.from_stage_id) {
        for name in &from_stage.members {
            if let Some(app) = apps.get_mut(name) {
                app.anim_x_override = Some((MAIN_X as f32 * (1.0 - t) - config.logical_width as f32 * t) as i32);
                app.anim_alpha = Some(1.0 - t);
            }
        }
    }

    // Animate incoming stage: slide left from right, fade in
    if let Some(to_stage) = ss.stages.iter().find(|s| s.id == anim.to_stage_id) {
        for (name, rect) in to_stage.members.iter().zip(main_rects.iter()) {
            if let Some(app) = apps.get_mut(name) {
                let start_x = config.logical_width as f32 + rect.0 as f32;
                app.anim_x_override = Some((start_x + (rect.0 as f32 - start_x) * t) as i32);
                app.anim_alpha = Some(t);
            }
        }
    }

    if anim.t >= 1.0 {
        finalize_stage_switch(apps, registry, config, space, anim);
        INPUT_LOCK_LEVEL.compare_exchange(
            LockLevel::StageSwitch as u8,
            LockLevel::None as u8,
            Ordering::AcqRel, Ordering::Relaxed,
        ).ok();
    }
}
```

`ANIM_DURATION_FRAMES = 12` (200ms at 60fps). `ease_out_quad(t) = 1 - (1-t)²`.

### 5.3 `finalize_stage_switch` — Explicit Definition (B3 Fix)

```rust
// supervisor/src/stage/animation.rs

pub fn finalize_stage_switch(
    apps: &mut HashMap<String, AppState>,
    registry: &StageRegistry,
    config: &DisplayConfig,
    space: u8,
    anim: &StageAnimation,
) {
    let ss = match registry.spaces.get(&space) {
        Some(s) => s,
        None => return,
    };

    // Step 1: Move outgoing apps off-screen
    // CRITICAL ORDERING: set win_x BEFORE clearing anim_x_override (B3 fix)
    if let Some(from_stage) = ss.stages.iter().find(|s| s.id == anim.from_stage_id) {
        for name in &from_stage.members {
            if let Some(app) = apps.get_mut(name) {
                app.win_x = OFFSCREEN_X;
                app.win_y = OFFSCREEN_Y;
                app.stage_offscreen = true;
                app.anim_x_override = None;  // cleared AFTER win_x is offscreen
                app.anim_alpha = None;
            }
        }
    }

    // Step 2: Snap incoming apps to final main-area positions
    let to_members = ss.stages.iter()
        .find(|s| s.id == anim.to_stage_id)
        .map(|s| s.members.clone())
        .unwrap_or_default();
    let rects = compute_main_area_layout(to_members.len(), config);
    for (name, rect) in to_members.iter().zip(rects.iter()) {
        if let Some(app) = apps.get_mut(name) {
            app.win_x = rect.0;
            app.win_y = rect.1;
            app.stage_offscreen = false;
            app.anim_x_override = None;  // cleared AFTER win_x is correct
            app.anim_alpha = None;
        }
    }
}
```

**Invariant**: `win_x` is always set to its final value before `anim_x_override` is
cleared. This eliminates the one-tick flash of wrong-position windows.

---

## 6. Main Area Layout

```rust
// supervisor/src/stage/layout.rs

const STRIP_W: u32    = 140;
const CHROME_H: u32   = 24;
const DOCK_H: u32     = 48;
const MAIN_X: u32     = STRIP_W + 8;
const MAIN_MARGIN: u32 = 8;

pub fn compute_main_area_layout(n: usize, config: &DisplayConfig) -> Vec<Rect> {
    if n == 0 { return vec![]; }
    let area_w = config.logical_width.saturating_sub(STRIP_W + MAIN_MARGIN * 2);
    let area_h = config.logical_height.saturating_sub(CHROME_H + DOCK_H + MAIN_MARGIN * 2);
    match n {
        1 => vec![Rect { x: MAIN_X + MAIN_MARGIN, y: CHROME_H + MAIN_MARGIN, w: area_w, h: area_h }],
        2 => {
            let half_h = (area_h - MAIN_MARGIN) / 2;
            vec![
                Rect { x: MAIN_X + MAIN_MARGIN, y: CHROME_H + MAIN_MARGIN, w: area_w, h: half_h },
                Rect { x: MAIN_X + MAIN_MARGIN, y: CHROME_H + MAIN_MARGIN + half_h + MAIN_MARGIN, w: area_w, h: half_h },
            ]
        }
        _ => {
            let cols = 2u32;
            let rows = (n as u32 + cols - 1) / cols;
            let cell_w = (area_w - MAIN_MARGIN) / cols;
            let cell_h = (area_h - MAIN_MARGIN * (rows - 1)) / rows;
            (0..n).map(|i| {
                let c = (i as u32) % cols;
                let r = (i as u32) / cols;
                Rect { x: MAIN_X + MAIN_MARGIN + c * (cell_w + MAIN_MARGIN),
                       y: CHROME_H + MAIN_MARGIN + r * (cell_h + MAIN_MARGIN),
                       w: cell_w, h: cell_h }
            }).collect()
        }
    }
}
```

---

## 7. Stage Lifecycle — Orphan GC

- **App launched**: added to active stage (if enabled); otherwise tiling mode unchanged.
- **App exited**: removed from its stage's `members`. If stage becomes empty, start 5s
  orphan timer (`orphan_at: Option<Instant>` on `Stage`). If still empty after 5s, remove.
- **Active stage emptied**: promote next stage in `stages` list to active. If none,
  Stage Manager disables itself for the space.

```rust
pub fn gc_orphan_stages(ss: &mut SpaceStages, now: Instant) {
    ss.stages.retain(|s| {
        if !s.members.is_empty() { return true; }
        s.orphan_at.map_or(true, |t| now.duration_since(t) < Duration::from_secs(5))
    });
}
```

---

## 8. Stage Strip WASM App + StageStripRouter (B4 Fix)

### 8.1 `StageStripRouter` with Lifecycle Callbacks

```rust
// supervisor/src/stage_strip_router.rs

pub struct StageStripRouter {
    queue: VecDeque<String>,
    strip_running: bool,
}

impl StageStripRouter {
    /// On stage-strip Running: synthesize strip_init then flush queued events.
    pub fn on_strip_running(&mut self, registry: &StageRegistry, space: u8,
                             strip_tx: &Sender<String>) {
        self.strip_running = true;
        if let Some(ss) = registry.spaces.get(&space) {
            let msg = format!("VYOMA_STAGE:strip_init:{}\n", build_strip_init_json(ss));
            strip_tx.send(msg).ok();
        }
        while let Some(msg) = self.queue.pop_front() {
            strip_tx.send(msg).ok();
        }
    }

    /// Not running: buffer events; strip_init will be re-synthesized on next on_strip_running.
    pub fn on_strip_not_running(&mut self) { self.strip_running = false; }

    /// Terminated with no restart: clear queue; Stage Manager degrades gracefully.
    pub fn on_strip_terminated_no_restart(&mut self) {
        self.strip_running = false;
        self.queue.clear();
    }

    pub fn route(&mut self, msg: String, strip_tx: &Sender<String>) {
        if self.strip_running {
            strip_tx.send(msg).ok();
        } else {
            if self.queue.len() >= 64 { self.queue.pop_front(); }
            self.queue.push_back(msg);
        }
    }
}
```

`strip_init` is always synthesized from current live `StageRegistry` state — idempotent
on crash/restart. Queue events arriving before Running are replayed after `strip_init`.

### 8.2 VYOMA_STAGE: Protocol

**Supervisor → strip (stdin push)**:
```
VYOMA_STAGE:strip_init:<json>             ← full stage list on strip startup/restart
VYOMA_STAGE:enabled                       ← SM turned on; resize to 140×h
VYOMA_STAGE:disabled                      ← SM turned off; collapse surface
VYOMA_STAGE:stage_add:<id>,<members_csv>
VYOMA_STAGE:stage_remove:<id>
VYOMA_STAGE:stage_members:<id>,<members_csv>
VYOMA_STAGE:active:<id>
VYOMA_STAGE:thumb_begin:<id>,<w>,<h>
VYOMA_STAGE:thumb_data:<base64>
VYOMA_STAGE:thumb_end
```

**Strip → supervisor (stdout)**:
```
@supervisor: stage_select <id>            ← user tapped a strip card
@supervisor: stage_manager_toggle         ← toggle button in strip
```

---

## 9. INPUT_LOCK_LEVEL: `StageSwitch = 3` (B1 Fix)

### 9.1 Enum Extension — No Renumbering

```rust
// supervisor/src/input.rs

pub enum LockLevel {
    None           = 0,
    MissionControl = 1,   // UNCHANGED from R25
    ChromeConsent  = 2,   // UNCHANGED from R25
    StageSwitch    = 3,   // NEW — swallows keys during animation
}
```

All R25 `compare_exchange` call sites remain correct — their numeric values (1, 2) are
unchanged. `StageSwitch(3)` is a swallow-only lock (highest numeric value, lowest routing
priority): keys are discarded during the 200ms animation.

### 9.2 Updated `route_keyboard`

```rust
fn route_keyboard(ev: KeyEvent, focused_app: &str) {
    match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        2 => send_key_to_app("chrome", ev),
        1 => send_key_to_app("mission-control", ev),
        3 => { /* swallow: stage switch animation in progress */ }
        _ => send_key_to_app(focused_app, ev),
    }
}
```

Auto-clear on lifecycle change away from Running for `stage-strip`:
```rust
pub fn on_stage_strip_lifecycle_change(new_state: LifecycleState) {
    if new_state != LifecycleState::Running {
        INPUT_LOCK_LEVEL.compare_exchange(
            LockLevel::StageSwitch as u8,
            LockLevel::None as u8,
            Ordering::AcqRel, Ordering::Relaxed,
        ).ok();
    }
}
```

---

## 10. Toggle

### 10.1 Enable Sequence

```
1. Set SpaceStages.enabled = true
2. Build initial stages from running apps (one stage per app)
3. Set active = stages[0].id
4. Position active stage members in main area
5. Move all other members to OFFSCREEN_X (stage_offscreen = true)
6. stage_strip_router.route("VYOMA_STAGE:enabled\n", strip_tx)
7. Publish strip_init via route (buffered if strip not yet Running)
8. Send thumbnails for strip cards (reuse mc_capture.rs CaptureKind::StageStrip)
```

### 10.2 Disable Sequence

```
1. Set SpaceStages.enabled = false
2. Move all apps to normal tiling layout (recompute via WM tiling, R21)
3. Clear stage_offscreen on all apps
4. stage_strip_router.route("VYOMA_STAGE:disabled\n", strip_tx)
5. strip collapses surface to 0×0
6. strip_clip_x_px = 0 in blit_clipped (no left-edge clip)
```

---

## 11. Spaces Integration

Stages are per-space. Space switch (`PENDING_SPACE_SWITCH`, R21) triggers:
```rust
// supervisor/src/stage/events.rs
pub fn on_space_switched(new_space: u8, registry: &StageRegistry,
                          strip_router: &mut StageStripRouter, strip_tx: &Sender<String>) {
    if let Some(ss) = registry.spaces.get(&new_space) {
        let msg = format!("VYOMA_STAGE:strip_init:{}\n", build_strip_init_json(ss));
        strip_router.route(msg, strip_tx);
    }
}
```
The strip re-renders for the new space's stage list.

---

## 12. Interaction with Dock

- Dock remains visible (z=65534 > stage-strip z=65532).
- Dock icon click triggers app launch → added to active stage.
- `stage_offscreen` apps are not reflected as "focused" in dock (lifecycle Running but not main area).
- Dock `mru_list` (R24) tracks app focus changes; works unchanged.

---

## 13. `blit_clipped` — 7-Step Formula with Horizontal Clip (B5 Fix)

```rust
// supervisor/src/compositor.rs — blit_clipped() complete 7-step formula

fn blit_clipped(fb: &mut FrameBuffer, snap: &AppSnapshot, config: &DisplayConfig,
                chrome_h_px: u32, dock_clip_y_px: u32, strip_clip_x_px: u32)
{
    let win_y_px = snap.win_y_pts.saturating_mul(config.scale_factor as u32);
    let win_x_px = snap.win_x_pts.saturating_mul(config.scale_factor as u32);

    // Step 0 (NEW — R26): left-edge clip for stage strip
    let effective_dest_x = if snap.is_space0 {
        win_x_px   // space-0 apps own strip region; no left-edge clip
    } else {
        win_x_px.max(strip_clip_x_px)
    };
    let src_x_offset = effective_dest_x.saturating_sub(win_x_px);
    let blittable_w = snap.surface_width_px.saturating_sub(src_x_offset);
    let right_overflow = (effective_dest_x + blittable_w).saturating_sub(config.physical_width);
    let blittable_w_clipped = blittable_w.saturating_sub(right_overflow);
    if blittable_w_clipped == 0 { return; }

    // Step 1: top-edge clip
    let effective_dest_y = if snap.is_space0 {
        win_y_px
    } else {
        win_y_px.max(chrome_h_px)
    };
    // Step 2: rows clipped from top of surface
    let src_y_offset = effective_dest_y.saturating_sub(win_y_px);
    // Step 3: height after top clip
    let blittable_h_after_top_clip = snap.surface_height_px.saturating_sub(src_y_offset);
    // Step 4: bottom of blittable region
    let blittable_bottom = (effective_dest_y as i64) + (blittable_h_after_top_clip as i64);
    // Step 5: bottom-edge clip
    let clip_bottom = if snap.is_space0 {
        config.physical_height as i64
    } else {
        dock_clip_y_px as i64
    };
    let clipped_bottom = blittable_bottom.min(clip_bottom);
    // Step 6: final height
    let blittable_h_combined = (clipped_bottom - effective_dest_y as i64).max(0) as u32;
    if blittable_h_combined == 0 { return; }

    // Step 7 (UPDATED): blit with horizontal + vertical offsets
    blit_surface(
        fb, &snap.surface,
        effective_dest_x,
        effective_dest_y,
        src_x_offset,          // NEW
        src_y_offset,
        blittable_w_clipped,   // NEW: replaces snap.surface_width_px
        blittable_h_combined,
        snap.anim_alpha.unwrap_or(snap.global_alpha),
    );
}
```

**`blit_surface` updated signature** (normative for all future rounds):
```rust
fn blit_surface(
    fb: &mut FrameBuffer,
    surface: &[u8],
    dest_x: u32, dest_y: u32,
    src_x_offset: u32, src_y_offset: u32,
    width: u32, height: u32,
    alpha: f32,
)
```
Iterates source rows from `src_y_offset..(src_y_offset + height)`, source columns from
`src_x_offset..(src_x_offset + width)`, with bounds check against `surface_width_px`.
This replaces the R24 7-parameter signature; all callers updated.

**`is_space0` flag**: replaces the name-string comparison `snap.app_name == "chrome"` from R24.
Set on `AppSnapshot` creation: `is_space0 = app.win_space == 0`. Eliminates string compares
in the hot blit path.

---

## 14. Thumbnail Capture for Strip

Reuses R25 `mc_capture.rs` `CaptureRequest` channel. New `CaptureKind` variant:

```rust
pub enum CaptureKind {
    MissionControl { all_spaces: bool },
    StageStrip { stage_id: StageId, members: Vec<String> },
}
```

`StageStrip` capture captures only the `members` list for a given stage (not all apps).
Strip thumbnail size: `124 × 84` logical points (fits in 140pt strip card with padding).
Delivery via `VYOMA_STAGE:thumb_begin/data/end` identical to `VYOMA_MC:` pattern.

---

## 15. Platform Matrix

| Feature | desktop-full | mobile | server-headless | others |
|---------|-------------|--------|----------------|--------|
| Stage Manager | yes | yes (bottom-row strip) | no | no |
| Stage strip | yes (left, 140pt) | yes (bottom, 80pt) | no | no |
| `stage-strip` WASM app | yes | yes | no | no |
| Animation | yes | yes | no | no |

Mobile: strip at bottom 80pt row; strip layout is horizontal. Main area takes full width
minus DOCK_H at bottom. `strip_clip_x_px = 0`, `dock_clip_y_px` adjusted.

---

## 16. File Layout

```
apps/stage-strip/src/
├── main.rs     (event loop: stdin parsing, strip_init, thumb rendering, stage_select)
├── state.rs    (StripState: stages, active_id, thumb map)
└── draw.rs     (render_strip, card layout)

supervisor/src/
├── stage/
│   ├── registry.rs    (StageRegistry, SpaceStages, Stage, StageId)
│   ├── layout.rs      (compute_main_area_layout)
│   ├── animation.rs   (StageAnimation, advance_frame, finalize_stage_switch)
│   ├── activation.rs  (handle_stage_select, drain pending_offscreen_resize)
│   ├── lifecycle.rs   (on_app_launched_stage, on_app_exited_stage, gc_orphan_stages)
│   └── events.rs      (on_space_switched, on_stage_manager_toggle)
├── stage_strip_router.rs  (StageStripRouter: on_strip_running/not_running/terminated)
├── compositor.rs          (blit_clipped 7-step; blit_surface updated signature; anim_alpha)
├── display.rs             (AppState: stage_offscreen, pending_offscreen_resize, anim_x_override, anim_alpha)
├── draw_cmd.rs            (ResizeSurface gate for stage_offscreen; Flush: last_flushed_snapshot gate)
└── input.rs               (LockLevel::StageSwitch=3; route_keyboard updated)
```

---

## 17. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: LockLevel renumbering breaks R25 call sites | `StageSwitch = 3` appended (not inserted); MC=1 and ChromeConsent=2 unchanged; all R25 `compare_exchange` call sites correct; StageSwitch is swallow-only (highest numeric value) |
| B2: Off-screen apps emit `resize_surface` corrupting geometry | `stage_offscreen: bool` + `pending_offscreen_resize: Option<(u32,u32)>` on AppState; `draw_cmd` blocks resize when offscreen; drain deferred resize into `pending_resize` on stage activation |
| B3: `finalize_stage_switch` undefined; one-tick flash | Fully specified: set `win_x` to offscreen/final BEFORE clearing `anim_x_override`; Step 1 moves outgoing off-screen, Step 2 snaps incoming to final rect |
| B4: `StageStripRouter` missing lifecycle callbacks; first-toggle race | Three callbacks defined; `stage-strip` launched at boot with 0×0 surface (eliminates race); `on_strip_running` synthesizes `strip_init` from live registry then flushes queue |
| B5: `blit_clipped` left-edge clip not propagated to `blit_surface` | 7-step formula: Step 0 computes `src_x_offset` + `blittable_w_clipped` + right-overflow; `blit_surface` signature extended with `src_x_offset` + `width` parameters; normative for all future rounds |
