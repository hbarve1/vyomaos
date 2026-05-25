# Research: Interactive Window Management

## Close Button

**Decision**: Close button already sends `SIGKILL` via `libc::kill(pid, SIGKILL)` in `mouse_input.rs:228–236`. The gap is that tiling reflow and focus transfer are not called after the kill.

**Required fix**: After sending `SIGKILL`, call `apply_tiling_layout(app_registry)` and `toast::auto_transfer_focus(&name, app_registry, focused)`. Both functions exist and are used in `app_threads.rs` on natural app exit — the same path works for close-button kills.

**Alternatives considered**: Using SIGTERM for graceful shutdown, but the existing IPC kill command also uses SIGKILL. Consistency takes priority.

---

## Window Drag

**Decision**: A `DRAG_STATE` module-level static in `mouse_input.rs` tracks which app is being dragged, the cursor position at drag start, and the window's `win_region` at drag start. On every `EV_SYN` during `btn_held & 1 != 0`, the delta from `drag_delta(sx, sy, cx, cy)` is added to `win_start`, clamped to screen bounds, and written back to `AppState.win_region`. The existing `MOUSE_DRAG_START` global continues to hold the cursor start position; `DRAG_STATE` holds the richer per-drag context.

**Drag initiation**: At left-button press (`EV_KEY BTN_LEFT, value=1`), before recording `MOUSE_DRAG_START`, check if the cursor is inside any window's title-bar strip _and_ NOT inside a traffic-light hit area. If yes, record that app's name and current `win_region` in `DRAG_STATE`. If a traffic-light is hit instead, `DRAG_STATE` is left `None` so that motion events are not treated as a drag.

**Clamping**: During drag, `win_region.x` is clamped to `[0, screen_w - win_w]`, `win_region.y` to `[MENUBAR_H, screen_h - win_h]`.

**Repaint during drag**: After updating `win_region`, repaint the title bar at the new position. The previous title-bar rows must also be cleared (filled with background colour) before painting the new position. Only the scanline rows of old and new title-bar positions are blitted — `flush_cursor_only()` is NOT sufficient here; a targeted `fill_rect` + `draw_titlebar` + partial `flush()` is used.

**Alternatives considered**: Updating `win_region` only on button release (no live drag) — rejected because SC-001 requires per-frame updates. Storing drag state in a new `mod drag;` submodule — unnecessary complexity given the small size.

---

## Snap-Back to Tiled Grid

**Decision**: Add `pub fn nearest_tiled_slot(pos: (u32,u32), n: usize, sw: u32, sh: u32) -> Option<(u32,u32,u32,u32)>` to `supervisor/src/windows.rs`. It calls `compute_tiling_with_hints(n, sw, sh - MENUBAR_H, &[(0,0);n])`, offsets y by MENUBAR_H (matching `apply_tiling_layout`), then finds the slot whose top-left corner is within 40 px Chebyshev distance of `pos`. Returns `Some(slot)` if within threshold, `None` otherwise.

**Called from**: `mouse_input.rs` on left-button release, after `DRAG_STATE` confirms a drag just ended.

**MENUBAR_H**: The function does not import `chrome::MENUBAR_H` (cross-module dependency). Instead it accepts a `menubar_h: u32` parameter, keeping `windows.rs` dependency-free.

**Alternatives considered**: Chebyshev vs. Euclidean distance — Chebyshev (max of |Δx|, |Δy|) is simpler and works well for axis-aligned grid slots.

---

## Minimize / Restore

**Decision**: Add two fields to `AppState` in `main.rs`:
- `minimized: bool` (default `false`)
- `pre_minimize_region: Option<(u32, u32, u32, u32)>` (default `None`)

**Minimize action**: On yellow-dot click, save `win_region` to `pre_minimize_region`, set `minimized = true`, and compute the strip position: 240×28 at `y = screen_h - 28`, offset by existing minimized-strip count × 240 horizontally.

**Strip count**: On minimize, count apps currently minimized to compute `x = minimized_count * 240`. Strips are placed left-to-right; wrap to a second row at `x >= screen_w` by subtracting `screen_w` from `x` and `y -= 28`.

**Restore action**: Detect a click inside a minimized strip — a 240×28 region at the computed position. On click, set `minimized = false`, restore `win_region = pre_minimize_region`, clear `pre_minimize_region`, repaint title bar at restored position.

**Draw commands while minimized**: Content draw commands continue to be processed (they update the back-buffer) but the clip region shrinks to 240×28; content rows outside the strip are clipped by `draw_cmd.rs` automatically since `win_region` is now the strip. Alternatively, a `minimized` guard can return early from draw commands — the simpler approach (the latter) avoids content painting into the strip.

**Alternatives considered**: Hiding app draw output entirely vs. using the strip as the clip region. The simpler path is to add a `minimized` guard in `draw_cmd::handle_draw_command` that returns without drawing if `st.minimized`. The title-bar and status-strip repaints (from chrome) still run normally on the strip region.

---

## File Layout

| File | Change |
|------|--------|
| `supervisor/src/main.rs` | Add `minimized: bool`, `pre_minimize_region: Option<…>` to `AppState`; initialize both |
| `supervisor/src/mouse_input.rs` | Add `DRAG_STATE` global; implement drag update loop; fix close (add reflow+focus); implement minimize; add strip click detection |
| `supervisor/src/windows.rs` | Add `nearest_tiled_slot(pos, n, sw, sh, menubar_h) -> Option<…>` |
| `supervisor/src/draw_cmd.rs` | Early return on `st.minimized` before processing draw commands |
| `supervisor/tests/drag_test.rs` | Unit tests: snap distance, drag clamping, minimize state |

`chrome.rs`, `display/mod.rs`, and `ipc_handlers.rs` require no changes.
