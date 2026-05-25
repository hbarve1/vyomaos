# Feature Specification: Windowing System and Mouse Input

**Feature Branch**: `002-windowing-mouse-input`

**Created**: 2026-05-24

**Status**: Draft

**Input**: Implement windowing system and mouse input for VyomaOS: each WASM app with display=true gets a tiled window region on the framebuffer (no overlapping, supervisor assigns regions), mouse cursor is rendered by the supervisor, click/move events are routed to the app whose window contains the cursor, apps declare preferred window size in vyoma.toml [window] table, keyboard focus follows mouse click, supervisor handles window layout on app spawn/exit.

## User Scenarios & Testing *(mandatory)*

### User Story 1 - Tiled Window Layout (Priority: P1)

When multiple display-capable apps are running, the user sees each app's output in its own dedicated, non-overlapping region of the screen. The supervisor divides the framebuffer into a grid of tiled windows at startup and reassigns the layout whenever an app spawns or exits. Each app paints only within its assigned region; the supervisor clips any drawing commands that exceed the boundary.

**Why this priority**: Without a functioning layout system, all display apps would overwrite each other on the framebuffer, making the GUI unusable. This is the foundational capability everything else depends on.

**Independent Test**: Boot VyomaOS with three display apps. Confirm each app's output appears in a distinct, non-overlapping area covering the full screen with no blank gaps. Delivers immediate visible value with no mouse hardware required.

**Acceptance Scenarios**:

1. **Given** two display apps are running, **When** the system boots, **Then** the screen is divided into two equal side-by-side regions, one per app, with no overlap and no uncovered area.
2. **Given** four display apps are running, **When** the system boots, **Then** the screen is divided into a 2×2 grid, one region per app.
3. **Given** three display apps are running and one exits, **When** the exit is detected, **Then** the remaining two apps are relaid out to fill the full screen within 500 ms without visual artifacts.
4. **Given** a display app issues a draw command that extends beyond its assigned region, **When** the supervisor processes it, **Then** the draw command is clipped silently to the app's region; no other app's content is affected.
5. **Given** an app declares a preferred minimum width and height in its manifest, **When** the supervisor assigns regions, **Then** the app receives a region at least as large as its preferred minimum dimensions, if screen space allows.

---

### User Story 2 - Mouse Cursor Rendering (Priority: P2)

The user sees a visible mouse cursor on screen that tracks physical mouse movement in real time. The cursor is rendered by the supervisor on top of all app windows and is always visible regardless of which app is in focus.

**Why this priority**: The cursor must exist before click routing is meaningful. It is independently testable without requiring any per-app mouse-event handling.

**Independent Test**: Move the physical mouse; confirm a cursor sprite moves smoothly across the full screen, crossing window boundaries freely. No app changes needed.

**Acceptance Scenarios**:

1. **Given** the system is running with a connected mouse, **When** the user moves the mouse, **Then** a cursor sprite moves on screen within 50 ms of the physical movement, tracking position accurately.
2. **Given** the cursor is near a window boundary, **When** the user moves the mouse across the boundary, **Then** the cursor crosses smoothly with no jump or disappearance.
3. **Given** an app is actively redrawing its window, **When** the cursor is positioned over that window, **Then** the cursor remains visible on top of the app's content at all times.
4. **Given** the cursor reaches the screen edge, **When** the user continues moving the mouse in that direction, **Then** the cursor stops at the edge without wrapping.

---

### User Story 3 - Mouse Event Routing to Apps (Priority: P2)

When the user clicks or moves the mouse over an app's window, that app receives the event coordinates in its own local coordinate space. Apps that declare `mouse = true` in their manifest receive move and click events; apps without that capability receive nothing.

**Why this priority**: Enables apps to respond to user interaction. Depends on US1 (window regions) and US2 (cursor position) being established first.

**Independent Test**: Run a display app with `mouse = true` that prints received mouse events to its output. Click and move inside its window; confirm events arrive with correct local coordinates. Run a second app without `mouse = true`; confirm it receives no events.

**Acceptance Scenarios**:

1. **Given** an app with `mouse = true` is running, **When** the user clicks inside its window, **Then** the app receives a click event with x/y coordinates relative to the top-left corner of its own window region.
2. **Given** an app with `mouse = true` is running, **When** the user moves the mouse inside its window, **Then** the app receives move events at a rate of at least 30 events per second.
3. **Given** an app without `mouse = true` in its manifest, **When** the user clicks anywhere on screen, **Then** that app receives no mouse events.
4. **Given** the cursor is outside an app's window, **When** the user clicks, **Then** that app receives no click event regardless of its capability declaration.
5. **Given** the user clicks on a window, **When** the click is processed, **Then** keyboard focus shifts to that app and it begins receiving keyboard input.

---

### User Story 4 - Keyboard Focus Follows Mouse Click (Priority: P3)

Clicking on any app's window transfers keyboard focus to that app. Only the focused app receives keyboard input. A visual indicator (highlighted window border) shows which app has focus.

**Why this priority**: Completes the interaction model. Depends on US1 and US3. Lower priority because keyboard routing already exists — this extends it with click-to-focus selection.

**Independent Test**: Run two keyboard-interactive apps side by side. Click on app A and type; confirm input goes to A. Click on app B and type; confirm input goes to B and A stops receiving it.

**Acceptance Scenarios**:

1. **Given** two apps are running, **When** the user clicks on app B's window, **Then** app B receives subsequent keyboard input and app A does not.
2. **Given** app A has focus, **When** the user clicks on app B, **Then** app A's window border changes to the unfocused style and app B's changes to the focused style within 100 ms.
3. **Given** the focused app exits, **When** the exit is detected, **Then** focus transfers to another running app automatically; no keyboard input is lost.
4. **Given** only one app is running, **When** the system starts, **Then** that app has keyboard focus by default.

---

### Edge Cases

- What happens when no display apps are running? The framebuffer shows a blank background; the cursor is still visible.
- What happens when more apps are spawned than the maximum tile count (9)? Apps beyond the limit are spawned without a visible window region; their draw commands are discarded silently until space becomes available.
- What happens if a mouse device is not present? Mouse cursor rendering and event routing are skipped; keyboard-only operation continues normally.
- What happens if an app exits while the cursor is inside its window? The window is removed, the layout reflows, and the cursor position is preserved at its absolute screen coordinates.
- What happens when an app crashes mid-draw (partial frame)? The supervisor discards the partial frame; the previous complete frame remains visible until the app is restarted.

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: The supervisor MUST divide the framebuffer into non-overlapping tiled regions, one per running display-capable app, covering the full screen area.
- **FR-002**: The supervisor MUST recompute and apply the tiled layout whenever a display-capable app spawns or exits, within 500 ms of the event.
- **FR-003**: The supervisor MUST clip all drawing commands from each app to that app's assigned window region; commands that exceed the boundary MUST be silently truncated.
- **FR-004**: Apps MUST be able to declare a preferred minimum width and height in their manifest's `[window]` section; the supervisor MUST honor these minimums when screen space permits.
- **FR-005**: The supervisor MUST render a mouse cursor sprite at the current pointer position, composited above all app windows, whenever a mouse device is present.
- **FR-006**: The supervisor MUST update the on-screen cursor position within 50 ms of receiving a mouse movement event from the input device.
- **FR-007**: The supervisor MUST route mouse move and click events to the app whose window region contains the cursor, translated to that app's local coordinate space (origin at top-left of the window), delivered as lines on the app's stdin using the format: `VYOMA_INPUT:mouse:move:<x>,<y>` for movement and `VYOMA_INPUT:mouse:click:<x>,<y>:<button>` for clicks (where `<button>` is `left`, `right`, or `middle`).
- **FR-008**: Mouse events MUST only be delivered to apps that declare `mouse = true` in their manifest; apps without this capability MUST receive no mouse events.
- **FR-009**: A mouse click on any app's window MUST transfer keyboard focus to that app; all subsequent keyboard input MUST be routed exclusively to the focused app.
- **FR-010**: The currently focused app's window MUST be visually distinguished from unfocused windows via a 2 px inset border (focused: accent color; unfocused: dim color). No title bar is rendered; the app owns 100% of its assigned window region pixels.
- **FR-011**: When the focused app exits, the supervisor MUST automatically transfer focus to another running app.
- **FR-012**: The tiling layout MUST support 1 through 9 simultaneously visible display apps without manual configuration.
- **FR-013**: All existing display apps that do not declare `mouse = true` MUST continue to function correctly without any source code modifications.

### Key Entities

- **Window Region**: A rectangular area of the framebuffer assigned exclusively to one app. Attributes: position (x, y), dimensions (width, height), owning app name, focus state.
- **App Window Preference**: Declared in the app manifest's `[window]` section. Attributes: preferred minimum width, preferred minimum height, optional title string.
- **Mouse Event**: An input event delivered to an app. Attributes: event type (move / left-click / right-click / scroll), x/y in app-local coordinates, timestamp.
- **Focus State**: Tracks which app currently receives keyboard input. Attributes: focused app name, previous app name (for border redraw).

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: With 4 display apps running, the full screen is covered by non-overlapping windows with zero unclaimed pixels, verified on every boot.
- **SC-002**: Window layout reflow after an app exit completes in under 500 ms with no visual tearing or blank regions during the transition.
- **SC-003**: Mouse cursor position on screen lags physical mouse movement by no more than 50 ms under normal load.
- **SC-004**: 100% of mouse events delivered to an app have coordinates correctly translated to that app's local coordinate space (verifiable by a test app that echoes received events).
- **SC-005**: Clicking on a window transfers keyboard focus within 100 ms, confirmed by the focused-border indicator updating.
- **SC-006**: All existing display apps (those without `mouse = true`) continue to function correctly after this change with zero source code modifications required.
- **SC-007**: The full `make test` suite (unit tests + smoke boot) continues to pass after implementation.

## Assumptions

- The physical screen resolution is fixed at boot time; runtime resize is out of scope for this phase.
- Mouse hardware delivers events via a standard Linux input device file already accessible to the supervisor.
- The tiled layout uses a column-first grid algorithm: columns = `ceil(sqrt(n))`, rows = `ceil(n / columns)`, windows filled left-to-right top-to-bottom; the last row's empty slots are absorbed by expanding those windows to fill the row. Manual window positioning is out of scope for this phase.
- Floating (overlapping) windows are out of scope; all windows are tiled.
- Window decoration is a 2 px inset border only (no title bar, no close button). The app owns 100% of its assigned region; focus state is indicated solely by border color (accent color when focused, dim color when unfocused).
- Apps that do not declare `display = true` are entirely unaffected by this feature.
- The supervisor runs on a single CPU core; compositing does not require multi-threaded rendering.
- Maximum simultaneously visible display apps is 9 (3×3 grid); apps beyond this are queued without a visible region.

## Clarifications

### Session 2026-05-24

- Q: What line format should the supervisor write to an app's stdin for mouse events? → A: `VYOMA_INPUT:mouse:move:<x>,<y>` for movement; `VYOMA_INPUT:mouse:click:<x>,<y>:<button>` for clicks (`button` = `left`, `right`, or `middle`). Mirrors the existing VYOMA_INPUT protocol prefix for consistency.
- Q: How should the supervisor compute the tiling grid for non-square app counts? → A: Column-first grid: `columns = ceil(sqrt(n))`, `rows = ceil(n / columns)`, filled left-to-right; last row's empty slots are absorbed by expanding those windows to fill the row width.
- Q: Does the supervisor render a title bar above each app's content area, or only a border? → A: Border only — a 2 px inset border on all sides; no title bar. App owns 100% of its assigned region. Focus shown by accent border color (focused) vs dim border color (unfocused).
