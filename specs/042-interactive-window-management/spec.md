# Feature Specification: Interactive Window Management

**Feature Branch**: `042-interactive-window-management`

**Created**: 2026-05-25

**Status**: Draft

**Input**: Interactive window management — title-bar drag to reposition, snap-back to tiled grid, functional close button, minimize/restore.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Title-Bar Drag to Move Window (Priority: P1)

The user presses and holds the left mouse button over a window's title bar, then moves the mouse. The window follows the cursor in real time. Releasing the button finalises the new position.

**Why this priority**: The current windowing system tiles windows in a fixed grid with no runtime rearrangement. Drag-to-move is the single most visible gap in the windowing UX and directly completes the interactive desktop story begun in spec-002.

**Independent Test**: Run two display apps. Hold left button on one title bar and move the mouse 100 px to the right. Confirm the window visually follows and settles at the new position. No other app's content should be disturbed.

**Acceptance Scenarios**:

1. **Given** a window at position (x, y), **When** the user presses the left button on its title bar and moves the mouse by (dx, dy), **Then** the window's displayed position updates to (x+dx, y+dy) within one render frame.
2. **Given** a window being dragged toward the screen left edge, **When** the window's left edge would go below x=0, **Then** it is clamped at x=0 so no part of the window falls off-screen.
3. **Given** a window is being dragged, **When** the user releases the left button, **Then** the window stays at the released position and subsequent mouse moves do not affect it.
4. **Given** two windows exist, **When** the user drags window A, **Then** window B's content, title bar, and status strip are unaffected.
5. **Given** a window is being dragged over another window, **When** the drag is in progress, **Then** only the dragged window's title bar is repainted per mouse event — no full-framebuffer redraws of other apps.

---

### User Story 2 - Snap-Back to Tiled Grid Position (Priority: P2)

After dragging a window near a tiled grid slot, releasing the mouse button within 40 px of that slot snaps the window exactly to that position.

**Why this priority**: Free dragging without snap risks messy layouts. Snap-back lets users restore the tiled arrangement without restarting apps, complementing the manual freedom of US1.

**Independent Test**: With four apps in a 2×2 grid, drag one window 30 px off its slot and release. Confirm it snaps back. Then drag it 50 px away and release. Confirm it stays at the dropped position.

**Acceptance Scenarios**:

1. **Given** a window released within 40 px (Chebyshev distance) of a tiled grid slot, **When** the button is released, **Then** the window snaps to the exact tiled slot dimensions.
2. **Given** a window released more than 40 px from every tiled slot, **When** the button is released, **Then** the window remains at the drag-drop position with no snap.
3. **Given** a window snaps to a different slot than its original, **When** the snap occurs, **Then** only the dragged window's region is updated — other windows are not moved.

---

### User Story 3 - Functional Close Button (Priority: P2)

Clicking the red traffic-light dot terminates the app. The window region is cleared, remaining apps reflow into the tiled layout, and focus transfers automatically.

**Why this priority**: The close button has been a visible stub since early development. Users expect it to close the window; implementing it completes the window-chrome interaction model.

**Independent Test**: Run two display apps. Click the red dot of one. Confirm it exits, its region is cleared, the remaining app expands to fill the screen, and keyboard focus transfers.

**Acceptance Scenarios**:

1. **Given** a running app, **When** the user clicks its close button (red dot, 12×12 hit area at the left of the title bar), **Then** the supervisor terminates the app within 200 ms.
2. **Given** the closed app held keyboard focus, **When** it exits, **Then** focus automatically transfers to another running display app — no key events are lost.
3. **Given** only one display app is running, **When** the close button is clicked, **Then** the app exits cleanly and the screen shows an empty desktop.
4. **Given** the app has restart = "always", **When** close is clicked, **Then** the supervisor terminates it and the app respawns per its restart policy.

---

### User Story 4 - Minimize and Restore (Priority: P3)

Clicking the yellow traffic-light dot collapses the window to a 240×28 strip at the bottom of the screen. Clicking that strip restores the window to its previous size and position.

**Why this priority**: Minimize lets users declutter the screen without killing an app. It completes the traffic-light interaction model and is the last pending stub action. Lower priority because the desktop is usable without it.

**Independent Test**: Run two apps. Minimize one via the yellow dot. Confirm it collapses to a bottom strip. Click the strip. Confirm the window restores to its prior dimensions.

**Acceptance Scenarios**:

1. **Given** a running windowed app, **When** the user clicks the minimize button (yellow dot), **Then** the app's region is replaced by a 240×28 strip at the bottom of the screen within 100 ms.
2. **Given** a minimized app strip, **When** the user clicks it, **Then** the window restores to its pre-minimize region within 100 ms and the app redraws its content.
3. **Given** an app is minimized, **When** another app sends draw commands, **Then** the minimized strip is not overwritten by the other app's content.
4. **Given** multiple apps are minimized simultaneously, **When** their strips would overlap, **Then** they are placed side-by-side horizontally with no overlap.

---

### Edge Cases

- What happens when the user drags a window whose app crash-loops and respawns mid-drag? The drag is cancelled; the respawned app receives a freshly tiled region.
- What happens when close is clicked on an app that has already exited (restart = "never")? The supervisor has already cleaned up; the click is a no-op.
- What happens if all apps are minimized? The desktop is empty except for the bottom strip row; the menu bar remains visible.
- What happens when the screen is too narrow to show all minimized strips side-by-side? Strips stack on a second row above the first.
- What happens when the drag starts on a traffic-light dot area? The button action fires instead; drag does not initiate.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: While the left mouse button is held over a window's title bar (excluding traffic-light hit areas) and the mouse moves, the supervisor MUST update that window's `win_region` by the cumulative drag delta on every mouse sync event.
- **FR-002**: The supervisor MUST clamp `win_region` during drag so the window cannot move off-screen: left edge ≥ 0, top edge ≥ menu-bar height, right edge ≤ screen width, bottom edge ≤ screen height.
- **FR-003**: On each drag update the supervisor MUST repaint only the dragged window's title bar and status strip; full redraws of other windows MUST NOT occur during the drag.
- **FR-004**: When the left button is released after a drag, the supervisor MUST compute the Chebyshev distance from the final window position to each tiled-grid slot for the current number of display apps; if the nearest slot is within 40 px, the window MUST snap to that slot's exact dimensions.
- **FR-005**: When the user clicks the close button (red dot, hit area wx+8..wx+20, wy+8..wy+20), the supervisor MUST terminate that app using the same kill path as `@supervisor: kill <name>`.
- **FR-006**: After a close-button kill the supervisor MUST trigger a tiling reflow for the remaining display apps and transfer keyboard focus via the existing `auto_transfer_focus()` function.
- **FR-007**: When the user clicks the minimize button (yellow dot, hit area wx+24..wx+36, wy+8..wy+20), the supervisor MUST save the current `win_region` and replace it with a 240×28 strip anchored at the screen bottom, positioned to avoid overlapping other minimized strips.
- **FR-008**: The supervisor MUST preserve the pre-minimize `win_region` in per-app state; restoring MUST return the window to exactly that saved region.
- **FR-009**: When the user clicks a minimized strip area, the supervisor MUST restore the app's `win_region` from saved state and trigger a title-bar repaint at the restored position.
- **FR-010**: Minimized apps MUST continue to run and receive keyboard events when focused; only their visual content area is hidden.
- **FR-011**: The supervisor MUST NOT issue a full-framebuffer blit during a drag; only scanline rows covered by the dragged title bar's old and new positions need to be updated.

### Key Entities

- **DragState**: Tracks which app is being dragged, the cursor's position at drag-start, and the window's `win_region` at drag-start. Stored as a global in `mouse_input.rs` alongside `MOUSE_DRAG_START`.
- **MinimizeRecord**: Per-app stored state — `minimized: bool` and `pre_minimize_region: Option<(u32,u32,u32,u32)>` — added to `AppState` in `main.rs`.
- **TiledSlot**: Computed on demand from `compute_tiling_with_hints` for snap-distance checks; not persistently stored.

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: A window drag updates the window's on-screen position within one render frame (≤ 33 ms at 30 fps) of each mouse-move event during a drag.
- **SC-002**: The close button terminates an app within 200 ms of the click; the vacated region is cleared and remaining windows are relaid out within one additional frame.
- **SC-003**: Minimize collapses the window to a strip within 100 ms; restore returns it to the prior position within 100 ms.
- **SC-004**: Snap-back triggers for every release within 40 px of a tiled slot and never triggers outside 40 px — verified by unit tests with zero false positives.
- **SC-005**: All existing supervisor unit tests continue to pass; zero new compiler warnings with `RUSTFLAGS=-D warnings`.

## Assumptions

- The supervisor's mouse event loop runs on Linux via evdev (`/dev/input/event*`); drag implementation reads the same raw events already processed in `mouse_input.rs`.
- Screen dimensions are obtained from `display::screen_size().unwrap_or((1440, 900))` as in `apply_tiling_layout`.
- The title bar drag region is the full title-bar strip minus the three traffic-light hit areas; clicking a button does not initiate a drag.
- Minimized strips reuse the existing `draw_titlebar()` function at 240×28; no new drawing primitive is required.
- Window content clipping for the updated `win_region` flows through the existing `draw_cmd.rs` clip logic without additional changes.
- Resize handles, maximize (already implemented via Alt+F snap), and multi-monitor support are explicitly out of scope.
