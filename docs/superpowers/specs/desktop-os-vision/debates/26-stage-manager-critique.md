# CRITIQUE — 5 Blocking Issues

**Subsystem**: Stage Manager (Round 26)
**Reviewing**: `26-stage-manager.md`

---

## B1: `INPUT_LOCK_LEVEL` Numeric Value Renumbering Breaks R25 Call Sites

### Problem

Section 12 of the spec promotes `MissionControl` from 1→2 and `ChromeConsent` from
2→3 to insert `StageSwitch = 1`. This is a **breaking change** to the `LockLevel` enum
that affects every existing call site from R23, R24, and R25 that stored or compared
against numeric values.

The R25 spec (section 4) contains several `compare_exchange` calls using the enum variant
names, but also stores `LockLevel::ChromeConsent as u8` and `PREV_LOCK_LEVEL` which hold
**runtime numeric values** written to `AtomicU8` during R25's lifetime. If the binary is
rebuilt with the new enum but any in-flight atomic value read by a compositor tick was
written by code compiled against the old enum (e.g., during a live reload scenario or if
the renumbering is applied incrementally), the priority routing table silently misroutes
keys. Even without hot-reload, the R26 spec provides no migration plan for the R23
`ChromeRouter` consent-lock raise/lower sequence, which uses a concrete `as u8` cast
visible in the normative code of R25 section 4.1.

More concretely: `on_chrome_lifecycle_change` in R25 calls:
```rust
INPUT_LOCK_LEVEL.compare_exchange(
    LockLevel::ChromeConsent as u8,   // was 2, now must be 3
    PREV_LOCK_LEVEL.load(...),
    ...
)
```
If that site is not updated, it compares against `2` when the current value is `3`, the
`compare_exchange` fails silently, and the lock is never released — input permanently
jammed after any chrome consent dialog.

### Proposed Fix

Do NOT renumber existing levels. Instead, insert `StageSwitch` at a value that does not
displace `MissionControl(1)` and `ChromeConsent(2)`:

```rust
pub enum LockLevel {
    None           = 0,
    MissionControl = 1,   // UNCHANGED from R25
    ChromeConsent  = 2,   // UNCHANGED from R25
    StageSwitch    = 3,   // NEW — higher numeric value, lower routing priority than MC
}
```

Update `route_keyboard` to handle the new ordering:

```rust
fn route_keyboard(ev: KeyEvent, focused_app: &str) {
    match INPUT_LOCK_LEVEL.load(Ordering::Acquire) {
        2 => send_key_to_app("chrome", ev),
        1 => send_key_to_app("mission-control", ev),
        3 => { /* swallow keys during stage switch; stage-strip never receives direct keyboard */ }
        _ => send_key_to_app(focused_app, ev),
    }
}
```

Priority semantics: `ChromeConsent(2)` > `MissionControl(1)` > `StageSwitch(3)` > `None(0)`.
The routing match is ordered by descending priority (highest number wins for consent and MC),
and `StageSwitch(3)` acts as a swallow-only lock with lowest routing priority. This preserves
all R25 call sites unchanged. All new R26 sites use `LockLevel::StageSwitch as u8 = 3`.

---

## B2: Off-Screen Placement Does Not Prevent Inactive-Stage Apps From Receiving Draw Commands

### Problem

Section 4.3 specifies that inactive-stage apps are hidden by setting `win_x = OFFSCREEN_X
= 0xFFFF`. However, these apps are NOT suspended (`win_visible = true`, lifecycle =
`Running`). This means:

1. These apps continue executing their draw loops and emitting `VYOMA_DRAW:` commands.
2. The supervisor processes these commands (calls `handle_draw_cmd`) and writes pixels
   into their `Surface` buffers — wasted CPU every frame.
3. The `last_flushed_snapshot` for off-screen apps is continuously updated, meaning the
   MC capture thread and stage-strip thumb refreshes can race with active draw command
   processing with no quiescence guarantee.
4. If the app's draw loop emits a `VYOMA_DRAW:resize_surface` command while off-screen,
   the supervisor will reallocate the surface and set `pending_resize`, which causes a
   compositor tick to call `drain_pending_resizes` — potentially changing `win_w`/`win_h`
   for an off-screen app in a way that corrupts the geometry that `handle_stage_select`
   prepared for when the stage becomes active.

The spec states in section 4.3: "Their surfaces remain allocated and continue to receive
draw commands; they are just not composited." This is presented as safe, but the
`resize_surface` risk and the CPU waste are not addressed.

### Proposed Fix

Mark inactive-stage apps as **semi-quiesced**: allow draw commands to the surface buffer
(so the app's render state stays fresh) but **block `resize_surface` commands** and
**block `last_flushed_snapshot` updates** for off-screen apps:

```rust
// supervisor/src/draw_cmd.rs — handle_draw_cmd() additions

pub fn handle_draw_cmd(app: &mut AppState, cmd: DrawCmd) {
    if app.surface_quiesced { return; }  // R25: fully quiesced apps
    match &cmd {
        DrawCmd::ResizeSurface { w, h } => {
            if app.stage_offscreen {
                // Store resize intent but do NOT apply now; apply when stage becomes active
                app.pending_offscreen_resize = Some((*w, *h));
                return;
            }
            // ... normal resize handling ...
        }
        DrawCmd::Flush => {
            // Normal flush: update surface, DRM blit handled by compositor.
            // BUT: do NOT update last_flushed_snapshot for off-screen apps here.
            // The snapshot is updated only when the stage becomes active (see activation.rs).
            if !app.stage_offscreen {
                app.last_flushed_snapshot = Some(Arc::new(app.surface.clone()));
            }
            // Still update surface_dirty so compositor knows to re-blit when stage activates.
        }
        _ => { /* normal handling */ }
    }
}
```

Add to `AppState`:
```rust
pub stage_offscreen: bool,              // true when app is in inactive stage
pub pending_offscreen_resize: Option<(u32, u32)>,  // resize deferred until stage activation
```

On stage activation (`handle_stage_select`), drain `pending_offscreen_resize` for each
incoming member by converting it to `pending_resize` (the normal R21 drain path). This
ensures the surface is the right size when the app first appears in the main area.

---

## B3: `StageAnimation.from_stage_id` Members Have Undefined Final Geometry

### Problem

Section 5.2 (`handle_stage_select`) moves the outgoing stage's apps off-screen by
eventually setting `win_x = OFFSCREEN_X` after the animation completes (`finalize_stage_switch`).
But the spec does not define **what `finalize_stage_switch` does to the outgoing stage's
apps** in detail. Specifically:

1. During animation, `anim_x_override` is set for outgoing apps. When `t >= 1.0`, the
   animation is cleared (`state.stage_anim = None`) and `anim_x_override` is set back to
   `None`. What is `win_x` for the outgoing apps at this point? The spec never sets
   `win_x = OFFSCREEN_X` for them during the switch sequence — only the incoming stage's
   apps are pre-positioned (section 5.2 pre-positions incoming members with `pending_resize`
   but says nothing about outgoing `win_x`).

2. If `anim_x_override` becomes `None` (animation cleared) before `win_x` is updated to
   `OFFSCREEN_X`, there is a **one-tick window** where the compositor blits outgoing apps
   at their old main-area `win_x`, causing a visual flash of the wrong stage's windows
   in the main area.

3. `finalize_stage_switch` is mentioned in section 5.3 but never defined (no code, no
   function signature). It is a load-bearing function with no specification.

### Proposed Fix

Specify `finalize_stage_switch` explicitly and ensure the off-screen move is atomic with
animation completion:

```rust
// supervisor/src/stage/animation.rs

/// Called inside vsync_tick when t >= 1.0. Holds vsync_lock.write() implicitly via
/// the outer compositor write pass.
pub fn finalize_stage_switch(
    apps: &mut HashMap<String, AppState>,
    registry: &mut StageRegistry,
    config: &DisplayConfig,
    space: u8,
    anim: &StageAnimation,
) {
    let ss = match registry.spaces.get_mut(&space) {
        Some(s) => s,
        None => return,
    };

    // Step 1: Move outgoing stage apps off-screen (MUST happen before clearing anim_x_override)
    if let Some(from_stage) = ss.stages.iter().find(|s| s.id == anim.from_stage_id) {
        for name in &from_stage.members.clone() {
            if let Some(app) = apps.get_mut(name) {
                app.win_x = OFFSCREEN_X;
                app.win_y = OFFSCREEN_Y;
                app.stage_offscreen = true;
                app.anim_x_override = None;   // clear override only AFTER win_x is offscreen
                app.anim_alpha = None;
            }
        }
    }

    // Step 2: Finalize incoming stage apps at their main-area positions
    let to_members = ss.stages.iter()
        .find(|s| s.id == anim.to_stage_id)
        .map(|s| s.members.clone())
        .unwrap_or_default();
    let rects = compute_main_area_layout(
        to_members.len(), MAIN_X, MAIN_Y,
        config.logical_width - STRIP_W, config.logical_height - CHROME_H - DOCK_H,
    );
    for (name, rect) in to_members.iter().zip(rects.iter()) {
        if let Some(app) = apps.get_mut(name) {
            app.win_x = rect.0;
            app.win_y = rect.1;
            app.stage_offscreen = false;
            app.anim_x_override = None;   // clear override AFTER win_x is correct
            app.anim_alpha = None;
        }
    }
}
```

The key ordering rule: **set `win_x` before clearing `anim_x_override`**. This eliminates
the one-tick flash. Adding this requirement as an explicit code comment in `animation.rs`.

---

## B4: `StageStripRouter` Is Not Defined for the `stage-strip` Startup / Not-Running State

### Problem

Section 18 lists `stage_strip_router.rs` as a mirror of `chrome_router.rs`, but the spec
never defines the three lifecycle callbacks that make the router safe:
`on_strip_running()`, `on_strip_not_running()`, and `on_strip_terminated_no_restart()`.

More critically, section 11.2 (Enable Sequence) sends `VYOMA_STAGE:enabled` and
`VYOMA_STAGE:strip_init` to `stage-strip` **immediately when Stage Manager is toggled**.
But if `stage-strip` is not yet in `Running` state (it is launched on first toggle, not
at boot), these messages are sent before the app can receive them. The router's queue
buffers them, but the spec does not state:

1. Whether `stage-strip` is launched at supervisor boot or on first toggle.
2. How the enable-sequence messages are replayed after `stage-strip` reaches `Running`.
3. Whether `strip_init` is idempotent — if `stage-strip` crashes and restarts, does it
   re-receive the stage list?

Without these answers, Stage Manager may silently fail to render on the first toggle if
`stage-strip` has not yet initialized, with no error surfaced to the user.

The R24 spec (DockRouter) solved this explicitly via `on_dock_running()` which calls
`flush_queue()` on the VecDeque. The R25 `McRouter` pattern is identical. The stage strip
is missing the equivalent.

### Proposed Fix

Define the `StageStripRouter` lifecycle callbacks explicitly:

```rust
// supervisor/src/stage_strip_router.rs

impl StageStripRouter {
    /// Called when stage-strip WASM app transitions to Running.
    /// Sends a synthetic strip_init message first, then flushes the buffered queue.
    pub fn on_strip_running(&mut self, registry: &StageRegistry, space: u8,
                             strip_tx: &Sender<String>) {
        // Synthesize strip_init from current registry state (idempotent: always full rebuild)
        if let Some(ss) = registry.spaces.get(&space) {
            let json = build_strip_init_json(ss);
            let msg = format!("VYOMA_STAGE:strip_init:{}\n", json);
            strip_tx.send(msg).ok();
        }
        // Flush any queued events that arrived during startup
        self.strip_running = true;
        while let Some(msg) = self.queue.pop_front() {
            strip_tx.send(msg).ok();
        }
    }

    /// Called when stage-strip is not running (Suspended / Launching).
    /// Enqueue events; strip_init will be re-synthesized on_strip_running.
    pub fn on_strip_not_running(&mut self) {
        self.strip_running = false;
    }

    /// Called when stage-strip terminates with no restart policy.
    /// Clear queue; Stage Manager gracefully degrades (no strip rendered,
    /// stage switching still works for supervisor-side geometry).
    pub fn on_strip_terminated_no_restart(&mut self) {
        self.strip_running = false;
        self.queue.clear();
    }

    /// Strip is launched at supervisor boot (always = true restart policy),
    /// but its surface starts at 0×0 (inactive) until Stage Manager is first enabled.
    /// This avoids the not-running race entirely.
}
```

Additionally, `stage-strip` must be launched at **supervisor boot** (not on first toggle)
with `win_w = 0` / `win_h = 0` surface (0×0 = not visible). On Stage Manager enable,
the supervisor sends `VYOMA_STAGE:enabled` which triggers the strip to resize its surface
to `140 × strip_h`. This mirrors how `mission-control` launches at boot with 0×0 and
expands on trigger (R25 section 2.3).

---

## B5: `blit_clipped` Left-Edge Step 0 Is Incomplete — Width Clipping Not Propagated to Height Steps

### Problem

Section 17.1 introduces a "Step 0" for left-edge clipping of space-N apps that overlaps
the stage strip region. The spec clips `blittable_w_after_left_clip` and `effective_dest_x`
but then states "Remainder of steps 1-6 proceed with effective_dest_x and
blittable_w_after_left_clip."

The existing steps 1-6 (from R24 B3 fix) are a **vertical-only** clip formula. Steps 1-6
compute `blittable_h_combined` based only on Y coordinates (`chrome_h_px`, `dock_clip_y_px`,
`win_y_px`, `src_y_offset`). They do not reference `blittable_w_after_left_clip` at all.
The call to `blit_surface` at the end of `blit_clipped` takes `(win_x_px, effective_dest_y,
surface_width_px, blittable_h_combined, src_y_offset)`. Adding `effective_dest_x` and
`blittable_w_after_left_clip` requires **also passing `src_x_offset`** to `blit_surface`,
and `blit_surface` must be updated to accept and apply horizontal source offset.

The spec does not update the `blit_surface` function signature. The R24 spec defines
`blit_surface(fb, &snap.surface, win_x_px, effective_dest_y, snap.surface_width_px,
blittable_h_combined, src_y_offset)`. A new `src_x_offset` parameter is silently required
but never mentioned. If `blit_surface` is called with `effective_dest_x` as the X position
but `snap.surface_width_px` (un-clipped width) as the width, and `src_x_offset = 0`, the
function will attempt to blit pixels that start from column 0 of the surface into column
`effective_dest_x` of the framebuffer — writing up to `surface_width_px` pixels starting
from `effective_dest_x`, which overflows the framebuffer's right edge for any app whose
surface is wider than `logical_w - strip_w`.

### Proposed Fix

Update the full `blit_clipped` signature and `blit_surface` call to carry horizontal clip
parameters explicitly:

```rust
// supervisor/src/compositor.rs — blit_clipped() complete 7-step formula

fn blit_clipped(fb: &mut FrameBuffer, snap: &AppSnapshot, config: &DisplayConfig,
                chrome_h_px: u32, dock_clip_y_px: u32, strip_clip_x_px: u32)
{
    let win_y_px = snap.win_y_pts.saturating_mul(config.scale_factor as u32);
    let win_x_px = snap.win_x_pts.saturating_mul(config.scale_factor as u32);

    // --- Horizontal clip (Step 0 — new for R26) ---
    let effective_dest_x = win_x_px.max(strip_clip_x_px);
    let src_x_offset = effective_dest_x.saturating_sub(win_x_px);
    let blittable_w = snap.surface_width_px.saturating_sub(src_x_offset);
    // Right-edge clip: do not exceed framebuffer width
    let right_overflow = (effective_dest_x + blittable_w)
        .saturating_sub(config.physical_width);
    let blittable_w_clipped = blittable_w.saturating_sub(right_overflow);

    if blittable_w_clipped == 0 { return; }

    // --- Vertical clip (Steps 1-6 from R24 B3, unchanged) ---
    let effective_dest_y = if snap.is_system_chrome {
        win_y_px
    } else {
        win_y_px.max(chrome_h_px)
    };
    let src_y_offset = effective_dest_y.saturating_sub(win_y_px);
    let blittable_h_after_top_clip = snap.surface_height_px.saturating_sub(src_y_offset);
    let blittable_bottom = (effective_dest_y as i64) + (blittable_h_after_top_clip as i64);
    let clip_bottom = if snap.is_system_chrome {
        config.physical_height as i64
    } else {
        dock_clip_y_px as i64
    };
    let clipped_bottom = blittable_bottom.min(clip_bottom);
    let blittable_h_combined = (clipped_bottom - effective_dest_y as i64).max(0) as u32;

    if blittable_h_combined == 0 { return; }

    // --- Blit (updated signature) ---
    blit_surface(
        fb, &snap.surface,
        effective_dest_x,       // dest X (after left clip)
        effective_dest_y,       // dest Y (after top clip)
        src_x_offset,           // source X skip (NEW)
        src_y_offset,           // source Y skip
        blittable_w_clipped,    // clipped width (NEW, replaces surface_width_px)
        blittable_h_combined,   // clipped height
        snap.anim_alpha,        // per-app alpha override (R26 animation)
    );
}
```

`blit_surface` is updated to accept `src_x_offset: u32` and `blittable_w: u32`,
iterating columns from `src_x_offset` to `src_x_offset + blittable_w` (with bounds
check against `surface_width_px`). This is a pure mechanical addition — no logic change
beyond the loop bounds. Document this signature change as normative for all future rounds.
