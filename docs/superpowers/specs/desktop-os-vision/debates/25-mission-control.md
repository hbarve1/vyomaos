# Architect Spec: Mission Control & Exposé (Round 25)

**Subsystem**: Mission Control & Exposé  
**macOS Analogue**: Mission Control (F3 / Ctrl-Up global overview), Exposé (App Windows per-app scatter)  
**Depends on**: R21 (WM spaces, PerSpaceZ, PENDING_SPACE_SWITCH), R22 (lifecycle, AppState), R23 (chrome, y-clip, 6-step blit), R24 (dock, Model A compositor, blit_clipped)  
**Status**: ARCHITECT DRAFT — critique pending

---

## 1. Overview

Mission Control provides a bird's-eye view of all virtual desktop Spaces and their
windows, allowing the user to navigate between spaces, drag windows between spaces
(deferred to R32), and choose a window to bring to focus. Exposé ("App Windows") is a
subset mode showing only the windows of the currently focused app.

Both modes are implemented as a **privileged WASM app** (`mission-control`) running in
space=0 (always on top, per R21/R24 Model A). This is the same architectural pattern as
`chrome` and `dock`. The app can expand its surface to full-screen during the overview,
then collapse back when dismissed.

### Design Constraints (inherited from prior rounds)

- space=0 apps are NEVER affected by space-switch events (R21)
- Only `chrome` and `dock` may be space=0 (R21 invariant). This round adds `mission-control`
  as the third permitted space=0 occupant. The supervisor assertion is updated accordingly.
- Model A compositor: Pass 1 = space-N, Pass 2 = space-0 (by z ascending)
- Z-order for space-0: chrome=65535, dock=65534, mission-control=65533 (new, inactive=0, visible only when active)
- `vsync_lock: Arc<RwLock<()>>` governs all surface mutations
- `pending_resize: Option<(w,h)>` drains between frames under `vsync_lock.write()`
- 500-line .rs file limit — multiple submodules required

---

## 2. Trigger Mechanism

### 2.1 F3 / Ctrl-Up Keyboard Shortcut

Mission Control is triggered by:
1. **F3** key press (primary trigger, macOS convention)
2. **Ctrl-Up** chord (secondary trigger)

Both triggers are intercepted in the supervisor's TTY input thread, identical to the
Ctrl-Tab interception for the dock switcher (R24 Section 5).

```rust
// supervisor/src/input.rs — extended key event routing

fn route_keyboard(state: &InputState, ev: KeyEvent) {
    // Mission Control triggers: check before forwarding to focused app
    if ev.key == Key::F3 {
        send_mc_trigger(&state.mc_router, MissionControlTrigger::MissionControl);
        return;
    }
    if ev.key == Key::Up && ev.ctrl_held && !ev.alt_held {
        send_mc_trigger(&state.mc_router, MissionControlTrigger::MissionControl);
        return;
    }
    // Exposé (App Windows) trigger: Ctrl-Down or Alt-F3
    if (ev.key == Key::Down && ev.ctrl_held) || (ev.key == Key::F3 && ev.alt_held) {
        send_mc_trigger(&state.mc_router, MissionControlTrigger::AppWindows);
        return;
    }
    // Input lock: if MC is active, arrow keys + Enter/Escape route to mc app
    if state.mc_input_locked.load(Ordering::Acquire) {
        send_to_app("mission-control", ev);
        return;
    }
    // Normal chrome-input-lock check (R23)
    if state.chrome_input_locked.load(Ordering::Acquire) {
        send_to_app("chrome", ev);
        return;
    }
    send_to_focused_app(&state.focused_app.load(), ev);
}
```

### 2.2 Supervisor → MC App Routing

The supervisor sends MC triggers via the `MCRouter` (Section 10). The trigger arrives
on the `mission-control` app's stdin:

```
VYOMA_MC:enter:mission_control
VYOMA_MC:enter:app_windows
```

Pressing the trigger again while MC is active sends:
```
VYOMA_MC:toggle_off
```

The escape key (and Enter-to-select) while `mc_input_locked` is true are forwarded
to the mission-control app directly.

### 2.3 `mc_input_locked: Arc<AtomicBool>`

Added to `SupervisorState`, mirrors `chrome_input_locked` (R23). Set to `true` when
MC enters active state; cleared when MC exits. The TTY input thread checks this flag
using the same `Ordering::Acquire` pattern as `chrome_input_locked`.

Auto-cleared when `mission-control` lifecycle transitions away from Running (identical
to `on_chrome_lifecycle_change`).

---

## 3. Architecture Choice — Mission Control as space=0 WASM App

### 3.1 Decision: WASM App (not supervisor-drawn)

**Rejected alternative A**: Supervisor-drawn overlay (draw directly in `flush_pass`).  
This would bloat `compositor.rs` beyond the 500-line limit and couple display logic with
supervisor core. The chrome/dock precedent demonstrates that privileged space-0 apps
are the correct pattern for complex overlays.

**Rejected alternative B**: A separate space=10 reserved for MC.  
Space 10 would require relaxing the 9-space max and special-casing the compositor;
space=0 already provides "always on top" with no new mechanism.

**Chosen: space=0 WASM app**, identical pattern to chrome and dock.

```toml
# apps/mission-control/vyoma.toml
[app]
name    = "mission-control"
version = "1.0.0"
wasm    = "mission-control.wasm"

[capabilities]
display = true
stdio   = true
shell   = true    # @supervisor: space <N>, focus <app>, mc_exit
```

`win_space = 0` is enforced by the supervisor. The supervisor assertion that guards the
space=0 invariant is updated:

```rust
// supervisor/src/wm/spaces.rs — updated assertion
assert!(
    app.win_space != 0
        || app.name == "chrome"
        || app.name == "dock"
        || app.name == "mission-control",
    "only chrome, dock, mission-control may use space=0"
);
```

### 3.2 Inactive Z-Index (B1 concern noted — see Critique)

When Mission Control is **inactive** (not in overview mode), it keeps its surface at
a minimal 1×1 pixel and sets `win_visible = false`. It is still space=0 and still
lives in the apps_sorted list, but `blit_clipped` skips zero-height or invisible
surfaces. This avoids any compositor impact during normal desktop use.

When MC becomes **active**, it sends:
```
VYOMA_DRAW:resize_surface:<logical_w>,<logical_h>
```
expanding to full screen, then draws the overview and flushes. The `pending_resize`
pattern drains this between compositor frames exactly as dock's switcher overlay does.

---

## 4. Window Thumbnail Generation

### 4.1 Thumbnail Source: Surface Snapshot

Each app's current `Surface` buffer is the thumbnail source. The MC app requests a
snapshot via a new `VYOMA_MC:capture_windows` IPC response from the supervisor.

The supervisor captures a snapshot of all visible app surfaces under `vsync_lock.read()`
to prevent tearing during copy. This read-lock acquisition is safe because:
- The compositor holds `vsync_lock.write()` only briefly for DRM blit (Step 4 in R21)
- Thumbnail capture happens in the MC app's request-processing path (not in the compositor thread)
- The snapshot is a point-in-time copy; it does not need to stay fresh during the overview

### 4.2 Thumbnail Data Protocol

The supervisor serializes thumbnails as raw BGRA data and delivers them via a new
mechanism: **shared memory pages** accessible to the mission-control WASM app via the
filesystem capability (`/tmp/mc_thumbnails/`).

Each thumbnail is written as `/tmp/mc_thumbnails/<app_name>.raw` with a header line:

```
<width_px>,<height_px>\n<BGRA bytes>
```

The supervisor writes these files under a fresh `vsync_lock.read()` per app, releasing
the lock between writes to avoid long read-lock holds that could stall the compositor.

The mission-control app reads these files after receiving the `VYOMA_MC:windows_ready`
notification.

### 4.3 Supervisor Capture Sequence

```
1. MC app sends: @supervisor: mc_capture_request
2. Supervisor receives request, marks capture_pending = true
3. On next compositor tick, after DRM blit (while vsync_lock is not held):
   - For each visible space-N app in active space:
     - Acquire vsync_lock.read()
     - Memcopy app.surface.data to a temporary Vec<u8>
     - Release vsync_lock.read()
     - Write Vec<u8> to /tmp/mc_thumbnails/<app_name>.raw
4. Write /tmp/mc_thumbnails/manifest.txt listing captured apps + their geometry
5. Send VYOMA_MC:windows_ready:<count> to MC app stdin
6. capture_pending = false
```

### 4.4 Thumbnail Scale-Down

The supervisor scales thumbnails down to fit within a 320×200pt bounding box during
the write step using a simple box-filter downsample. This keeps the raw files small
and reduces time the `vsync_lock.read()` is held.

```rust
// supervisor/src/mc_capture.rs

pub fn scale_down_surface(src: &[u8], src_w: u32, src_h: u32,
                           max_w: u32, max_h: u32) -> (Vec<u8>, u32, u32) {
    let scale_x = (src_w + max_w - 1) / max_w;   // integer ceil
    let scale_y = (src_h + max_h - 1) / max_h;
    let scale   = scale_x.max(scale_y).max(1);
    let dst_w   = src_w / scale;
    let dst_h   = src_h / scale;
    let mut dst = vec![0u8; (dst_w * dst_h * 4) as usize];
    for dy in 0..dst_h {
        for dx in 0..dst_w {
            let sx = dx * scale;
            let sy = dy * scale;
            let idx = ((sy * src_w + sx) * 4) as usize;
            let odx = ((dy * dst_w + dx) * 4) as usize;
            dst[odx..odx+4].copy_from_slice(&src[idx..idx+4]);
        }
    }
    (dst, dst_w, dst_h)
}
```

Note: This is a nearest-neighbour downsample, not true box-filter — acceptable for
thumbnail preview quality.

---

## 5. Layout Algorithm

### 5.1 Thumbnail Grid Layout

The Mission Control overview displays two regions:
1. **Space Strip** (top, 80pt height): one slot per space
2. **Window Grid** (remaining area, below space strip, above dock): thumbnails of windows in the selected space

```
+--------------------------------------------------+
| [Space 1] [Space 2] [Space 3]  [+Add Space]      |  80pt
+--------------------------------------------------+
|                                                  |
|  +----------+  +----------+  +----------+        |
|  | App A    |  | App B    |  | App C    |        |
|  | thumb    |  | thumb    |  | thumb    |        |
|  +----------+  +----------+  +----------+        |
|                                                  |
+--------------------------------------------------+
| [Dock strip - always visible]                    |  48pt
+--------------------------------------------------+
```

Available height for window grid:
```
grid_h = logical_h - CHROME_HEIGHT_PTS - 80 - 48
       = logical_h - 24 - 80 - 48
       = logical_h - 152
```

### 5.2 Grid Packing Algorithm

Given N windows, the grid packs them into rows and columns to maximise thumbnail size
while maintaining aspect ratio:

```rust
// apps/mission-control/src/layout.rs

pub struct ThumbRect {
    pub x: u32, pub y: u32,
    pub w: u32, pub h: u32,
    pub app_name: String,
}

pub fn compute_grid_layout(
    windows: &[WindowInfo],
    grid_x: u32, grid_y: u32,   // top-left of available grid area
    grid_w: u32, grid_h: u32,   // available area (logical pts)
    padding: u32,                // gap between thumbnails (default 16pt)
) -> Vec<ThumbRect> {
    let n = windows.len();
    if n == 0 { return vec![]; }

    // Find best (cols, rows) pair that maximises thumb area
    let mut best_thumb_w = 0u32;
    let mut best_cols = 1usize;
    for cols in 1..=n {
        let rows = (n + cols - 1) / cols;
        let thumb_w = (grid_w.saturating_sub(padding * (cols as u32 + 1))) / cols as u32;
        let thumb_h = (grid_h.saturating_sub(padding * (rows as u32 + 1))) / rows as u32;
        // Constrain to 16:10 max aspect ratio
        let thumb_w = thumb_w.min(thumb_h * 16 / 10);
        if thumb_w > best_thumb_w {
            best_thumb_w = thumb_w;
            best_cols = cols;
        }
    }

    let best_rows = (n + best_cols - 1) / best_cols;
    let thumb_h = (grid_h.saturating_sub(padding * (best_rows as u32 + 1))) / best_rows as u32;
    let thumb_w = best_thumb_w;

    // Center the grid block horizontally
    let total_row_w = best_cols as u32 * thumb_w + (best_cols as u32 + 1) * padding;
    let x_offset = grid_x + grid_w.saturating_sub(total_row_w) / 2;

    let mut rects = Vec::with_capacity(n);
    for (i, win) in windows.iter().enumerate() {
        let col = i % best_cols;
        let row = i / best_cols;
        let x = x_offset + padding + col as u32 * (thumb_w + padding);
        let y = grid_y + padding + row as u32 * (thumb_h + padding);
        rects.push(ThumbRect { x, y, w: thumb_w, h: thumb_h, app_name: win.app_name.clone() });
    }
    rects
}
```

### 5.3 Thumbnail Rendering

The MC app draws each thumbnail using VYOMA_DRAW commands:
1. `fill_rect` with the thumbnail background (dark border)
2. For each captured thumbnail image: draw as a BGRA blit (new `draw_image` command — Section 13)
3. `draw_text` below each thumbnail for the app name (size `s` = 4×8)
4. Selected thumbnail: `fill_rect` with highlight border color

### 5.4 Animation

Animation is achieved via a simple alpha fade triggered on entry/exit:

**Entry animation** (enter keyframe sequence, ~200ms):
```
Frame 0: MC overlay at alpha=0 (transparent)
Frame 1: alpha=64
Frame 2: alpha=128
Frame 3: alpha=192
Frame 4: alpha=255 (fully opaque)
```

The MC app adjusts the global_alpha of its VYOMA_DRAW fill rects on each redraw step.
The supervisor's existing `global_alpha` field in AppState (R11) governs how `blit_surface`
composites the surface.

**Exit animation**: reverse sequence, then collapse surface.

Animation frames are driven by the MC app's own event loop iterating until animation
completes. Each frame: draw + flush + sleep ~40ms (giving ~5 frames at ~25fps).

---

## 6. Space Strip

### 6.1 Space Strip Layout

The space strip occupies the top 80pt of the MC overlay (below the chrome bar):

```
y range: CHROME_HEIGHT_PTS .. CHROME_HEIGHT_PTS + 80  (24..104 logical pts)
```

Each space slot is a thumbnail of the space's primary visible window (or a blank
rectangle if the space is empty). Slots are:
- Width: `(logical_w - 2 * STRIP_PADDING) / max(count, 1)`, max 200pt each
- Height: `STRIP_SLOT_H = 60pt` (inside 80pt strip, 10pt padding above/below)
- Selected space: bright highlight border (4pt)
- Current active space: filled indicator dot below slot label

### 6.2 Space Operations from MC

The MC app issues supervisor IPC to manage spaces:

```
@supervisor: space_add              → add new space (supervisor: SpaceRegistry::add)
@supervisor: space_del <N>          → delete space N
@supervisor: space <N>              → switch to space N (uses PENDING_SPACE_SWITCH)
```

The supervisor routes these via the existing WM command handler.

When the user selects a space slot in the space strip (via arrow key + Enter):
1. MC app sends `@supervisor: space <N>`
2. MC app sends `@supervisor: mc_exit` (exit overview)
3. Supervisor sets `mc_input_locked = false`, sets `mission-control win_visible = false`

### 6.3 Space Strip Thumbnails

Space thumbnails are generated via the same `mc_capture_request` flow, but with an
additional parameter:

```
@supervisor: mc_capture_request_all_spaces
```

This captures all spaces (not just the active one). For non-active spaces, the supervisor
reads the last-known surface state of apps in those spaces. Apps in non-active spaces
may be `Suspended` (no new frames), so their surface contains the last rendered frame.

### 6.4 Adding a New Space

The `[+Add Space]` button in the space strip is navigable via arrow keys. Selecting it
triggers `@supervisor: space_add` and the MC app redraws the space strip after receiving
`VYOMA_MC:space_added:<N>` from the supervisor.

---

## 7. Exposé Mode (App Windows)

### 7.1 Entry

App Windows Exposé shows only the windows belonging to the currently focused app. This
is activated by Ctrl-Down or Alt-F3.

The trigger sends `VYOMA_MC:enter:app_windows` to the MC app's stdin. The MC app
queries the focused app name by issuing:

```
@supervisor: mc_focused_app_query
```

Supervisor responds on MC's stdin:

```
VYOMA_MC:focused_app:<app_name>
```

### 7.2 Layout in Exposé Mode

In App Windows mode, the space strip is **hidden** (height reduced to 0). All available
height (logical_h - CHROME_HEIGHT_PTS - 48) is used for the window grid.

The window list shows only windows for the queried app. Since the current architecture
supports one window per app (R21), the typical case is exactly one thumbnail shown
centered on screen with the app name below.

Future multi-window support (when an app can spawn child windows) naturally extends
to showing multiple thumbnails for the same app.

### 7.3 Dismissal

Escape: exit Exposé without switching focus. Existing focused app retains focus.
Enter: focus the selected window (same as Mission Control exit with focus change).

---

## 8. Input While Active

### 8.1 Keyboard Navigation

When `mc_input_locked = true`, all non-trigger keyboard events are sent to the
`mission-control` app. The MC app handles:

| Key | Action |
|-----|--------|
| Arrow Left/Right | Move selection left/right within the grid |
| Arrow Up/Down | Move selection up/down between rows in grid, or into/out of space strip |
| Enter / Space | Select hovered thumbnail → focus window and exit MC |
| Escape | Exit MC without changing focus |
| F3 / Ctrl-Up | Toggle off MC (same as Escape) |
| Tab | Cycle to next thumbnail |
| Shift-Tab | Cycle to previous thumbnail |
| Delete | (Space strip only) Delete the selected space |

The MC app maintains `selected_index: usize` in its internal state. Arrow key moves
adjust this index and trigger a redraw (no surface resize needed for selection changes —
only highlight color changes).

### 8.2 Mouse (deferred to R32)

Mouse hover and click within the MC overlay are deferred to R32 (Mouse & Gestures).
The MC app does not register for mouse events in this round. Click-to-select and
drag-to-rearrange spaces are R32 features.

---

## 9. Transition To/From Normal Compositing

### 9.1 Entry Transition

1. User presses F3 (or Ctrl-Up)
2. Supervisor TTY thread routes trigger to `mc_router`; sets `mc_input_locked = true`
3. `mc_router` delivers `VYOMA_MC:enter:mission_control` to MC app stdin
4. MC app issues `VYOMA_DRAW:resize_surface:<logical_w>,<logical_h>` (pending_resize queued)
5. MC app issues `@supervisor: mc_capture_request` (supervisor schedules thumbnail capture)
6. On next compositor tick:
   a. `pending_resize` drained → MC surface expanded to full screen
   b. `mc_surface_ready = true` (set on first flush after resize)
   c. Compositor Pass 2 now blits MC surface on top of everything (Model A, z=65533 but visible)

Between steps 3 and 5, there is a brief window where MC surface is being resized but
thumbnails are not yet drawn. The MC app draws a plain dark background immediately
after resize, before the thumbnails arrive:

```
1. resize_surface → full screen
2. fill_rect (0,0,full,full) dark overlay   ← immediate
3. flush                                     ← commit dark overlay
4. @supervisor: mc_capture_request
5. (await VYOMA_MC:windows_ready)
6. draw thumbnails
7. flush
```

### 9.2 Exit Transition

1. User presses Escape (or selects a window)
2. MC app:
   a. (Optional) Sends focus command: `@supervisor: focus <selected_app>`
   b. Runs exit animation (alpha fade out, ~200ms, 5 frames)
   c. After animation completes: `VYOMA_DRAW:resize_surface:1,1` (collapse to minimal)
   d. `VYOMA_DRAW:flush`
   e. Sends `@supervisor: mc_exit`
3. Supervisor receives `mc_exit`:
   a. Sets `mc_input_locked = false`
   b. Sets `mission-control.win_visible = false`
4. Compositor: on next tick, MC app's 1×1 surface is blitted (invisible effect)
5. Normal compositor pass resumes (space-N apps fully visible again)

### 9.3 Normal Mode: MC Surface is Invisible

During normal desktop use:
- `mission-control.win_visible = false`
- MC surface size: 1×1 px (minimal allocation)
- `blit_clipped` skips apps with `win_visible = false`
- No visual impact on frame rate

---

## 10. Interaction with Dock

### 10.1 Dock Remains Visible During MC

The dock (z=65534 in space-0) is blitted after space-N apps but before chrome (z=65535)
in Pass 2. Mission-control (z=65533) is blitted before dock in Pass 2. Therefore:

Pass 2 blit order: mission-control(65533) → dock(65534) → chrome(65535)

The dock is **always visible** during MC because it is blitted after MC in the z-order.
The MC overlay's dark background extends to `y = logical_h - 48` (not to the bottom),
leaving the dock visible.

Specifically, the MC app's fill_rect for the background:
```
VYOMA_DRAW:fill_rect:0,24,<logical_w>,<logical_h - 24 - 48>,<DARK_OVERLAY_COLOR>
```

This leaves the 48pt dock region undrawn by MC, so the dock's own surface shows through.

### 10.2 Upswipe Gesture Trigger (deferred to R32)

In macOS, a three-finger upswipe on the trackpad opens Mission Control. This is deferred
to R32 (Mouse & Gestures). In R32, the gesture recogniser will send `VYOMA_MC:enter:mission_control`
via the same `mc_router` channel.

### 10.3 Chrome Remains on Top

Chrome (z=65535) is always blitted last in Pass 2, on top of MC. The MC overlay does
not draw into the top 24pt chrome region. The MC overlay's content starts at y=24pt.

---

## 11. Exit Action

### 11.1 Window Selection

When the user selects a thumbnail (Enter key):
1. MC app notes the `app_name` of the selected thumbnail
2. MC app sends: `@supervisor: focus <app_name>`
3. MC app sends: `@supervisor: space <space_N>` if the selected window is in a
   different space than the current active space
4. MC app runs exit animation
5. MC app collapses surface, sends `@supervisor: mc_exit`

Supervisor handles `focus <app_name>` via the existing WM focus handler (R21).
If `space <N>` is also sent, PENDING_SPACE_SWITCH is set atomically (R21).

### 11.2 Space Selection

When the user selects a space in the space strip (Enter key):
1. MC app sends: `@supervisor: space <N>`
2. MC app exits (no window focus change within the new space — the previously focused
   app in that space retains focus per R21 space-switch logic)

### 11.3 No Selection (Escape)

MC exits without any focus or space change. `mc_input_locked = false`. The compositor
continues rendering the same active space and focused app.

---

## 12. Platform Matrix

| Feature | desktop-full | mobile | server-headless | iot-edge | robotics-rt | mcu-minimal |
|---------|-------------|--------|----------------|---------|------------|------------|
| Mission Control | yes | partial (see below) | no | no | no | no |
| Exposé (App Windows) | yes | yes (simplified) | no | no | no | no |
| Space strip | yes | no (single space) | no | no | no | no |
| Space creation/deletion | yes | no | no | no | no | no |
| F3 / Ctrl-Up trigger | yes | no (swipe gesture R32) | no | no | no | no |
| Thumbnail capture | yes | yes (single app) | no | no | no | no |
| `mission-control` WASM app | yes | yes | no | no | no | no |

### 12.1 Mobile Mission Control

On mobile (ARM64 tablet/phone profile), there is one space and windows are full-screen.
Mission Control maps to an **App Switcher** view: a horizontal strip of app thumbnails
(similar to iOS App Switcher). Triggered by platform-specific gesture (deferred R32).

The `mission-control` WASM app runs on mobile but in a simplified configuration:
- No space strip (single space, no add/delete)
- App thumbnails shown as a horizontal scrollable row
- Triggered by `VYOMA_MC:enter:app_switcher` (separate trigger type)

The same WASM binary handles all trigger types via the `enter` message parameter.

---

## 13. New VYOMA_DRAW:draw_image Command

Mission Control requires blitting pre-rendered BGRA thumbnail data, which cannot be
done with existing `fill_rect` / `draw_text` commands. A new draw command is added:

```
VYOMA_DRAW:draw_image:<x>,<y>,<w>,<h>,<path>
```

Where `<path>` is a `/tmp/mc_thumbnails/<app_name>.raw` path. The supervisor loads
the raw BGRA bytes from the path and blits them into the app's Surface at `(x, y)`.

```rust
// supervisor/src/draw_cmd.rs — new variant

DrawCmd::DrawImage { x, y, w, h, path }

fn handle_draw_image(surface: &mut Surface, cmd: &DrawCmd, config: &DisplayConfig) {
    let DrawCmd::DrawImage { x, y, w, h, path } = cmd else { return; };
    // Read raw BGRA file
    let raw = match std::fs::read(path) {
        Ok(data) => data,
        Err(_) => return,  // silently skip missing thumbnail
    };
    let expected = (w * h * 4) as usize + /* header */ 32;
    if raw.len() < (w * h * 4) as usize { return; }  // malformed
    // Blit raw BGRA into surface at (x*sf, y*sf)
    let sf = config.scale_factor as u32;
    blit_raw_bgra(surface, &raw[..( w * h * 4) as usize],
                  *x * sf, *y * sf, *w * sf, *h * sf, config.physical_width);
}
```

**Security note**: `path` is restricted to `/tmp/mc_thumbnails/` prefix by the supervisor.
Any path not under this directory is rejected and logged. Only the `mission-control` app
may issue `draw_image` commands (capability gate: `mc_display = true` in vyoma.toml).

---

## 14. `mc_surface_ready` and Fallback Fill

Following the same pattern as `chrome_surface_ready` (R23) and `dock_surface_ready` (R24):

```rust
// supervisor/src/compositor.rs — added to CompositorState

pub mc_surface_ready: bool,   // set true on MC app's first flush; false at boot and on MC exit
```

When MC is active but `mc_surface_ready = false` (MC is initializing):
- No fallback fill is drawn (unlike chrome/dock which have permanent real-estate)
- The space-N apps below remain visible until MC's surface is first flushed
- This provides a natural "apps still visible while MC loads" effect

`mc_surface_ready` is reset to `false` when MC collapses its surface (sends `mc_exit`).

---

## 15. `MCRouter` — Message Routing to Mission Control App

```rust
// supervisor/src/mc_router.rs

pub struct MCRouter {
    queue: VecDeque<String>,
    mc_running: bool,
}

impl MCRouter {
    // Identical API to ChromeRouter and DockRouter (R23, R24)
    pub fn route(&mut self, msg: String, mc_tx: &Sender<String>) { /* ... */ }
    pub fn on_mc_running(&mut self, mc_tx: &Sender<String>) { /* ... */ }
    pub fn on_mc_not_running(&mut self) { /* ... */ }
    pub fn on_mc_terminated_no_restart(&mut self) { /* queue.clear(); mc_running = false; */ }
}
```

Drop-oldest ring buffer capacity: 64. Same overflow semantics as `ChromeRouter`.

Events routed to MC app stdin:

```
VYOMA_MC:enter:<mode>            ← mode = mission_control | app_windows | app_switcher
VYOMA_MC:toggle_off              ← second F3 press while active
VYOMA_MC:windows_ready:<count>   ← thumbnails written to /tmp/mc_thumbnails/
VYOMA_MC:focused_app:<name>      ← response to mc_focused_app_query
VYOMA_MC:space_changed:<N>       ← active space changed (triggered by PENDING_SPACE_SWITCH)
VYOMA_MC:space_added:<N>         ← new space added
VYOMA_MC:space_deleted:<N>       ← space deleted
VYOMA_MC:app_launched:<name>     ← for live updates while MC is open
VYOMA_MC:app_exited:<name>       ← for live updates while MC is open
```

---

## 16. VYOMA_MC: Stdout Protocol (App → Supervisor)

```
@supervisor: mc_capture_request
@supervisor: mc_capture_request_all_spaces
@supervisor: mc_focused_app_query
@supervisor: mc_exit
@supervisor: focus <app_name>
@supervisor: space <N>
@supervisor: space_add
@supervisor: space_del <N>
```

These are handled by the existing supervisor IPC handler (`ipc_handlers.rs`). The `focus`,
`space`, `space_add`, `space_del` commands are already defined in R21/R23/R24. New commands
are `mc_capture_request`, `mc_capture_request_all_spaces`, `mc_focused_app_query`, `mc_exit`.

---

## 17. WIT Interface `vyoma:mission-control@1.0.0`

```wit
package vyoma:mission-control@1.0.0;

record window-thumbnail {
    app-name: string,
    x: u32, y: u32,          // logical points in MC overlay
    thumb-w: u32, thumb-h: u32,
    win-title: string,
    space: u8,
    focused: bool,
}

enum mc-trigger {
    mission-control,
    app-windows,
    app-switcher,
}

interface mission-control {
    /// Request thumbnail capture of all windows in active space.
    /// Returns path prefix where thumbnails are written.
    request-capture: func(all-spaces: bool) -> result<string, string>;

    /// Get current window list with geometry (for layout computation).
    list-windows: func(space: option<u8>) -> result<list<window-thumbnail>, string>;

    /// Enter mission control mode — lock input to this app.
    enter-mc: func(trigger: mc-trigger) -> result<_, string>;

    /// Exit mission control mode — unlock input.
    exit-mc: func() -> result<_, string>;

    /// Get the currently focused app name.
    focused-app: func() -> result<string, string>;
}

world mission-control-world {
    import mission-control;
}
```

Capability gate: only the app named `"mission-control"` in boot.toml can call `enter-mc`
and `exit-mc`. `list-windows` and `request-capture` are also restricted to this app.

---

## 18. Compositor Changes for MC

The compositor's `flush_pass` is updated to include `mc_surface_ready` in the fallback
fill logic and to handle `mission-control` in the clip bypass list:

```rust
// supervisor/src/compositor.rs — flush_pass()

fn flush_pass(fb: &mut FrameBuffer, apps_sorted: &[AppSnapshot], config: &DisplayConfig,
              chrome_surface_ready: bool, dock_surface_ready: bool, _mc_surface_ready: bool)
{
    // ... (chrome and dock fallback fills unchanged from R24) ...

    // Pass 1: active-space apps (space-N)
    let mut space_n: Vec<_> = apps_sorted.iter()
        .filter(|a| a.win_space != 0 && a.win_visible)
        .collect();
    space_n.sort_by_key(|a| a.z_index);
    for snap in &space_n {
        blit_clipped(fb, snap, config, chrome_h_px, dock_clip_y_px);
    }

    // Pass 2: space-0 apps sorted by z ascending: mc(65533) → dock(65534) → chrome(65535)
    let mut space_0: Vec<_> = apps_sorted.iter()
        .filter(|a| a.win_space == 0 && a.win_visible)
        .collect();
    space_0.sort_by_key(|a| a.z_index);
    for snap in &space_0 {
        blit_clipped(fb, snap, config, chrome_h_px, dock_clip_y_px);
    }
}
```

The `blit_clipped` function's clip bypass list is extended:

```rust
// supervisor/src/compositor.rs — blit_clipped()

let bypass_chrome_clip = snap.app_name == "chrome"
    || snap.app_name == "dock"
    || snap.app_name == "mission-control";

let bypass_dock_clip = snap.app_name == "chrome"
    || snap.app_name == "dock"
    || snap.app_name == "mission-control";
```

This allows `mission-control` to draw over the full screen including the chrome and
dock regions when needed (e.g., the dark overlay background fill behind the dock area).

---

## 19. Supervisor Command Handlers (New)

```rust
// supervisor/src/ipc_handlers.rs — new mc-specific commands

fn handle_mc_capture_request(all_spaces: bool, apps: &AppsMap,
                              vsync_lock: &Arc<RwLock<()>>,
                              config: &DisplayConfig,
                              mc_tx: &Sender<String>)
{
    // Spawn a capture task on the supervisor's io thread pool
    // (avoids blocking the main event loop)
    let apps_snapshot = {
        let _r = vsync_lock.read();
        snapshot_all_surfaces(apps, all_spaces)
    };
    // Write thumbnails to /tmp/mc_thumbnails/ (off main thread)
    let count = write_thumbnails(apps_snapshot, config);
    let _ = mc_tx.send(format!("VYOMA_MC:windows_ready:{}\n", count));
}

fn handle_mc_exit(state: &mut SupervisorState) {
    state.mc_input_locked.store(false, Ordering::Release);
    if let Some(mc_app) = state.apps.get_mut("mission-control") {
        mc_app.win_visible = false;
    }
    state.mc_surface_ready = false;
}

fn handle_mc_focused_app_query(state: &SupervisorState) {
    let focused = state.focused_app.load().clone();
    let _ = state.mc_stdin_tx.send(format!("VYOMA_MC:focused_app:{}\n", focused));
}
```

---

## 20. File Layout

```
apps/mission-control/
├── vyoma.toml                (win_space=0, display=true, stdio=true, shell=true)
└── src/
    ├── main.rs               (event loop: stdin parsing, state machine)
    ├── state.rs              (MCState, MCMode, thumbnail registry, selection)
    ├── layout.rs             (compute_grid_layout, space strip layout)
    ├── draw.rs               (render_mc_overlay, render_space_strip, render_grid)
    └── animation.rs          (fade_in, fade_out keyframe sequences)

supervisor/src/
├── mc_router.rs              (MCRouter — identical pattern to ChromeRouter)
├── mc_capture.rs             (snapshot_all_surfaces, write_thumbnails, scale_down_surface)
├── ipc_handlers.rs           (mc_capture_request, mc_exit, mc_focused_app_query handlers)
├── input.rs                  (F3/Ctrl-Up/Ctrl-Down trigger interception; mc_input_locked)
├── compositor.rs             (blit_clipped: bypass for mission-control; mc_surface_ready)
└── draw_cmd.rs               (draw_image command: path-restricted BGRA blit)
```

---

## 21. Invariants & Assertions

1. **space=0 membership**: Only `chrome`, `dock`, `mission-control` may have `win_space = 0`.
2. **MC z-order**: `mission-control.win_z.get(0) = 65533` (fixed, set at startup by supervisor).
3. **`mc_input_locked` = true** implies `mission-control.lifecycle == Running`.
4. **Thumbnail paths**: `draw_image` only accepts paths under `/tmp/mc_thumbnails/`.
5. **MC active implies `win_visible = true`**: `mc_exit` always sets `win_visible = false`.
6. **`mc_surface_ready` lifecycle**: set true on MC's first flush per activation; reset on `mc_exit`.

---

## 22. Blocking Issues (Pre-Critique)

The following issues are expected to be raised in the critique:

1. Thumbnail capture via filesystem (`/tmp/mc_thumbnails/`) exposes raw pixel data to
   the WASM app via the filesystem — requires `filesystem = true` on the MC app, which
   grants broad `/data` access. Potential security concern.
2. `mc_surface_ready = false` with no fallback fill means a blank region during MC load.
3. The `draw_image` command requires the supervisor to read files during the blit path —
   introducing I/O latency in the compositor loop.
4. `blit_clipped` bypass for `mission-control` allows MC to overdraw chrome and dock
   regions — this is intentional but could cause visual glitches if MC exits mid-draw.
5. Capture of all-spaces surfaces for Suspended apps may yield stale or partially-drawn
   thumbnails.

---

*End of Architect Spec. See `25-mission-control-critique.md` for blocking issues.*
