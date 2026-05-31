# Spec: Stage Manager (Round 26)

**Subsystem**: Stage Manager
**macOS Analogue**: Stage Manager (macOS 13 Ventura / iPadOS 16+)
**Depends on**: R21 (WM spaces, PerSpaceZ, PENDING_SPACE_SWITCH), R22 (lifecycle, surface_quiesced, last_flushed_snapshot), R23 (chrome, INPUT_LOCK_LEVEL), R24 (dock, DockRouter, Model A compositor), R25 (mission-control, mc_capture, draw_thumb, pending_focus)
**Status**: DRAFT — Round 26

---

## 1. What Stage Manager Is

Stage Manager is a window-grouping and focus mode that replaces the default tiling/floating
WM layout for users who want an iPad-style task switching experience on the desktop.

In Stage Manager mode:

- One "stage" — a set of one or more related app windows — occupies the main content
  area (center of screen). Only windows belonging to the active stage are visible at full
  resolution in the main area.
- Inactive stages appear as compact preview thumbnails stacked vertically in a 140pt-wide
  left-side strip. Up to five inactive stages are shown simultaneously; the strip scrolls
  if more exist.
- Switching to an inactive stage instantly moves the current stage to the strip and brings
  the selected stage to the main area with a horizontal slide animation (~200ms).
- The Dock remains visible at the bottom. Menu-bar chrome remains at the top. Stage Manager
  does not touch space-0 apps.
- Stage Manager is opt-in per space and can be toggled off, reverting to normal tiling.

The UX goal is a focused, clutter-free desktop: only one app (or small group) demands
attention at a time, but switching between task contexts is a single click.

---

## 2. Architecture Choice

### 2.1 Three Options Considered

**Option A — Supervisor-side compositing mode**: The supervisor directly manages stage
layout as a WM mode flag. Stage positioning is computed in `tiling.rs` / `wm/`, window
geometry set on `AppState`. No new WASM app.

**Option B — Dedicated space-0 WASM overlay**: A privileged `stage-manager` WASM app in
space=0 renders the left strip, handles animation, and issues `VYOMA_WM:move` commands
to reposition regular-space apps.

**Option C — Integrated into WM with thin WASM strip renderer**: The supervisor handles
all layout geometry (stage data model, active/inactive slot positions) and animation
scheduling. The left strip is drawn by a thin space-0 WASM app (`stage-strip`) whose sole
job is rendering thumbnails and reporting selection events. The main-area app windows are
regular space-N windows repositioned by the supervisor.

### 2.2 Decision: Option C — Integrated WM with thin strip renderer

**Rationale**:

Option A (pure supervisor) requires the supervisor to render the left strip directly into
the framebuffer. This is fine for simple fills but requires thumbnail pixel data, label
rendering, and hover-state logic inside the supervisor binary. That violates the design
philosophy of keeping the supervisor policy-free for UI chrome.

Option B (full space-0 WASM overlay) puts too much authority in a WASM app: issuing
`VYOMA_WM:move` commands for all non-strip windows on every stage switch would serialize
a large number of WM operations through app stdout, causing visible jank on multi-window
stages. It also re-introduces the surface-allocation race from R21 B1 if the WASM app
drives resizes for many windows simultaneously.

Option C is the correct split:
- **Supervisor owns all geometry**: stage data model, active stage layout, inactive strip
  slot assignment, animation interpolation, and `pending_resize` drainage are all in the
  supervisor. This keeps window positioning atomic with the compositor tick.
- **`stage-strip` WASM app owns rendering**: it receives thumbnail data and draws the
  left strip surface. It has no ability to move windows — it only reports selection events
  via `@supervisor: stage_select <stage_id>`.
- This mirrors the R25 pattern: mission-control owns its rendering, supervisor owns window
  geometry. No new design surface is introduced.

### 2.3 `stage-strip` as space=0 App

```toml
# apps/stage-strip/vyoma.toml
[app]
name = "stage-strip"
version = "1.0.0"
wasm = "stage-strip.wasm"

[capabilities]
display = true
stdio   = true
shell   = true   # @supervisor: stage_select <stage_id>
# NO filesystem, NO network
```

`win_space = 0`, `win_x = 0`, `win_y = chrome_h` (logical pts, default 24),
`win_w = 140`, `win_h = logical_h - chrome_h - dock_h` (default 528 at 960×600).
Z-index: **65532** — below mission-control (65533), dock (65534), chrome (65535).

**Space-0 assertion update** (extends R25):
```rust
assert!(
    app.win_space != 0
        || matches!(app.name.as_str(),
            "chrome" | "dock" | "mission-control" | "stage-strip"),
    "only chrome/dock/mission-control/stage-strip may use space=0"
);
```

### 2.4 Compositor Model A — Four space-0 Apps

Pass 2 in `flush_pass` now has four space-0 apps sorted by z ascending:
```
stage-strip (65532) → mission-control (65533) → dock (65534) → chrome (65535)
```

The `stage-strip` surface clips to `[0, chrome_h_px]` on the left side. Regular app
windows in the main area are shifted right by 140 logical points when Stage Manager is
active (see section 4.3).

---

## 3. Stage Data Model

### 3.1 What is a Stage?

A Stage is a named set of app windows that share a logical task context. At any given
moment, exactly one stage per space is "active" (main area); all others are "inactive"
(left strip). Stages are per-space: space 1 has its own stage list, space 2 has its own,
etc.

```rust
// supervisor/src/stage/mod.rs

/// Unique identifier for a stage within a space.
pub type StageId = u32;

pub struct Stage {
    pub id:      StageId,
    pub space:   u8,
    pub members: Vec<String>,   // app names, insertion-ordered; first = primary window
    pub label:   String,        // display name = primary app's win_title (auto)
    pub thumb_dirty: bool,      // true when any member surface changed since last strip render
}

pub struct StageRegistry {
    /// Per-space stage lists.
    pub spaces: HashMap<u8, SpaceStages>,
}

pub struct SpaceStages {
    pub stages:  Vec<Stage>,     // all stages for this space, in strip order (index 0 = top)
    pub active:  StageId,        // id of the currently active (main-area) stage
    pub next_id: StageId,        // monotonically increasing ID counter
    pub enabled: bool,           // Stage Manager on/off for this space
}
```

`StageRegistry` is owned by `SupervisorState` and protected by the same `apps` mutex
(no additional lock introduced; all stage mutations happen on the event-loop thread).

### 3.2 Stage Creation

Stages are created in three ways:

1. **Auto-create on app launch (default)**: When a new app is launched into a
   Stage-Manager-enabled space and no explicit stage was specified, the supervisor creates
   a new single-member stage for that app and places it in the strip. The newly launched
   app does NOT displace the active stage unless the user explicitly switches.

2. **Launch into active stage**: The user may hold a modifier key when clicking a Dock
   icon to add the app to the current active stage. Implemented in R32 (mouse+gestures);
   for now, the IPC command `@supervisor: stage_add_to_active <app_name>` is the
   programmatic path.

3. **IPC explicit creation**: `@supervisor: stage_new <app_name>` creates a new stage
   for the named app (moves it out of its current stage).

### 3.3 Stage Membership Rules

- Each non-space-0 app belongs to exactly one stage in its space.
- Moving an app to a different stage is via `@supervisor: stage_move <app_name> <stage_id>`.
- If all members of a stage exit, the stage is an "orphan" — retained for 5 seconds then
  automatically garbage-collected (see section 7.2).
- Space-0 apps (chrome, dock, mission-control, stage-strip) are never members of any stage.

### 3.4 Active Stage Determination

On startup (Stage Manager first enabled for a space), the supervisor creates one stage
per currently running app in that space, then makes the most recently focused app's stage
the active stage.

When Stage Manager is toggled on mid-session, the active stage is the stage containing
the currently focused app.

```rust
// supervisor/src/stage/registry.rs

impl SpaceStages {
    /// Returns the active Stage (panics if stages list is empty — invariant: always ≥1).
    pub fn active_stage(&self) -> &Stage {
        self.stages.iter().find(|s| s.id == self.active).expect("active stage missing")
    }

    pub fn active_stage_mut(&mut self) -> &mut Stage {
        self.stages.iter_mut().find(|s| s.id == self.active).expect("active stage missing")
    }

    /// Returns all inactive stages (strip stages), in strip order.
    pub fn inactive_stages(&self) -> impl Iterator<Item = &Stage> {
        self.stages.iter().filter(|s| s.id != self.active)
    }
}
```

---

## 4. Left Strip Layout

### 4.1 Dimensions

```
strip_x:  0
strip_y:  chrome_h   (24pt default)
strip_w:  140pt
strip_h:  logical_h - chrome_h - dock_h   (528pt at 960×600 with 24pt chrome, 48pt dock)
```

The main content area is pushed right:
```
main_x:   140pt     (when Stage Manager enabled)
main_y:   chrome_h  (24pt)
main_w:   logical_w - 140    (820pt at 960 wide)
main_h:   logical_h - chrome_h - dock_h  (528pt)
```

When Stage Manager is disabled, `main_x = 0`, `main_w = logical_w` (normal tiling area).

### 4.2 Thumbnail Card Layout

Each inactive stage is represented by a card in the strip. Cards are stacked from top to
bottom. Maximum 5 cards visible at once; strip scrolls (deferred to R32 for actual scroll
gesture; for now, only the first 5 inactive stages are shown).

```
card_w:   124pt   (140 - 8pt margin on each side)
card_h:   84pt
card_gap: 12pt
card_x:   8pt (left margin)
card_y_n: chrome_h + 12 + n * (card_h + card_gap)    (n = 0-based strip index)
```

Each card contains:
- A thumbnail preview of the primary window of the stage (scaled to fit `124×68pt`)
- A label below the thumbnail: primary app's `win_title`, truncated to 14 chars, `s`
  font (4×8 pts scaled), centered

Cards are rendered by `stage-strip` using `VYOMA_DRAW:draw_thumb` (R25 protocol) for the
thumbnail and `VYOMA_DRAW:draw_text` for the label.

### 4.3 Main Area App Positioning

When Stage Manager is active for a space, the auto-tiling layout is replaced by stage
layout. The main-area apps (members of the active stage) are tiled within the main content
area rectangle `(140, chrome_h, logical_w - 140, logical_h - chrome_h - dock_h)`.

```rust
// supervisor/src/stage/layout.rs

pub fn compute_main_area_layout(
    n_members: usize,
    main_x: u32, main_y: u32, main_w: u32, main_h: u32,
) -> Vec<(u32, u32, u32, u32)> {
    // Re-uses tiling.rs compute_tiling semantics, but origin-shifted
    let rects = crate::wm::tiling::compute_tiling(n_members, main_w, main_h);
    rects.into_iter().map(|(x, y, w, h)| (x + main_x, y + main_y, w, h)).collect()
}
```

Inactive-stage apps are placed **off-screen** while their stage is inactive:

```rust
/// Off-screen position for inactive stage apps (outside framebuffer bounds).
pub const OFFSCREEN_X: u32 = 0xFFFF;
pub const OFFSCREEN_Y: u32 = 0xFFFF;
```

Setting `win_x = OFFSCREEN_X`, `win_y = OFFSCREEN_Y` causes `blit_clipped` to produce
zero blittable rows (clip formula eliminates them). Their surfaces remain allocated and
continue to receive draw commands; they are just not composited. This avoids killing and
respawning apps on stage switch.

**Important**: inactive-stage apps still have `win_visible = true`. They are hidden via
off-screen placement, not via visibility flag. This preserves their lifecycle state
(`Running`/`Suspended`) for accurate dock indicators.

### 4.4 Strip Surface Startup Fallback

`stage_strip_surface_ready: bool` flag (false at boot). When false, supervisor fills the
left 140px column from `chrome_h_px` to `dock_clip_y_px` with `STAGE_STRIP_BG_COLOR =
0x242434FF`. Mirrors dock/chrome startup pattern from R23/R24.

---

## 5. Stage Activation (Switching)

### 5.1 Selection Event

When the user clicks a strip thumbnail, `stage-strip` WASM app sends:
```
@supervisor: stage_select <stage_id>
```

This triggers the stage-switch sequence in the supervisor event loop.

### 5.2 Stage-Switch Sequence

```rust
// supervisor/src/stage/activation.rs

pub fn handle_stage_select(
    stage_id: StageId,
    state: &mut SupervisorState,
) {
    let space = state.spaces.active;
    let space_stages = match state.stage_registry.spaces.get_mut(&space) {
        Some(s) => s,
        None => return,
    };

    if stage_id == space_stages.active { return; }   // already active

    let prev_active_id = space_stages.active;

    // 1. Record animation parameters
    state.stage_anim = Some(StageAnimation {
        from_stage_id: prev_active_id,
        to_stage_id:   stage_id,
        start_time:    Instant::now(),
        duration_ms:   200,
        phase:         AnimPhase::SlideOut,
    });
    INPUT_LOCK_LEVEL.store(LockLevel::StageSwitch as u8, Ordering::Release);

    // 2. Move previous active stage members off-screen (will be animated by compositor)
    //    Full geometry snap happens in compositor tick after animation completes.
    //    Pre-position new active stage members in main area (off-screen initially).
    let new_members = space_stages.stages.iter()
        .find(|s| s.id == stage_id)
        .map(|s| s.members.clone())
        .unwrap_or_default();
    let main_rects = compute_main_area_layout(
        new_members.len(),
        MAIN_X, MAIN_Y, state.config.logical_width - 140,
        state.config.logical_height - CHROME_H - DOCK_H,
    );
    for (name, rect) in new_members.iter().zip(main_rects.iter()) {
        if let Some(app) = state.apps.get_mut(name) {
            app.pending_resize = Some((rect.2, rect.3));
        }
    }

    // 3. Update active stage id
    space_stages.active = stage_id;

    // 4. Focus primary member of new active stage
    if let Some(primary) = new_members.first() {
        state.pending_focus = Some(primary.clone());
    }

    // 5. Notify stage-strip to redraw
    state.stage_strip_router.route(
        format!("VYOMA_STAGE:stage_activated:{}\n", stage_id),
        &state.stage_strip_tx,
    );
}
```

### 5.3 Atomic Compositor Application

The compositor tick applies stage geometry changes atomically (same pattern as R21 space
switch + R25 pending_focus):

```rust
// supervisor/src/compositor.rs — vsync_tick(), after PENDING_SPACE_SWITCH block

// Apply pending stage switch geometry
if let Some(ref anim) = state.stage_anim {
    let elapsed_ms = anim.start_time.elapsed().as_millis() as u32;
    let t = (elapsed_ms as f32 / anim.duration_ms as f32).min(1.0);

    apply_stage_animation(fb, &mut state.apps, anim, t, &state.config, &state.stage_registry);

    if t >= 1.0 {
        // Animation complete: finalize geometry, clear anim state, release input lock
        finalize_stage_switch(&mut state.apps, &mut state.stage_registry,
                              &state.config, state.spaces.active);
        state.stage_anim = None;
        INPUT_LOCK_LEVEL.compare_exchange(
            LockLevel::StageSwitch as u8,
            LockLevel::None as u8,
            Ordering::AcqRel, Ordering::Relaxed,
        ).ok();
    }
}
```

---

## 6. Window Grouping Rules

### 6.1 Default: One App per Stage

At Stage Manager enable time, each running app in the space gets its own single-member
stage. This is the simplest and safest default — no implicit grouping decisions are made.

### 6.2 Explicit Grouping

Apps can be grouped into a stage via:

| IPC Command | Effect |
|-------------|--------|
| `@supervisor: stage_add_to_active <app>` | Adds `<app>` to active stage; removes from its old stage (may orphan it) |
| `@supervisor: stage_move <app> <stage_id>` | Moves `<app>` to `<stage_id>`; removes from old stage |
| `@supervisor: stage_new <app>` | Creates new single-member stage for `<app>`, moves it out of current stage |
| `@supervisor: stage_remove <app>` | Removes `<app>` from its stage (creates new single-member stage for it) |
| `@supervisor: stage_merge <id1> <id2>` | Merges stage `id2` into stage `id1`; `id2` is dissolved |

### 6.3 Automatic Grouping Heuristics

Automatic grouping is **conservative and opt-in only**. The following heuristic is applied
only when Stage Manager is enabled and a new app is launched:

**Launcher-origin rule**: If app B is launched by app A via IPC (`@supervisor: launch <B>`
from app A's stdout), and A is in the active stage, then B is added to A's stage
automatically. This covers "open in new window" workflows (e.g., a text editor spawning
a diff viewer).

No other automatic grouping rules are applied in R26. IPC-launch parenthood is the only
signal. Heuristics based on window title similarity, desktop position, or launch timing
are deferred to a later round.

### 6.4 Dragging (Deferred to R32)

Drag-to-group (dragging a strip thumbnail onto the main area to merge stages) and
drag-to-ungroup (dragging a window out of the main area into the strip as a new stage)
are deferred to R32 (Mouse & Gestures). The IPC commands in 6.2 serve as the programmatic
interface until then.

---

## 7. Stage Persistence

### 7.1 Stages Survive App Launch and Exit (Within Session)

Stages are ephemeral per session — they are not persisted to disk. Within a session:

- **App exit**: the app is removed from its stage's `members` list. If other members
  remain, the stage is intact. If zero members remain, the stage becomes an orphan.
- **App launch (restart policy `always`)**: the app rejoins its prior stage when it
  restarts (by name lookup in the stage registry before creating a new stage).
- **Space switch**: stages for a space are preserved in `StageRegistry.spaces[N]` while
  space N is inactive. Switching back to space N restores the exact stage configuration.

### 7.2 Orphan Garbage Collection

An orphan stage (zero members) is retained for **5 seconds** with a visible empty card
in the strip (greyed out). After 5 seconds, it is removed from the stage list.

```rust
// supervisor/src/stage/registry.rs

pub struct OrphanTimer {
    pub stage_id: StageId,
    pub expires:  Instant,
}

// SupervisorState holds: pub orphan_timers: Vec<OrphanTimer>
// On each supervisor tick (not compositor tick), expire and remove:
fn tick_orphan_gc(state: &mut SupervisorState) {
    let now = Instant::now();
    let expired: Vec<StageId> = state.orphan_timers.iter()
        .filter(|t| t.expires <= now)
        .map(|t| t.stage_id)
        .collect();
    for id in expired {
        state.orphan_timers.retain(|t| t.stage_id != id);
        let space = state.spaces.active;   // orphan GC only runs for active space
        if let Some(ss) = state.stage_registry.spaces.get_mut(&space) {
            ss.stages.retain(|s| s.id != id);
            // Notify strip to remove card
            state.stage_strip_router.route(
                format!("VYOMA_STAGE:stage_removed:{}\n", id),
                &state.stage_strip_tx,
            );
        }
    }
}
```

**Invariant**: The active stage is never an orphan. If all members of the active stage
exit, the next stage in the strip (index 0) is automatically promoted to active before
the orphan timer starts on the former active stage.

### 7.3 Stage Persistence Across Space Switch

```rust
// supervisor/src/stage/registry.rs

// On space switch (applied in vsync_tick after PENDING_SPACE_SWITCH):
pub fn on_space_switched(registry: &mut StageRegistry, new_space: u8) {
    // Old space stages: all apps go off-screen (already handled by win_x = OFFSCREEN_X)
    // New space stages: active stage apps go to main area positions; inactive go off-screen
    if let Some(ss) = registry.spaces.get(&new_space) {
        if !ss.enabled { return; }
        // reapply_stage_layout handles both active and inactive positioning
    }
}
```

---

## 8. Interaction with Spaces

### 8.1 Per-Space Stage Lists

Each space has its own independent `SpaceStages` entry in `StageRegistry.spaces`. Space 1
stages are completely separate from space 2 stages. Switching spaces shows only that
space's stage strip.

### 8.2 Stage Manager Enabled State is Per-Space

Each `SpaceStages.enabled: bool` flag is independent. The user may have Stage Manager on
in space 1 and off in space 2. Toggling via:
```
@supervisor: stage_toggle          ← toggle for active space
@supervisor: stage_enable <space>  ← enable for specific space
@supervisor: stage_disable <space> ← disable for specific space
```

### 8.3 Stage Manager + Space Switch Sequence

When Stage Manager is enabled and the user switches spaces:

1. `PENDING_SPACE_SWITCH` is set (normal R21 mechanism).
2. Compositor tick applies space switch (same as non-Stage-Manager path).
3. After space switch, `on_space_switched` is called: the new space's active stage apps
   are assigned main-area positions; inactive stage apps go off-screen.
4. `pending_resize` drains normally on next compositor tick.
5. If the new space has Stage Manager disabled, normal tiling is applied.

No new atomic is introduced for Stage Manager — it piggybacks on `PENDING_SPACE_SWITCH`
and `pending_resize` from R21.

---

## 9. Interaction with Dock

### 9.1 Dock Launches Into Active Stage by Default

When the user clicks a Dock icon while Stage Manager is active:

- The supervisor launches the app normally (or focuses it if already running).
- If the app is already running in an inactive stage, it is NOT automatically brought to
  the main area — the user must click the strip thumbnail to switch stages.
- If the app is already running in the active stage, it is focused (standard behavior).
- If the app is launching fresh, it is placed in a new single-member stage in the strip
  (not auto-added to active stage). This avoids disruptive grouping.

### 9.2 Dock Strip Clip

The dock remains at the bottom. The stage strip occupies `x = 0..140`. The dock occupies
`y = dock_y..logical_h`. These regions overlap only in the bottom-left corner (140 × 48pt).
The stage strip surface height is `logical_h - chrome_h - dock_h`, so it does NOT extend
into the dock region. No additional clip logic is needed.

### 9.3 Dock Running Indicators

Dock running indicators for apps in inactive stages continue to show (apps are Running,
not Terminated). This is correct because off-screen placement does not change lifecycle
state. The user can tell an app is running even if it is in an inactive stage.

---

## 10. Animation

### 10.1 Stage Swap Animation

The stage-switch animation is a horizontal slide:

- **Outgoing stage** (main area → strip): apps slide left from `main_x` toward
  `OFFSCREEN_X` over 200ms. Simultaneously fade from alpha 1.0 to 0.6.
- **Incoming stage** (strip → main area): apps start at `x = -main_w` (just off the
  left edge, hidden under the strip) and slide right to their final main-area positions
  over 200ms. Simultaneously fade from alpha 0.6 to 1.0.

Mathematically, the compositor applies a linear interpolation `t ∈ [0, 1]` at each tick:

```rust
// supervisor/src/stage/animation.rs

pub struct StageAnimation {
    pub from_stage_id: StageId,
    pub to_stage_id:   StageId,
    pub start_time:    Instant,
    pub duration_ms:   u32,         // 200
    pub phase:         AnimPhase,
}

pub enum AnimPhase {
    SlideOut,     // t=0..1: outgoing moves left, incoming moves in from left
    // Single combined phase: both happen simultaneously
}

pub fn apply_stage_animation(
    fb: &mut FrameBuffer,
    apps: &mut HashMap<String, AppState>,
    anim: &StageAnimation,
    t: f32,
    config: &DisplayConfig,
    registry: &StageRegistry,
) {
    let space = config.active_space;
    let ss = match registry.spaces.get(&space) {
        Some(s) => s,
        None => return,
    };

    let eased_t = ease_in_out_cubic(t);

    // Outgoing stage: main_x → -(main_w + STRIP_W)
    let out_x = lerp(MAIN_X_PX, -(MAIN_W_PX as i32) - STRIP_W_PX as i32, eased_t);
    let out_alpha = lerp(255.0, 153.0, eased_t) as u8;   // 1.0 → 0.6
    if let Some(from_stage) = ss.stages.iter().find(|s| s.id == anim.from_stage_id) {
        for name in &from_stage.members {
            if let Some(app) = apps.get_mut(name) {
                app.anim_x_override = Some(out_x.max(0) as u32);
                app.anim_alpha = Some(out_alpha);
            }
        }
    }

    // Incoming stage: -(main_w) → final main positions
    let in_x_offset = lerp(-(MAIN_W_PX as i32), 0, eased_t);
    let in_alpha = lerp(153.0, 255.0, eased_t) as u8;    // 0.6 → 1.0
    if let Some(to_stage) = ss.stages.iter().find(|s| s.id == anim.to_stage_id) {
        let rects = compute_main_area_layout(
            to_stage.members.len(), MAIN_X, MAIN_Y,
            config.logical_width - STRIP_W, config.logical_height - CHROME_H - DOCK_H,
        );
        for (name, rect) in to_stage.members.iter().zip(rects.iter()) {
            if let Some(app) = apps.get_mut(name) {
                let target_x_px = rect.0.saturating_mul(config.scale_factor as u32);
                let animated_x = (target_x_px as i32 + in_x_offset).max(STRIP_W_PX as i32);
                app.anim_x_override = Some(animated_x as u32);
                app.anim_alpha = Some(in_alpha);
            }
        }
    }
}
```

### 10.2 New AppState Animation Fields

```rust
// supervisor/src/app_state.rs — additions for Stage Manager animation

pub struct AppState {
    // ... existing fields ...
    pub anim_x_override: Option<u32>,   // physical px; overrides win_x during animation
    pub anim_alpha:      Option<u8>,    // 0-255; overrides surface alpha during blit
}
```

These are `None` at rest and set only during the ~200ms animation window. The blit path
checks `anim_x_override` and `anim_alpha` before using `win_x` and the default alpha.

### 10.3 Strip Card Animation

Simultaneously with the window animation, the strip reorders its cards. The outgoing
active stage card animates from the left (origin, since it was in the main area) into
the strip at the top position. The `stage-strip` WASM app handles this internally —
the supervisor sends it a `VYOMA_STAGE:stage_activated` event and `stage-strip` runs
its own card slide-down animation using `VYOMA_DRAW` commands.

The strip card animation is purely cosmetic — supervisor geometry decisions do not wait
for it.

---

## 11. Toggle On/Off

### 11.1 Toggle Command

```
@supervisor: stage_toggle                     ← toggle Stage Manager for active space
@supervisor: stage_toggle_all                 ← enable/disable across all spaces
```

### 11.2 Enable Sequence

When Stage Manager is enabled for a space:

1. Supervisor creates one stage per currently running non-space-0 app in that space.
2. The most-recently-focused app's stage becomes active.
3. Active stage apps are repositioned to the main area (140pt-offset tiling).
4. All other apps are moved off-screen.
5. The `stage-strip` WASM app surface is expanded to full strip height.
6. Supervisor sends `VYOMA_STAGE:enabled` + `VYOMA_STAGE:strip_init:<json_of_stages>` to
   `stage-strip`.
7. `stage-strip` draws initial cards from thumbnails (requests via `@supervisor:
   sm_thumb_request <stage_id>`).

### 11.3 Disable Sequence

When Stage Manager is disabled for a space:

1. All apps are brought back to screen (off-screen placement cleared).
2. Normal `apply_tiling_layout` is called for the space.
3. The `stage-strip` surface is collapsed to 0×0 and flushed.
4. `stage_strip_surface_ready` is set to false.
5. Stage data for the space is preserved in `StageRegistry` (not destroyed) — re-enabling
   Stage Manager restores the prior stage groupings.

### 11.4 Input Lock During Toggle

During the enable/disable transition (~100ms), `INPUT_LOCK_LEVEL` is raised to
`LockLevel::StageSwitch` to prevent key events from routing to apps with partially-
updated geometry. Released when all `pending_resize` drains complete (checked by
supervisor via a `resize_pending_count` counter decremented on each drain).

---

## 12. INPUT_LOCK_LEVEL Update

Stage Manager introduces a new lock level between None and MissionControl:

```rust
// supervisor/src/input.rs — updated LockLevel enum

pub enum LockLevel {
    None           = 0,
    StageSwitch    = 1,   // NEW: during stage animation / SM enable-disable
    MissionControl = 2,   // was 1 in R25 — BUMPED
    ChromeConsent  = 3,   // was 2 in R25 — BUMPED
}
```

**Breaking change from R25**: `MissionControl` moves from 1→2, `ChromeConsent` from 2→3.
All `compare_exchange` and `store` call sites using numeric literals must be updated to
use the enum variant. Sites using the enum variant are unchanged.

Priority routing (updated):
```rust
fn route_keyboard(ev: KeyEvent, focused_app: &str) {
    match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        3 => send_key_to_app("chrome", ev),
        2 => send_key_to_app("mission-control", ev),
        1 => { /* swallow keys during stage switch */ }
        _ => send_key_to_app(focused_app, ev),
    }
}
```

---

## 13. Thumbnail Capture for Strip

### 13.1 Reuse R25 Capture Infrastructure

Stage Manager thumbnails use the same `mc_capture.rs` `CaptureRequest` channel and
`capture_thread_main` from R25. A new `CaptureKind` discriminant is added:

```rust
// supervisor/src/mc_capture.rs

pub enum CaptureKind {
    MissionControl { all_spaces: bool },
    StageStrip     { app_name: String },   // NEW: single-app thumb for strip card
}

pub struct CaptureRequest {
    pub kind:     CaptureKind,
    pub reply_tx: Sender<Vec<ThumbData>>,
}
```

### 13.2 Stage Strip Thumbnail Protocol

When `stage-strip` needs to render a card thumbnail, it sends:
```
@supervisor: sm_thumb_request <stage_id>
```

Supervisor looks up the stage's primary member app name, posts a `CaptureRequest` with
`CaptureKind::StageStrip { app_name }` to the capture thread. Delivery to `stage-strip`
uses the same `thumb_begin/thumb_data/thumb_end` framing as R25.

Thumbnail size: `120×70px` physical (≈ `card_w × 70/scale_factor` logical), downscaled
in the capture thread.

### 13.3 Thumbnail Freshness

Strip thumbnails are updated on:
1. Stage switch (incoming active→strip needs fresh thumb of the stage now going inactive).
2. `AppState.thumb_dirty = true` set on each flush of apps in inactive stages — the
   supervisor queues a thumb refresh for that stage (debounced to at most once per 2s for
   inactive apps to avoid capture-thread saturation).

---

## 14. VYOMA_STAGE: Protocol

### 14.1 Supervisor → stage-strip (stdin push)

```
VYOMA_STAGE:enabled
VYOMA_STAGE:disabled
VYOMA_STAGE:strip_init:<json>           ← initial stage list; JSON array of {id, label}
VYOMA_STAGE:stage_activated:<stage_id>  ← new active stage (strip should reorder cards)
VYOMA_STAGE:stage_added:<stage_id>,<label>        ← new stage appeared in strip
VYOMA_STAGE:stage_removed:<stage_id>   ← orphan GC removed a stage
VYOMA_STAGE:stage_label:<stage_id>,<label>         ← label changed (win_title update)
VYOMA_STAGE:stage_orphaned:<stage_id>  ← all members exited; card greyed out
VYOMA_STAGE:thumb_begin:<stage_id>,<w_px>,<h_px>
VYOMA_STAGE:thumb_data:<base64_chunk>
VYOMA_STAGE:thumb_end
```

### 14.2 stage-strip → Supervisor (stdout)

```
@supervisor: stage_select <stage_id>          ← user clicked a strip card
@supervisor: sm_thumb_request <stage_id>      ← strip wants a fresh thumbnail
@supervisor: stage_strip_ready                ← strip signals initial draw complete
```

### 14.3 Supervisor IPC Commands (from any app with `shell = true`)

```
@supervisor: stage_toggle
@supervisor: stage_toggle_all
@supervisor: stage_enable <space>
@supervisor: stage_disable <space>
@supervisor: stage_new <app_name>
@supervisor: stage_add_to_active <app_name>
@supervisor: stage_move <app_name> <stage_id>
@supervisor: stage_remove <app_name>
@supervisor: stage_merge <id1> <id2>
@supervisor: stage_select <stage_id>
```

---

## 15. WIT Interface `vyoma:stage-manager@1.0.0`

```wit
package vyoma:stage-manager@1.0.0;

record stage-info {
    id:      u32,
    label:   string,
    members: list<string>,  // app names
    active:  bool,
}

interface stage-manager {
    list-stages:          func() -> result<list<stage-info>, string>;
    active-stage:         func() -> result<u32, string>;            // returns stage id
    select-stage:         func(stage-id: u32) -> result<_, string>;
    new-stage:            func(app-name: string) -> result<u32, string>;
    add-to-active:        func(app-name: string) -> result<_, string>;
    move-to-stage:        func(app-name: string, stage-id: u32) -> result<_, string>;
    merge-stages:         func(target-id: u32, src-id: u32) -> result<_, string>;
    stage-manager-enabled: func() -> result<bool, string>;
    set-stage-manager:    func(enabled: bool) -> result<_, string>;
}

world stage-manager-world {
    import stage-manager;
}
```

---

## 16. Platform Matrix

| Feature | desktop-full | mobile | server-headless | iot-edge | robotics-rt | mcu-minimal |
|---------|-------------|--------|----------------|----------|-------------|-------------|
| Stage Manager | yes | yes (simplified) | no | no | no | no |
| stage-strip WASM app | yes | yes | no | no | no | no |
| Left 140pt strip | yes | no (bottom row) | no | no | no | no |
| Stage animation | yes | yes (fade only) | no | no | no | no |
| Multi-member stages | yes | yes | no | no | no | no |
| Auto-group on launch | yes | yes | no | no | no | no |

**Mobile variant** (`mobile` profile): Stage Manager is the default layout mode (iPad
Stage Manager analogue). The strip is not a left-side column but a bottom row of cards
above the dock — 120pt tall, horizontally scrolling. Card layout and animation differ but
the data model and supervisor-side stage registry are identical. The `stage-strip` WASM
app reads its layout mode from an env var set by the platform profile loader.

**server-headless, iot-edge, robotics-rt, mcu-minimal**: no display subsystem; Stage
Manager is gated out by the same `PLATFORM_HAS_DISPLAY` flag used for chrome/dock.

---

## 17. Compositor Integration Summary

The `flush_pass` function (R24 Model A) is updated to handle four space-0 apps:

```rust
// supervisor/src/compositor.rs — flush_pass() updated for R26

fn flush_pass(
    fb: &mut FrameBuffer,
    apps_sorted: &[AppSnapshot],
    config: &DisplayConfig,
    chrome_surface_ready: bool,
    dock_surface_ready: bool,
    stage_strip_surface_ready: bool,
)
{
    let chrome_h_px     = 24u32 * config.scale_factor as u32;
    let dock_clip_y_px  = config.physical_height - 48u32 * config.scale_factor as u32;
    let strip_clip_x_px = if stage_strip_active {
        140u32 * config.scale_factor as u32
    } else {
        0
    };

    // Startup fallbacks
    if !chrome_surface_ready {
        fb.fill_rows(0, chrome_h_px, MENU_BAR_BG_COLOR);
    }
    if !dock_surface_ready {
        fb.fill_rows(dock_clip_y_px, config.physical_height - dock_clip_y_px, MENU_BAR_BG_COLOR);
    }
    if stage_strip_active && !stage_strip_surface_ready {
        fb.fill_cols(0, strip_clip_x_px, chrome_h_px, dock_clip_y_px, STAGE_STRIP_BG_COLOR);
    }

    // Pass 1: active-space apps (space-N)
    // blit_clipped gains strip_clip_x_px as a new left-edge exclusion parameter
    let mut space_n: Vec<_> = apps_sorted.iter()
        .filter(|a| a.win_space != 0)
        .collect();
    space_n.sort_by_key(|a| a.z_index);
    for snap in &space_n {
        blit_clipped(fb, snap, config, chrome_h_px, dock_clip_y_px, strip_clip_x_px);
    }

    // Pass 2: space-0 apps (ascending z: strip=65532, mc=65533, dock=65534, chrome=65535)
    let mut space_0: Vec<_> = apps_sorted.iter()
        .filter(|a| a.win_space == 0)
        .collect();
    space_0.sort_by_key(|a| a.z_index);
    for snap in &space_0 {
        blit_clipped(fb, snap, config, chrome_h_px, dock_clip_y_px, 0);
        // space-0 apps: no left-strip clip — they own their region
    }
}
```

### 17.1 `blit_clipped` Left-Edge Extension

The 6-step `blit_clipped` function (R24 B3 fix) gains a Step 0 — left-edge clip for
space-N apps:

```rust
// Step 0 (new): left-edge clip for stage strip exclusion
let effective_dest_x = win_x_px.max(strip_clip_x_px);
let src_x_offset = effective_dest_x.saturating_sub(win_x_px);
let blittable_w_after_left_clip = snap.surface_width_px.saturating_sub(src_x_offset);
// Remainder of steps 1-6 proceed with effective_dest_x and blittable_w_after_left_clip
```

This ensures that if any app window is positioned in the strip region (e.g., due to
an off-by-one in layout calculation), its pixels are clipped to the main area.

---

## 18. File Layout

```
supervisor/src/stage/
├── mod.rs          (StageRegistry, SpaceStages, Stage, StageId type alias)
├── registry.rs     (SpaceStages impl: active_stage, inactive_stages, orphan GC)
├── activation.rs   (handle_stage_select, handle_stage_add_to_active, handle_stage_merge)
├── layout.rs       (compute_main_area_layout, OFFSCREEN_X/Y constants, STRIP_W/MAIN_X consts)
├── animation.rs    (StageAnimation, AnimPhase, apply_stage_animation, ease_in_out_cubic)
└── commands.rs     (handle_stage_toggle, handle_stage_enable, handle_stage_new,
                     handle_stage_move, handle_stage_remove, tick_orphan_gc)

supervisor/src/
├── stage_strip_router.rs  (StageStripRouter — mirror of chrome_router.rs; 64-slot VecDeque)
├── compositor.rs          (flush_pass: 4 space-0 apps, blit_clipped left-edge step 0,
                            anim_x_override/anim_alpha blit path, stage_strip_surface_ready)
├── app_state.rs           (anim_x_override: Option<u32>, anim_alpha: Option<u8>)
├── input.rs               (LockLevel: StageSwitch=1, MissionControl=2, ChromeConsent=3)
├── mc_capture.rs          (CaptureKind enum: MissionControl | StageStrip, sm_thumb_request)
└── main.rs                (stage_registry init, stage_toggle handler, stage_strip_tx channel)

apps/stage-strip/src/
├── main.rs    (event loop: VYOMA_STAGE: parsing, thumb decode, draw calls, stage_select emit)
├── state.rs   (StripState: cards Vec<StripCard>, active_stage_id, scroll_offset)
└── draw.rs    (render_strip_cards, render_orphan_card, render_card_thumbnail)
```

---

## 19. Invariants and Assertions

The following invariants must hold at all times (enforced by debug assertions in
`stage/registry.rs`):

1. Every non-space-0 running app in a Stage-Manager-enabled space is a member of exactly
   one stage in that space.
2. The `SpaceStages.active` ID always refers to a stage in `SpaceStages.stages`.
3. The active stage has at least one member app that is `Running` or `Launching`.
4. `anim_x_override` and `anim_alpha` are `None` on all `AppState`s outside of a
   `stage_anim` compositor pass.
5. `stage_anim` is `None` when `INPUT_LOCK_LEVEL != LockLevel::StageSwitch`.

---

## 20. Open Questions (deferred)

1. **Drag-to-group / drag-to-ungroup**: deferred to R32 (Mouse & Gestures). The strip
   thumbnail drag target and the main-area drop zone geometry are pre-defined here but
   gesture recognition is not.

2. **Strip scrolling**: more than 5 inactive stages overflow the strip. Scroll gesture
   deferred to R32. For now, only the first 5 are shown.

3. **Stage naming / custom labels**: the label defaults to the primary member's
   `win_title`. User-editable stage labels deferred to a later round.

4. **Stage persistence across reboot**: no disk persistence in R26. Stage groupings are
   session-only.

5. **Stage Manager + fullscreen app**: an app in fullscreen mode (`win_fullscreen = true`)
   forces Stage Manager strip hidden for that space until the app exits fullscreen.
   Interaction details deferred to R27 (Fullscreen).

6. **Thumbnail GIF-like animation**: showing last-N frames of active app as strip card
   animation (macOS live thumbnails). Deferred — requires ring-buffer snapshot capture
   changes beyond R25 scope.
