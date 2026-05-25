# Tasks: Windowing System and Mouse Input

**Input**: Design documents from `specs/002-windowing-mouse-input/`

**Prerequisites**: plan.md ✅, spec.md ✅, research.md ✅, data-model.md ✅, contracts/ ✅

---

## Phase 1: Setup

**Purpose**: New module scaffolding and test app creation; no logic yet.

- [ ] T001 Add `pub mod windows;` to `supervisor/src/lib.rs`
- [ ] T002 [P] Create empty `supervisor/src/windows.rs` with module doc comment and function stub: `pub fn compute_tiling(n: usize, sw: u32, sh: u32) -> Vec<(u32, u32, u32, u32)> { vec![] }`
- [ ] T003 [P] Create `supervisor/tests/window_layout_test.rs` with test skeleton (no assertions yet; just `#[test] fn placeholder() {}`)
- [ ] T004 [P] Create `apps/mouse-demo/` directory, `Cargo.toml`, `src/main.rs` stub (prints "mouse-demo started" and loops), and `vyoma.toml` with `display=true, mouse=true`
- [ ] T005 Verify `cargo check --target x86_64-unknown-linux-musl` still passes with zero new warnings after setup

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core layout function + cursor state structures that all user stories depend on.

⚠️ CRITICAL: No user-story work can begin until this phase is complete.

- [ ] T006 Implement `compute_tiling(n, sw, sh)` in `supervisor/src/windows.rs`: cols=ceil(sqrt(n)), rows=ceil(n/cols), base cell sizes, last-row expansion algorithm (see contracts/window-layout-contract.md); accept optional `min_sizes: &[(u32,u32)]` — if any app's declared width > cell_w, give it a full-row slot (best-effort, log warning if all constraints cannot fit)
- [ ] T007 Write unit tests in `supervisor/tests/window_layout_test.rs`: test_1_fullscreen, test_2_side_by_side, test_4_apps_2x2, test_3_last_row_expands, test_no_overlap_1_to_9, test_full_coverage_1_to_9
- [ ] T008 Run `cargo test --target x86_64-unknown-linux-musl window_layout` and verify all 6 tests pass
- [ ] T009 Add `CursorState` struct to `supervisor/src/display.rs` inside `Framebuffer`: fields `cx: i32, cy: i32, visible: bool, saved_under: [u8; 12 * 19 * 4], drawn: bool`; initialize in `open_fb()` with center position
- [ ] T010 Add `CURSOR_W: u32 = 12`, `CURSOR_H: u32 = 19`, and `CURSOR_MASK: [u16; 19]` constants for a standard arrow sprite in `supervisor/src/display.rs`
- [ ] T011 Verify `cargo check` passes with zero new warnings

**Checkpoint**: `compute_tiling` tests pass; `CursorState` compiles; `CURSOR_MASK` defined.

---

## Phase 3: User Story 1 — Tiled Window Layout (Priority: P1) 🎯 MVP

**Goal**: Supervisor auto-assigns non-overlapping tiled regions to all display apps on boot and reflow on spawn/exit.

**Independent Test**: Boot with 3 display apps; verify each occupies a distinct non-overlapping screen region; kill one and verify 2-window reflow within 500 ms.

### Implementation for User Story 1

- [ ] T012 [US1] Add helper `fn collect_display_apps(registry: &AppRegistry) -> Vec<String>` in `supervisor/src/main.rs` that returns names of all Running apps with `has_display=true`, in registration order
- [ ] T013 [US1] Add `fn apply_tiling_layout(registry: &AppRegistry)` in `supervisor/src/main.rs` that calls `compute_tiling`, then updates each display app's `win_region` in the registry; log `log_info!(Subsystem::Display, …, "layout reflow: {n} display apps → {cols}×{rows}")` 
- [ ] T014 [US1] Call `apply_tiling_layout()` in `launch_app_threads()` path (after app spawn) — only when the new app is a display app
- [ ] T015 [US1] Call `apply_tiling_layout()` in `wait_app()` (after app exit) — only when the exited app had a win_region
- [ ] T016 [US1] Stop reading `x/y/w/h` from manifest `WindowRegion` into `win_region` (those 4 fields are now supervisor-computed); still read `width/height` as min-size hints (pass to `compute_tiling` for best-effort min enforcement)
- [ ] T017 [US1] After tiling reflow, repaint 2 px inset borders for all display apps via `handle_draw_command` path: call a new `fn repaint_all_borders(registry, focused)` that locks framebuffer and draws borders in the back-buffer
- [ ] T018 [US1] Write integration smoke test in `supervisor/tests/window_layout_test.rs`: `test_apply_tiling_4_apps` — construct mock AppRegistry with 4 display apps, call `apply_tiling_layout`, assert all win_regions non-overlapping and covering the screen
- [ ] T019 [US1] Run `cargo test --target x86_64-unknown-linux-musl` and verify all tests pass; run `make build` and verify binary still compiles

**Checkpoint**: US1 complete. Boot with multiple display apps and observe tiled layout with no overlap and full coverage.

---

## Phase 4: User Story 2 — Mouse Cursor Rendering (Priority: P2)

**Goal**: A visible cursor sprite tracks physical mouse movement in real time, composited above all app content.

**Independent Test**: Move the physical mouse in QEMU GUI; observe a 12×19 arrow cursor moving smoothly without tearing.

### Implementation for User Story 2

- [ ] T020 [US2] Implement `Framebuffer::draw_cursor(&mut self)` in `supervisor/src/display.rs`: save `CURSOR_W×CURSOR_H` BGRA pixels from `self.back` into `self.cursor.saved_under`; paint cursor sprite using `CURSOR_MASK` in white (0xFFFFFF) with 1px black outline; set `self.cursor.drawn = true`; no-op if `!self.cursor.visible`
- [ ] T021 [US2] Implement `Framebuffer::restore_under_cursor(&mut self)` in `supervisor/src/display.rs`: if `self.cursor.drawn`, copy `saved_under` back into correct offset of `self.back`; set `self.cursor.drawn = false`
- [ ] T022 [US2] Modify `Framebuffer::flush()` in `supervisor/src/display.rs`: call `self.restore_under_cursor()` then `self.draw_cursor()` then `copy_nonoverlapping` (cursor on top of final frame); call `self.restore_under_cursor()` again after blit (clean back-buffer for next frame)
- [ ] T023 [US2] Add `pub fn set_cursor_pos(cx: i32, cy: i32)` to `supervisor/src/display.rs`: lock `FB`, clamp to `(0..fb.width-1, 0..fb.height-1)`, update `cursor.cx/cy`
- [ ] T024 [US2] Add `pub fn enable_cursor()` to `supervisor/src/display.rs`: lock `FB`, set `cursor.visible = true`; log cursor enabled
- [ ] T025 [US2] In mouse-input thread (`supervisor/src/main.rs`): replace hardcoded `const SCREEN_W: i32 = 1440; const SCREEN_H: i32 = 900;` with values from `display::screen_size().unwrap_or((1024, 768))`; call `display::enable_cursor()` after `open_mouse_device()` succeeds; call `display::set_cursor_pos(cx, cy)` on every EV_SYN
- [ ] T026 [US2] Write unit test in `supervisor/tests/display_test.rs`: `test_cursor_draw_restore` — create a mock Framebuffer with known back-buffer content, call `draw_cursor()`, verify saved_under matches original pixels, call `restore_under_cursor()`, verify back-buffer fully restored
- [ ] T027 [US2] Run `cargo test --target x86_64-unknown-linux-musl display` and verify tests pass; run `make build`

**Checkpoint**: US2 complete. Run `make run-gui` with a mouse device; cursor sprite visible and tracks mouse movement.

---

## Phase 5: User Story 3 — Mouse Event Routing (Priority: P2)

**Goal**: Mouse move/click events routed to the correct app in new `VYOMA_INPUT:mouse:move:x,y` / `click:x,y:button` format; non-mouse apps unaffected.

**Independent Test**: Run mouse-demo app (display+mouse); click and move inside its window; verify events arrive with correct local coordinates. Non-mouse app receives nothing.

### Implementation for User Story 3

- [ ] T028 [US3] In `dispatch_mouse()` (`supervisor/src/main.rs`): replace `send_reply(&name, &format!("VYOMA_INPUT:mouse:{lx},{ly},{btn}"), inbox)` with separate move/click dispatch: if `btn == 0` emit `VYOMA_INPUT:mouse:move:{lx},{ly}`; if `btn > 0` emit `VYOMA_INPUT:mouse:click:{lx},{ly}:{bname}` where `bname = match btn { 1 => "left", 2 => "right", 4 => "middle", _ => "left" }`
- [ ] T029 [US3] Add `BTN_RIGHT (0x111)` and `BTN_MIDDLE (0x112)` handling to the mouse-input thread (`supervisor/src/main.rs`): update `btn` bitmask (bit 0=left, bit 1=right, bit 2=middle) on `EV_KEY` events
- [ ] T030 [US3] Update click/move dispatch in mouse-input thread: on `EV_KEY BTN_*` press (value=1), set a `pending_click_mask` bitmask flag (bit 0=left, 1=right, 2=middle); on `EV_SYN`, after updating cx/cy, if `pending_click_mask != 0` call `dispatch_mouse(cx, cy, pending_click_mask, …)` then clear mask; if position changed call `dispatch_mouse(cx, cy, 0, …)` for move (ensures all dispatches happen on EV_SYN — one per sync frame)
- [ ] T031 [US3] Implement `apps/mouse-demo/src/main.rs`: read stdin lines, detect `VYOMA_INPUT:mouse:` prefix, parse and display event type + coordinates in the assigned window region using `VYOMA_DRAW:draw_text`; flush each frame
- [ ] T032 [US3] Add mouse-demo to `base/scripts/rootfs.sh` (copy wasm binary into initramfs) and to `base/boot.toml` (restart=never, capabilities: display+mouse)
- [ ] T033 [US3] Write unit tests in `supervisor/tests/` for dispatch format: `test_move_event_format` — verify emitted string starts with `VYOMA_INPUT:mouse:move:`; `test_click_event_format_left` — verify `VYOMA_INPUT:mouse:click:x,y:left`; `test_no_event_for_non_mouse_app` — verify app without mouse=true receives nothing; `test_click_coord_translation` — place win_region at (100,200,400,300), cursor at (142,287), verify received lx=42 ly=87
- [ ] T034 [US3] Run `cargo test --target x86_64-unknown-linux-musl` and verify new tests pass; run `make build`

**Checkpoint**: US3 complete. Mouse-demo app displays received events with correct local coordinates.

---

## Phase 6: User Story 4 — Keyboard Focus Follows Click (Priority: P3)

**Goal**: Click on any window transfers keyboard focus there; 2px inset border indicates focus state; focused app exits → focus auto-transfers.

**Independent Test**: Run two keyboard-interactive apps; click A → type → only A gets input; click B → only B gets input.

### Implementation for User Story 4

- [ ] T035 [US4] Replace title-bar chrome in `handle_draw_command()` flush path (`supervisor/src/main.rs`): remove the 20 px title bar, app name label, and red close-button drawing; replace with a 2px-wide inset border using 4 `fill_rect` calls: top strip `(wx, wy, ww, 2)`, bottom strip `(wx, wy+wh-2, ww, 2)`, left strip `(wx, wy, 2, wh)`, right strip `(wx+ww-2, wy, 2, wh)` — all in `border_color` (accent if focused else dim); read `focused` state to determine color
- [ ] T036 [US4] Remove close-button hit-detection from `dispatch_mouse()`: delete the "P32: check close-button clicks" block (no chrome means no close button)
- [ ] T037 [US4] Implement `fn auto_transfer_focus(exiting_app: &str, registry: &AppRegistry, focused: &FocusedApp)` in `supervisor/src/main.rs`: if `focused == exiting_app`, find the first other Running display-or-shell app and set it as new focused; log the transfer; if no other app exists set `None`
- [ ] T038 [US4] Call `auto_transfer_focus()` from `wait_app()` after the app's state transitions to `Stopped`
- [ ] T039 [US4] After click-to-focus in `dispatch_mouse()`: call `repaint_all_borders(registry, focused)` to immediately update border colors in the back-buffer
- [ ] T040 [US4] Define border color constants in `supervisor/src/main.rs`: `const BORDER_FOCUSED: u32 = 0x89B4FAFF;` and `const BORDER_UNFOCUSED: u32 = 0x45475AFF;`
- [ ] T041 [US4] Write unit tests: `test_auto_transfer_focus_on_exit` — mock registry with 2 apps, call `auto_transfer_focus("app1", …)`, verify focused is now "app2"; `test_no_focus_when_last_app_exits` — single app exits, verify focused is None
- [ ] T042 [US4] Run `cargo test --target x86_64-unknown-linux-musl` and verify all tests pass

**Checkpoint**: US4 complete. Click-to-focus, border indicator, and auto-transfer all work.

---

## Phase 7: Polish & Cross-Cutting Concerns

**Purpose**: Observability, FR-013 regression check, and final validation.

- [ ] T043 [P] Add `log_info!` to `apply_tiling_layout`: log each app name and its new (x,y,w,h) after reflow
- [ ] T044 [P] Add `log_info!` to cursor init path: `"cursor: sprite enabled at ({cx},{cy})"`
- [ ] T045 [P] Verify FR-013: build all existing apps that do NOT have `mouse=true`; confirm they compile without modification and their display output is unaffected
- [ ] T046 Run `make unit-test` (full `cargo test` suite) and verify all tests pass with zero warnings
- [ ] T047 Run `make smoke` (headless QEMU boot) and verify `SMOKE: PASS`
- [ ] T048 Run `make run-gui DISPLAY_BACKEND=cocoa` (or sdl) and manually verify: tiled layout, cursor sprite, event routing, focus border — against quickstart.md Integration Scenarios 1–4
- [ ] T049 [P] Update `apps/*/vyoma.toml` for any apps that currently use `mouse=true` to handle new event format (parse `VYOMA_INPUT:mouse:move:` and `VYOMA_INPUT:mouse:click:` instead of old format)
- [ ] T050 Run `make test` (build + unit-test + smoke) and verify all pass; report final binary size: `ls -lh supervisor/target/x86_64-unknown-linux-musl/release/supervisor`

---

## Dependencies & Execution Order

### Phase Dependencies

- **Phase 1 (Setup)**: No dependencies — start immediately
- **Phase 2 (Foundational)**: Requires Phase 1 — blocks all user stories
- **Phase 3 (US1 Tiling)**: Requires Phase 2 — MVP; must complete before US2/US3/US4 can be verified end-to-end
- **Phase 4 (US2 Cursor)**: Requires Phase 2; can start in parallel with Phase 3 (cursor state is in display.rs, tiling is in windows.rs)
- **Phase 5 (US3 Events)**: Requires Phase 3 (win_region must be set correctly before event routing); requires Phase 2
- **Phase 6 (US4 Focus)**: Requires Phase 3 (layout must be stable); requires Phase 5 (click routing)
- **Phase 7 (Polish)**: Requires all user story phases complete

### Parallel Opportunities

- T002, T003, T004 can run in parallel (different files)
- T006 and T009/T010 can run in parallel (different files: windows.rs vs display.rs)
- T020, T021 can run in parallel (both in display.rs but non-overlapping functions)
- T023, T024 can run in parallel
- T043, T044, T045, T049 can run in parallel (independent files)

---

## Implementation Strategy

### MVP (US1 Only — 19 tasks)

1. Phases 1–3 (T001–T019)
2. Verify: boot with 3 display apps → tiled layout visible → kill one → reflow
3. This delivers SC-001 and SC-002 with no mouse hardware required

### Full Feature (All US — 50 tasks)

Complete Phases 1–7 in order; Phases 3 and 4 can overlap (different files).

---

## Notes

- `[P]` = different files, no inter-task dependency within same phase
- Constitution: `cargo check` zero warnings required after every phase
- FR-013: existing display apps MUST work without modification
- Mouse event format change (T028–T030) is a breaking change for any app currently using `mouse=true` — handle in T049
