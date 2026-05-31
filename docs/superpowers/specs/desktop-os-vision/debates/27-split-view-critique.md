# Critique: Full Screen & Split View (Round 27)

**Subsystem**: Full Screen & Split View  
**Round**: 27  
**Critiquing**: `27-split-view.md`

---

## CRITIQUE — 5 Blocking Issues

---

### B1: `FS_CHROME_SUPPRESSED` / `FS_DOCK_SUPPRESSED` race between compositor tick and space switch

**Problem**:

Section §4.3 specifies that `FS_CHROME_SUPPRESSED` and `FS_DOCK_SUPPRESSED` are
`AtomicBool` globals updated in the compositor tick *after* the
`PENDING_SPACE_SWITCH` block. However, there is a window between the space switch
being committed and the suppression flags being updated. Specifically:

```rust
// vsync_tick:
// 1. Apply PENDING_SPACE_SWITCH (spaces.active = new_space)
// 2. ... suppression flags NOT yet updated here ...
// 3. Drain pending_resize
// 4. BEGIN blit loop   ← at this point chrome/dock may be wrongly visible/hidden
//    chrome_clip_px = if FS_CHROME_SUPPRESSED ... ← read here
// 5. (later) FS_CHROME_SUPPRESSED.store(...)      ← update happens AFTER the read
```

If the space switch changes to a FS space, the suppression flag is set to `true` only
*after* the blit loop has already read it as `false`. Chrome and dock are blitted for
exactly one frame while the FS app has already been composited full-screen — producing
a one-frame glitch where chrome and dock incorrectly overlay the FS app.

The reverse glitch occurs on exit: switching away from a FS space, the suppression
flag remains `true` during the first non-FS frame, making chrome and dock invisible
for one tick.

**Proposed fix**:

Move the `FS_CHROME_SUPPRESSED` / `FS_DOCK_SUPPRESSED` update to be computed
*inside* the `vsync_lock.write()` block that applies the space switch, before the
`pending_resize` drain and before the blit loop:

```rust
// supervisor/src/compositor.rs — vsync_tick(), inside space switch write-lock block

let _write = vsync_lock.write();
spaces.active = pending;
fb.fill(BACKGROUND_COLOR);
mark_new_space_dirty(&mut apps, pending);
compact_z_order(&mut apps, pending);

// Compute FS suppression flags atomically with space switch
let is_fs = fs_registry.find_by_space(pending).is_some();
FS_CHROME_SUPPRESSED.store(is_fs, Ordering::Release);
FS_DOCK_SUPPRESSED.store(is_fs, Ordering::Release);
// write-lock drops here
```

This guarantees that when the blit loop reads the flags, they already reflect the
newly active space with no intervening tick where they are stale.

Additionally, `chrome_clip_px` and `dock_clip_px` should be computed inside the
`vsync_lock.read()` block (after the write-lock is dropped, before the blit loop)
so that they read the freshly updated atomics:

```rust
let _read = vsync_lock.read();
let chrome_clip_px = if FS_CHROME_SUPPRESSED.load(Ordering::Acquire) { 0 }
                      else { CHROME_H_PTS * config.scale_factor as u32 };
let dock_clip_px   = if FS_DOCK_SUPPRESSED.load(Ordering::Acquire) { config.physical_height }
                      else { config.physical_height - DOCK_H_PTS * config.scale_factor as u32 };
// blit loop follows, using chrome_clip_px and dock_clip_px
```

---

### B2: `release_fs_space` space-count shrinking logic is incorrect for non-tail spaces

**Problem**:

Section §4.2 specifies `release_fs_space` as:

```rust
if space == space_reg.count && space_reg.count > 1 {
    space_reg.count -= 1;
}
```

This only shrinks the count when the released space is the *last* (highest-numbered)
space. If FS spaces are allocated in the middle or multiple FS spaces exist
(e.g., spaces 1,2,3,4 exist; FS spaces are 3 and 4; user exits space 3 but not 4),
releasing space 3 leaves space_reg.count = 4 correctly. But if the user then exits
space 4, space_reg.count drops to 3 and space 3 is "reused" for the next normal space
or FS allocation — except that space 3 already had apps migrated to space 1 and its
entry in `fs_spaces` was not actually removed.

More critically: if FS spaces are 2 and 3 (with normal space 1), releasing space 2
does nothing (2 ≠ count=3). Space 2 becomes a "ghost" — `fs_spaces` says it is
released but it still exists in SpaceRegistry conceptually. The next `allocate_fs_space`
sets count=4 and allocates space 4, leaving space 2 as a permanent gap. Over time,
repeated FS-enter/exit cycles leak space numbers upward until the cap of 9 is hit.

**Proposed fix**:

`SpaceRegistry` needs a proper free-list instead of relying on the high-water-mark
`count` field. Add a `free_list: Vec<u8>` to `SpaceRegistry` for released spaces:

```rust
// supervisor/src/wm/spaces.rs

pub struct SpaceRegistry {
    pub active:    u8,
    pub count:     u8,       // total allocated (high-water mark, 1-9)
    pub free_list: Vec<u8>,  // previously-allocated spaces now available for reuse
}

impl SpaceRegistry {
    pub fn allocate_next(&mut self) -> Option<u8> {
        if let Some(s) = self.free_list.pop() {
            return Some(s);   // reuse released space
        }
        if self.count >= 9 { return None; }
        self.count += 1;
        Some(self.count)
    }

    pub fn release(&mut self, space: u8) {
        // Don't add to free_list if it is the top of the high-water mark
        if space == self.count && self.count > 1 {
            self.count -= 1;
        } else if space >= 1 && space <= self.count && !self.free_list.contains(&space) {
            self.free_list.push(space);
        }
    }
}
```

`FsRegistry::release_fs_space` calls `space_reg.release(space)` instead of the
inline shrink logic. MC space-strip must skip spaces in the `free_list` when rendering
space cards. `PENDING_SPACE_SWITCH` validation must reject free-list spaces.

---

### B3: `FsEnterAnimation` reuses `LockLevel::StageSwitch` — conflicts with concurrent stage animation

**Problem**:

Section §17 specifies that FS enter/exit animations set
`INPUT_LOCK_LEVEL = LockLevel::StageSwitch (3)` and rely on `advance_fs_anim`
clearing it when `t >= 1.0`. However, a concurrent stage switch animation
(R26 `StageAnimation`) also sets and clears `LockLevel::StageSwitch`. The two
animations can conflict:

**Scenario**:
1. User triggers FS enter on an app in a Stage Manager space → `INPUT_LOCK_LEVEL = 3`.
2. The app is removed from its stage (§10.1). If Stage Manager has multiple stages,
   this triggers `handle_stage_select` for the now-empty stage → also attempts to set
   `INPUT_LOCK_LEVEL = 3` and install a `StageAnimation`.
3. Both `advance_fs_anim` and `advance_frame` (stage animation) run concurrently.
   When the stage animation finishes first, it clears `INPUT_LOCK_LEVEL = 3` (via
   `compare_exchange`) — but the FS animation is still in progress. Keys are now
   unblocked mid-FS-transition.
4. Conversely, when FS animation finishes, it clears the lock even if a stage animation
   is still running.

Additionally, reusing the `StageSwitch` lock name for FS animation makes it impossible
for `route_keyboard` to distinguish "stage animation in progress" from "FS animation in
progress" — if future rounds need to route keys differently during FS transitions, this
becomes a maintenance hazard.

**Proposed fix**:

Add a dedicated `LockLevel::FsTransition = 4` to the `LockLevel` enum:

```rust
// supervisor/src/input.rs

pub enum LockLevel {
    None           = 0,
    MissionControl = 1,
    ChromeConsent  = 2,
    StageSwitch    = 3,   // R26 — stage animation swallow
    FsTransition   = 4,   // R27 — full-screen enter/exit swallow
}
```

FS enter/exit uses `LockLevel::FsTransition`:
```rust
INPUT_LOCK_LEVEL.store(LockLevel::FsTransition as u8, Ordering::Release);
```

`route_keyboard` updated:
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

`on_app_process_exited` (lifecycle crash handler) should auto-clear `FsTransition`
via `compare_exchange`:
```rust
INPUT_LOCK_LEVEL.compare_exchange(
    LockLevel::FsTransition as u8,
    LockLevel::None as u8,
    Ordering::AcqRel, Ordering::Relaxed,
).ok();
```

No existing R25/R26 `compare_exchange` call sites are affected (they use values 1, 2, 3).
`FsTransition = 4` does not renumber any existing constant.

---

### B4: `splitview_exit_app` does not restore the *exiting* app to its `fs_pre_space` before stage re-integration

**Problem**:

Section §7.3 specifies the `splitview_exit_app` sequence as:

```
Step 1: Restore exiting app geometry (fs_pre_x/y/w/h/space; set fs_mode = Normal)
Step 2: Promote remaining app to FullScreen
Step 3: Update FsPair.secondary = ""
Step 4: Send VYOMA_FS notifications
```

However, Step 1 sets `app.win_space = app.fs_pre_space` (implied by "restore"),
but the spec for §10.3 (Stage re-integration) says the app is re-added to its
`fs_pre_space`'s active stage *after* the geometry restore. There is a critical
missing sub-step: the `pending_focus` and `PENDING_SPACE_SWITCH` ordering from R25
is not applied here.

Concretely: when the exiting app is restored to `fs_pre_space`, if that space is
not the currently active space, the app's `win_space` is changed (Step 1) but the
user is left staring at the SplitView dedicated space (which now shows only one app).
No space switch is issued. The MC space strip still shows the FS space card as valid.

Furthermore, §10.3's re-integration code:
```rust
if let Some(stage) = ss.stages.iter_mut().find(|s| s.id == ss.active) {
    stage.members.push(app.name.clone());
}
```
runs immediately during the IPC handler — before `PENDING_SPACE_SWITCH` is applied
and before `compact_z_order` is called for the restored space. The app is in the stage
list but has no z-index in its restored space (its `PerSpaceZ` for `fs_pre_space` may
have been stale or zero since FS entry moved the app out of that space).

**Proposed fix**:

Add two explicit steps to the `splitview_exit_app` sequence and fix the ordering:

```
Step 1: Restore exiting app geometry
  app.win_x = app.fs_pre_x; app.win_y = app.fs_pre_y;
  app.pending_resize = Some((app.fs_pre_w, app.fs_pre_h));
  app.win_space = app.fs_pre_space;
  app.fs_mode = Normal; app.split_role = None; app.split_ratio = None;

Step 1a: Re-assign z-order in restored space
  // Compact BEFORE stage re-integration so z is valid when stage renders
  app.win_z.set(app.fs_pre_space, 0);       // will be compacted to correct value
  wm::zorder::compact_z_order(&mut apps, app.fs_pre_space);

Step 1b: Issue space switch to restored space (deferred via PENDING_FOCUS pattern)
  PENDING_SPACE_SWITCH.store(app.fs_pre_space, Ordering::Release);
  state.pending_focus = Some(app.name.clone());   // R25 deferred focus pattern

Step 1c: Re-integrate into stage (AFTER compact, BEFORE space switch applies)
  if let Some(ss) = stage_reg.spaces.get_mut(&app.fs_pre_space) {
      if ss.enabled {
          if let Some(stage) = ss.stages.iter_mut().find(|s| s.id == ss.active) {
              stage.members.push(app.name.clone());
          }
      }
  }

Step 2: Promote remaining app to FullScreen (unchanged)
Step 3: Update FsPair (unchanged)
Step 4: Notifications (unchanged)
```

The `PENDING_FOCUS` + `PENDING_SPACE_SWITCH` pattern (R25 §11) guarantees the exiting
app is focused *after* the space switch commit — exactly the same ordering guarantee
used by Mission Control exit.

---

### B5: `compute_tile_rect` does not account for Stage Manager strip when SM is enabled, but the conditional adjustment is placed in the wrong layer

**Problem**:

Section §9.5 acknowledges that tile geometry must account for the 140pt Stage Manager
strip. The adjustment is expressed as inline conditional logic inside an unnamed call
site:

```rust
// If SM enabled, adjust left tile to start at STRIP_W + MAIN_MARGIN:
if sm_enabled && side == TileSide::Left {
    rect.x = STRIP_W + MAIN_MARGIN;
    rect.w = (config.logical_width / 2).saturating_sub(STRIP_W + MAIN_MARGIN);
}
```

This logic is outside `compute_tile_rect` (§9.3) — it mutates the returned `Rect`
after the fact. The problem is threefold:

1. **The call site is unspecified.** `handle_tile_window` (§9.2) calls
   `compute_tile_rect` and applies `pending_resize`. The SM adjustment must happen
   between these two calls but no specific call site in `fs/commands.rs` is named.
   Implementors may place it in the wrong place (e.g., inside `blit_clipped` rather
   than during geometry setup), producing the wrong surface size.

2. **Surface size vs. blit position mismatch.** `pending_resize` sends the surface
   *dimensions* to the app. If SM is enabled, the tile width is narrower than `logical_w/2`.
   But if the SM conditional is not applied before `pending_resize = Some((w, h))` is set,
   the app allocates a full `logical_w/2`-wide surface and the supervisor blits it from
   `x = STRIP_W + MAIN_MARGIN` — meaning the right portion of the app's surface is
   clipped by `strip_clip_x_px`. The app draws into regions that are never shown,
   wastes memory, and the left edge of the visible content is cut off.

3. **SM toggle while tiled.** If the user enables or disables Stage Manager while an
   app is tiled (`fs_mode = TileWindow`), the tile geometry is not recomputed. The app
   remains at its old position and size, now misaligned with or overlapping the strip.

**Proposed fix**:

Move the SM awareness into `compute_tile_rect` as a first-class parameter:

```rust
// supervisor/src/fs/layout.rs

pub fn compute_tile_rect(
    side: TileSide,
    config: &DisplayConfig,
    sm_enabled: bool,
) -> Rect {
    let content_y = CHROME_H_PTS;
    let content_h = config.logical_height.saturating_sub(CHROME_H_PTS + DOCK_H_PTS);
    let strip_offset = if sm_enabled { STRIP_W + MAIN_MARGIN } else { 0 };
    let usable_w = config.logical_width.saturating_sub(strip_offset);
    let half_w = usable_w / 2;
    match side {
        TileSide::Left  => Rect { x: strip_offset, y: content_y, w: half_w, h: content_h },
        TileSide::Right => Rect { x: strip_offset + half_w, y: content_y,
                                   w: usable_w - half_w, h: content_h },
        TileSide::None  => unreachable!(),
    }
}
```

All callers pass `sm_enabled` from `SpaceStages.enabled` for the relevant space.

For the SM toggle case, add a hook in the Stage Manager toggle handler
(`fs/commands.rs` or `stage/events.rs`):

```rust
// supervisor/src/stage/events.rs — on_stage_manager_toggled()

pub fn on_stage_manager_toggled(enabled: bool, apps: &mut HashMap<String, AppState>,
                                 config: &DisplayConfig, space: u8) {
    // Recompute geometry for any tiled app in this space
    for app in apps.values_mut().filter(|a| a.win_space == space
                                          && a.fs_mode == FullScreenMode::TileWindow) {
        let new_rect = compute_tile_rect(app.tile_side, config, enabled);
        app.win_x = new_rect.x;
        app.win_y = new_rect.y;
        app.pending_resize = Some((new_rect.w, new_rect.h));
        send_to_app_stdin(&app.name,
            &format!("VYOMA_FS:tile_geometry_changed:{},{},{},{}\n",
                     new_rect.x, new_rect.y, new_rect.w, new_rect.h));
    }
}
```

This hook is called at the end of the SM enable and disable sequences (R26 §10.1 and
§10.2) for the active space.
