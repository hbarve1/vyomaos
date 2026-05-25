# Tasks: Interactive Window Management

**Input**: Design documents from `specs/042-interactive-window-management/`

**Branch**: `042-interactive-window-management` | **Spec**: [spec.md](spec.md) | **Plan**: [plan.md](plan.md)

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no deps on incomplete tasks)
- **[Story]**: US1–US4 maps to user stories in spec.md

---

## Phase 1: Setup

**Purpose**: Establish baseline before any changes.

- [ ] T001 Run `cargo test --target x86_64-unknown-linux-musl` inside Docker (`make unit-test`) and confirm all existing tests pass — record pass count as baseline
- [ ] T002 [P] Create `supervisor/tests/drag_test.rs` with module-level `#[cfg(test)]` block and two placeholder `use` lines for `supervisor::windows` — file must compile cleanly

---

## Phase 2: Foundational (Blocking Prerequisite)

**Purpose**: Extend `AppState` with minimize fields — required by US4 logic and `draw_cmd.rs` guard. Must complete before US4 phases, can overlap with US1/US2/US3.

**⚠️ CRITICAL**: `draw_cmd.rs` guard (US4) cannot compile without these fields.

- [ ] T003 Add `minimized: bool` (default `false`) and `pre_minimize_region: Option<(u32, u32, u32, u32)>` (default `None`) to the `AppState` struct in `supervisor/src/main.rs`; initialize both fields at every `AppState { .. }` construction site in the file

**Checkpoint**: `cargo check --target x86_64-unknown-linux-musl` passes with zero warnings.

---

## Phase 3: User Story 1 — Title-Bar Drag to Move Window (Priority: P1) 🎯 MVP

**Goal**: While left button is held on a title bar, `win_region` updates live on every mouse sync event; title bar repaints at new position; screen edges clamp movement.

**Independent Test**: Run two display apps, hold left button on one title bar, move mouse 100 px right — window follows; release — window stays; other window untouched.

### Tests for User Story 1

- [ ] T004 [P] [US1] In `supervisor/tests/drag_test.rs`, write `test_drag_clamp_x_min`: verify that clamping `new_x = (wx as i32 + dx).clamp(0, (sw - ww as i32).max(0)) as u32` with `wx=5, dx=-20, sw=1440, ww=720` yields `0`
- [ ] T005 [P] [US1] In `supervisor/tests/drag_test.rs`, write `test_drag_clamp_y_min`: verify clamping `new_y` with `wy=10, dy=-30, sh=900, wh=400, menubar_h=24` yields `24` (MENUBAR_H floor)

### Implementation for User Story 1

- [ ] T006 [US1] Add `DragState` struct (`app_name: String`, `cursor_start: (i32,i32)`, `win_start: (u32,u32,u32,u32)`) and `static DRAG_STATE: OnceLock<Mutex<Option<DragState>>>` with `fn drag_state()` accessor to `supervisor/src/mouse_input.rs`
- [ ] T007 [US1] In the `pending_click_mask & 1 != 0` (left-button-press) path in `supervisor/src/mouse_input.rs`: after recording `MOUSE_DRAG_START`, iterate `z_snap` to find the title-bar app under cursor (excluding traffic-light hit areas `wx+8..wx+20` and `wx+24..wx+52`); if found, write `Some(DragState { .. })` into `drag_state()`
- [ ] T008 [US1] Replace the existing log-only drag block (the `btn_held & 1 != 0` branch in `supervisor/src/mouse_input.rs`) with: read `drag_state()`, compute `(dx, dy)` via `drag_delta`, clamp new `(x, y)` to `[0, sw-ww]` × `[MENUBAR_H, sh-wh]`, write `new_region` into registry, call `fb.fill_rect` on old title-bar rows then `draw_titlebar` at new pos then `fb.flush()`

**Checkpoint**: `cargo check` zero warnings. Drag moves title bar live; other windows unaffected.

---

## Phase 4: User Story 2 — Snap-Back to Tiled Grid (Priority: P2)

**Goal**: On left-button release after a drag, if the window's top-left is within 40 px Chebyshev distance of a tiled slot, snap it to that slot exactly.

**Independent Test**: Four apps in 2×2 grid — drag 30 px off slot → release → snaps back. Drag 50 px → release → stays.

### Tests for User Story 2

- [ ] T009 [P] [US2] In `supervisor/tests/drag_test.rs`, write `test_nearest_tiled_slot_snap`: call `nearest_tiled_slot((5, 29), 1, 1440, 900, 24)` (pos 5 px from origin of single-app slot `(0,24,1440,876)`) — assert `Some((0, 24, 1440, 876))`
- [ ] T010 [P] [US2] In `supervisor/tests/drag_test.rs`, write `test_nearest_tiled_slot_no_snap`: call `nearest_tiled_slot((100, 200), 1, 1440, 900, 24)` (>40 px from `(0,24)`) — assert `None`

### Implementation for User Story 2

- [ ] T011 [US2] Add `pub fn nearest_tiled_slot(win_pos: (u32, u32), n_apps: usize, sw: u32, sh: u32, menubar_h: u32) -> Option<(u32, u32, u32, u32)>` to `supervisor/src/windows.rs`: call `compute_tiling_with_hints(n_apps, sw, sh.saturating_sub(menubar_h), &vec![(0u32,0u32); n_apps])`, offset each slot's y by `menubar_h`, find slot with minimum Chebyshev distance to `win_pos`, return it only if distance ≤ 40
- [ ] T012 [US2] In the `pending_release_mask` (left-button-release) path in `supervisor/src/mouse_input.rs`: call `drag_state().lock().unwrap().take()`, if `Some(ds)` read current `win_region` from registry, count running display apps, call `windows::nearest_tiled_slot(...)`, if `Some(snapped)` write snapped region back into registry and log the snap

**Checkpoint**: Unit tests T009/T010 pass. Release near slot snaps; release far does not.

---

## Phase 5: User Story 3 — Functional Close Button (Priority: P2)

**Goal**: Red dot click kills the app; tiling reflows; focus transfers. The exit watcher in `app_threads.rs` already handles reflow + focus on any app death — close button needs only to send the kill signal (already implemented).

**Independent Test**: Two display apps — click red dot — app exits ≤200 ms — remaining app expands — focus transfers.

### Verification for User Story 3

- [ ] T013 [US3] Read `supervisor/src/mouse_input.rs` `TrafficLight::Close` arm and `supervisor/src/app_threads.rs` `wait_app()` function; confirm SIGKILL is sent in the close arm and that `apply_tiling_layout` + `auto_transfer_focus` are called on app exit in `wait_app()`; add a one-line `// Exit watcher in app_threads.rs handles reflow + focus transfer` comment to the Close arm if it lacks one — **no logic change required**
- [ ] T014 [P] [US3] In `supervisor/tests/drag_test.rs`, write `test_close_button_no_code_change`: a doc-test-style `#[test]` that asserts `true` (serves as documentation that US3 is implemented via the exit-watcher path) with a comment citing `app_threads::wait_app`

**Checkpoint**: `cargo test` still passes. Close button terminates app; reflow and focus work through existing exit path.

---

## Phase 6: User Story 4 — Minimize and Restore (Priority: P3)

**Goal**: Yellow dot collapses window to 240×28 strip at screen bottom; clicking strip restores to saved region.

**Independent Test**: Two apps — click yellow dot — window collapses to bottom strip — click strip — window restores to prior size and position.

### Tests for User Story 4

- [ ] T015 [P] [US4] In `supervisor/tests/drag_test.rs`, write `test_minimize_strip_layout`: for `minimized_count=0`, `sw=1440`, `sh=900` — assert strip pos is `(0, 872, 240, 28)` (y = 900 - 28)
- [ ] T016 [P] [US4] In `supervisor/tests/drag_test.rs`, write `test_minimize_strip_wrap`: for `minimized_count=6`, `sw=1440` — strip 6 starts at `x=1440` → wraps to `x=0`, `y=872-28=844` (second row)

### Implementation for User Story 4

- [ ] T017 [US4] Replace the stub `TrafficLight::Minimize` arm in `supervisor/src/mouse_input.rs` with: count currently-minimized apps to compute strip `x = count * 240` (wrap if `x >= sw`), set `st.pre_minimize_region = st.win_region`, `st.minimized = true`, `st.win_region = Some((strip_x, strip_y, 240, 28))`, call `draw_titlebar` at strip pos + `fb.flush()`
- [ ] T018 [US4] In the click-dispatch path of `supervisor/src/mouse_input.rs` (before the traffic-light hit test), add a minimized-strip restore check: scan registry for any app with `minimized == true` whose strip region contains `(cx, cy)`; if found, call `.pre_minimize_region.take()`, set `win_region` to saved region, `minimized = false`, repaint title bar at restored pos, `return`
- [ ] T019 [US4] Add early-return guard to `supervisor/src/draw_cmd.rs` in `handle_draw_command`: after resolving the sender's `AppState`, if `st.minimized` is `true` return immediately without processing the draw command

**Checkpoint**: `cargo test` passes. Minimize/restore works; minimized app draw commands are silently dropped.

---

## Phase 7: Polish & Verification

- [ ] T020 Run `RUSTFLAGS=-D warnings cargo test --target x86_64-unknown-linux-musl` (via `make unit-test`) — assert zero warnings, all tests pass, ≥ 5 new tests in `drag_test.rs`
- [ ] T021 [P] Verify 500-line rule: `wc -l supervisor/src/*.rs` — no file may exceed 500 lines

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No deps — start immediately
- **Foundational (Phase 2)**: No deps — start immediately, overlaps with Phase 3 US1
- **US1 (Phase 3)**: No deps on Foundational; can start immediately
- **US2 (Phase 4)**: Depends on US1 drag release path (T008) — start after T008
- **US3 (Phase 5)**: Independent — can run in parallel with US1/US2
- **US4 (Phase 6)**: Depends on Foundational T003 (AppState fields) — start after T003
- **Polish (Phase 7)**: Depends on all phases complete

### Within Each User Story

- Tests ([P] marked) written before implementation tasks for that story
- `DragState` struct (T006) before drag initiation (T007) and update (T008)
- `nearest_tiled_slot` (T011) before snap integration (T012)
- Minimize handler (T017) before strip click detection (T018)
- draw_cmd guard (T019) last in US4

### Parallel Opportunities

- T001, T002, T003 can all start simultaneously
- T004, T005 (US1 tests), T009, T010 (US2 tests), T015, T016 (US4 tests) all parallelizable
- T013 (US3 verify) fully independent — run anytime
- T020 and T021 (Polish) can run in parallel at end

---

## Parallel Example: US2

```bash
# Both tests written before implementation:
Task T009: test_nearest_tiled_slot_snap  →  supervisor/tests/drag_test.rs
Task T010: test_nearest_tiled_slot_no_snap  →  supervisor/tests/drag_test.rs
# Then implement:
Task T011: nearest_tiled_slot()  →  supervisor/src/windows.rs
Task T012: drag release snap  →  supervisor/src/mouse_input.rs
```

---

## Implementation Strategy

### MVP First (US1 + US2 only)

1. Complete T001–T002 (Setup)
2. Complete T003 (Foundational)
3. Complete T004–T008 (US1 drag) → window drag works
4. Complete T009–T012 (US2 snap) → snap works
5. **STOP and VALIDATE**: drag + snap independently testable

### Full Feature

Add US3 (T013–T014, trivial), then US4 (T015–T019), then polish (T020–T021).

### File Summary

| File | Tasks | Change |
|------|-------|--------|
| `supervisor/src/main.rs` | T003 | Add 2 fields to AppState |
| `supervisor/src/windows.rs` | T011 | Add `nearest_tiled_slot()` |
| `supervisor/src/mouse_input.rs` | T006–T008, T012, T013, T017–T018 | DragState, drag loop, snap, minimize, strip click |
| `supervisor/src/draw_cmd.rs` | T019 | Early return on minimized |
| `supervisor/tests/drag_test.rs` | T002, T004–T005, T009–T010, T014–T016 | New: ≥5 unit tests |

---

## Notes

- No file may exceed 500 lines (CLAUDE.md constraint) — check with T021
- `RUSTFLAGS=-D warnings` must produce zero new warnings
- All existing tests must continue passing throughout
- Close button (US3) requires NO new implementation — verified in T013
