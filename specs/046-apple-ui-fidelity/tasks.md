# Tasks: Apple-Platform UI Fidelity

**Branch**: `046-apple-ui-fidelity`
**Input**: `specs/046-apple-ui-fidelity/` — plan.md, spec.md, research.md, data-model.md, contracts/vyoma-draw-v2.md

---

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Add dependencies, font files, and shared rendering primitives used by all user stories.

- [ ] T001 Add `fontdue = "0.7"` and `lodepng = "3"` to `supervisor/Cargo.toml`
- [ ] T002 Download Inter-Regular.ttf, Inter-Bold.ttf, IBMPlexMono-Regular.ttf, IBMPlexMono-Bold.ttf (OFL) to `base/fonts/`
- [ ] T003 Update `base/rootfs.sh` to copy `base/fonts/*.ttf` into `$ROOTFS/fonts/` during initramfs build
- [ ] T004 Create `supervisor/src/font/` directory with empty `mod.rs` and `cache.rs` stubs
- [ ] T005 Create `supervisor/src/image/` directory with empty `mod.rs` stub
- [ ] T006 Create `supervisor/src/display/compositor.rs` stub with `blend_over` signature
- [ ] T007 Create `supervisor/src/display/animator.rs` stub with `Animation` struct skeleton
- [ ] T008 Add `mod font;`, `mod image;` entries to `supervisor/src/main.rs`; verify `cargo check` passes

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Porter-Duff compositing and VYOMA_DRAW v2 parser extensions — required by all rendering user stories.

- [ ] T009 Implement `blend_over(src: u32, dst: u32) -> u32` Porter-Duff "over" operator in `supervisor/src/display/compositor.rs`
- [ ] T010 Implement `unpack(rgba: u32) -> (u8,u8,u8,u8)` and `pack(r,g,b,a) -> u32` helpers in `supervisor/src/display/compositor.rs`
- [ ] T011 Replace flat pixel-write in `supervisor/src/display/mod.rs` framebuffer path with `blend_over` call
- [ ] T012 Add `DrawCmd::Glyph { x, y, rgba, pt, bold, mono, text }` variant to enum in `supervisor/src/draw_cmd.rs`
- [ ] T013 Add `DrawCmd::Image { x, y, w, h, path: String }` variant to enum in `supervisor/src/draw_cmd.rs`
- [ ] T014 Add `DrawCmd::RoundedRect { x, y, w, h, rgba, radius }` variant to enum in `supervisor/src/draw_cmd.rs`
- [ ] T015 Parse `draw_glyph:<x>,<y>,<rgba>,<pt>,<weight>,<text>` in `supervisor/src/draw_cmd.rs` — weight: `regular`|`bold`|`mono`
- [ ] T016 Parse `draw_image:<x>,<y>,<w>,<h>,<path>` in `supervisor/src/draw_cmd.rs`
- [ ] T017 Parse `fill_rect_r:<x>,<y>,<w>,<h>,<rgba>,<radius>` in `supervisor/src/draw_cmd.rs`
- [ ] T018 Verify `cargo check` passes with zero new warnings after all T009–T017 changes

---

## Phase 3: US1 — Scalable Font Rendering (P1)

**Story goal**: Replace 8×16 bitmap font with smooth anti-aliased fontdue rendering; apps use `draw_glyph` with pt sizes.

**Independent test**: Boot VyomaOS, open Notes, type text. Characters are smooth-edged at 14pt; no pixel staircase visible at 2× zoom.

- [ ] T019 [US1] Implement `GlyphBitmap { coverage: Vec<u8>, width, height, advance_x, bearing_y }` struct in `supervisor/src/font/mod.rs`
- [ ] T020 [US1] Implement `FontCache { ui, ui_bold, mono, cache: HashMap }` struct in `supervisor/src/font/mod.rs`
- [ ] T021 [US1] Implement `FontCache::load(ui_path, bold_path, mono_path) -> Self` — reads TTF bytes, parses with fontdue in `supervisor/src/font/mod.rs`
- [ ] T022 [US1] Implement `FontCache::rasterize(&mut self, ch, pt, bold, mono) -> &GlyphBitmap` with HashMap cache key `(char, pt×10 as u32, bold, mono)` in `supervisor/src/font/cache.rs`
- [ ] T023 [US1] Add `FontCache::load("/fonts/Inter-Regular.ttf", "/fonts/Inter-Bold.ttf", "/fonts/IBMPlexMono-Regular.ttf")` call in supervisor startup in `supervisor/src/main.rs`
- [ ] T024 [US1] Implement `composite_glyph(fb, bitmap, x, y, rgba)` in `supervisor/src/display/compositor.rs` — per-pixel: `out_alpha = coverage * color_alpha / 255`, then `blend_over` tinted src with dst
- [ ] T025 [US1] Wire `DrawCmd::Glyph` into display render loop in `supervisor/src/display/mod.rs` — iterate chars, call `rasterize`, call `composite_glyph`, advance cursor by `advance_x`
- [ ] T026 [P] [US1] Migrate title-bar app name `draw_text` → `draw_glyph` at 13pt bold in `supervisor/src/chrome.rs`
- [ ] T027 [P] [US1] Migrate menu-bar clock `draw_text` → `draw_glyph` at 12pt regular in `supervisor/src/chrome.rs`
- [ ] T028 [P] [US1] Migrate status-bar uptime text `draw_text` → `draw_glyph` at 11pt regular in `supervisor/src/chrome.rs`
- [ ] T029 [US1] Run `make unit-test` and confirm zero new warnings; commit `feat(font): scalable font rendering with fontdue`

---

## Phase 4: US2 — Image and Icon Rendering (P2)

**Story goal**: PNG icons in desktop grid and dock; wallpaper from image file.

**Independent test**: Desktop grid shows distinct PNG icons per app. Screenshot reveals non-solid-color pixels in icon cells.

- [ ] T030 [US2] Implement `RgbaImage { rgba: Vec<u8>, width: u32, height: u32 }` struct in `supervisor/src/image/mod.rs`
- [ ] T031 [US2] Implement `ImageCache { cache: HashMap<String, Option<RgbaImage>> }` in `supervisor/src/image/mod.rs`
- [ ] T032 [US2] Implement `ImageCache::get(&mut self, path: &str) -> Option<&RgbaImage>` — read file, `lodepng::decode32`, store in cache; return `None` on decode error (no panic) in `supervisor/src/image/mod.rs`
- [ ] T033 [US2] Implement `blit_image(fb, img, dst_x, dst_y, dst_w, dst_h)` nearest-neighbor scale + `blend_over` per pixel in `supervisor/src/display/compositor.rs`
- [ ] T034 [US2] Wire `DrawCmd::Image` into display render loop in `supervisor/src/display/mod.rs` — call `image_cache.get(path)`, call `blit_image`; log warning if path not found
- [ ] T035 [US2] Add `icon: Option<String>` field to `WindowConfig` struct in `supervisor/src/manifest.rs`; update TOML parser (field is optional, defaults to `None`)
- [ ] T036 [P] [US2] Create placeholder `icon.png` (48×48 solid-color PNG) in `apps/settings/`, `apps/notes/`, `apps/shell/`; add `icon = "icon.png"` to each `vyoma.toml`
- [ ] T037 [US2] Update `apps/desktop/src/main.rs` app-grid cell renderer — emit `VYOMA_DRAW:draw_image:<x>,<y>,48,48,<icon_path>` if icon declared; else fall back to colored text box
- [ ] T038 [US2] Update `apps/dock/src/main.rs` dock-item renderer — emit `VYOMA_DRAW:draw_image` for icon; 32×32 size in dock strip
- [ ] T039 [US2] Run `make build && make run-gui` visual check — icons visible; run `make unit-test`; commit `feat(image): PNG icon rendering in desktop and dock`

---

## Phase 5: US3 — Alpha Compositing and Visual Depth (P3)

**Story goal**: Drop shadows under windows; frosted-glass Spotlight overlay; translucent panels.

**Independent test**: Open Spotlight — wallpaper visible but dimmed behind panel. Move a window — soft shadow follows and overlaps window below.

- [ ] T040 [US3] Implement `rounded_rect_coverage(px, py, w, h, radius) -> u8` distance-field corner mask in `supervisor/src/display/compositor.rs`
- [ ] T041 [US3] Implement `draw_rounded_rect(fb, x, y, w, h, rgba, radius)` using coverage mask × alpha blend in `supervisor/src/display/compositor.rs`
- [ ] T042 [US3] Wire `DrawCmd::RoundedRect` into display render loop in `supervisor/src/display/mod.rs`
- [ ] T043 [US3] Implement `generate_shadow_mask(w, h, blur_r, alpha) -> Vec<u8>` — binary shape fill → 2-pass separable box blur in `supervisor/src/display/compositor.rs`
- [ ] T044 [US3] Add shadow rendering in `supervisor/src/chrome.rs` window paint path — call `generate_shadow_mask` offset by (0, 4), composite beneath window layer before frame
- [ ] T045 [US3] Update `apps/spotlight/src/main.rs` — replace solid `fill_rect` background with `VYOMA_DRAW:fill_rect_r:0,0,1440,900,503316735,0` (80% alpha dark overlay)
- [ ] T046 [US3] Run `make build && make run-gui` — verify frosted Spotlight and window shadows; run `make unit-test`; commit `feat(compositor): alpha compositing, drop shadows, rounded rects`

---

## Phase 6: US4 — Window Chrome Polish (P3)

**Story goal**: Rounded corners, traffic-light buttons, smooth open/close/minimize animations.

**Independent test**: Launch shell from dock — window scales in from 85%→100% over ≤300ms. Click red button — window fades out. All corners are visibly rounded.

*Depends on: US1 (draw_glyph for button labels), US3 (blend_over for fade animations)*

- [ ] T047 [US4] Replace square window border `fill_rect` calls with `draw_rounded_rect(radius=12)` in `supervisor/src/chrome.rs`
- [ ] T048 [US4] Replace `[×]`/`[_]`/`[▪]` text buttons with filled circles in `supervisor/src/chrome.rs`:
  - Red `0xFF5F57FF` at `(frame_x+12, title_center)`, radius 6 → close
  - Yellow `0xFFBD2DFF` at `(frame_x+28, title_center)`, radius 6 → minimize
  - Green `0x28C840FF` at `(frame_x+44, title_center)`, radius 6 → maximize
- [ ] T049 [US4] Update close/minimize/maximize click hit-box coordinates in `supervisor/src/chrome.rs` to match new circle positions
- [ ] T050 [US4] Implement `AnimKind` enum (`Open`, `Close`, `Minimize`) and `AnimState { scale: f32, alpha: u8 }` in `supervisor/src/display/animator.rs`
- [ ] T051 [US4] Implement `Animation::new(kind, duration_ms) -> Self` and `Animation::sample(elapsed_ms) -> AnimState` with linear interpolation; snap to final state if `elapsed_ms >= duration_ms` in `supervisor/src/display/animator.rs`
  - Open: scale 0.85→1.0, alpha 0→255, 300ms
  - Close: scale 1.0→0.85, alpha 255→0, 250ms
  - Minimize: scale 1.0→0.2, alpha 255→0, 300ms
- [ ] T052 [US4] Add `pending_anim: Option<Animation>` field to `AppState` in `supervisor/src/main.rs`
- [ ] T053 [US4] In `supervisor/src/app_threads.rs` app-spawn path — enqueue `AnimKind::Open` on `AppState.pending_anim`
- [ ] T054 [US4] In `supervisor/src/ipc_handlers.rs` kill path — enqueue `AnimKind::Close`; defer `AppState` removal until animation completes (check `AnimState.alpha == 0`)
- [ ] T055 [US4] In minimize handler in `supervisor/src/chrome.rs` — enqueue `AnimKind::Minimize`; collapse window to title-bar-only strip at bottom after anim completes
- [ ] T056 [US4] In display frame tick in `supervisor/src/display/mod.rs` — for each app with `pending_anim`, call `anim.sample(elapsed_ms)`, apply `scale` as window layer transform, apply `alpha` via `set_layer_alpha` before composite
- [ ] T057 [US4] Run `make unit-test`; manually verify animations in `make run-gui`; commit `feat(chrome): rounded corners, traffic-light buttons, open/close/minimize animations`

---

## Phase 7: US5 — System UI Components (P4)

**Story goal**: Menu bar dropdowns, right-click context menus, notification banners with icon+title+body.

**Independent test**: Click app name in menu bar → dropdown appears with items. Arrow keys navigate. Enter sends IPC. Right-click desktop → context menu. An app IPC `@supervisor: notify Title|Body` → banner slides in, auto-dismisses after 4s.

*Depends on: US1 (draw_glyph for text), US2 (draw_image for icon in banner), US3 (fill_rect_r for translucent panel)*

- [ ] T058 [US5] Add `menu_items: Vec<MenuItem>` to `AppManifest` and `MenuItem { label: String, action: String, enabled: bool }` struct in `supervisor/src/manifest.rs`; update TOML parser for `[[menu_items]]` array
- [ ] T059 [US5] Implement `render_dropdown(fb, items, anchor_x, anchor_y, selected)` in `supervisor/src/chrome.rs` — `fill_rect_r` background (dark 95% alpha radius=8), one `draw_glyph` row per item (13pt regular), selected row highlighted with `fill_rect_r`
- [ ] T060 [US5] Add `dropdown_open: bool` and `dropdown_selected: usize` state to chrome in `supervisor/src/chrome.rs`
- [ ] T061 [US5] In `supervisor/src/mouse_input.rs` — on left-click in menu bar active-app-name region: set `dropdown_open = true`, trigger repaint
- [ ] T062 [US5] In keyboard handler in `supervisor/src/main.rs` — when `dropdown_open`: ArrowDown/Up moves `dropdown_selected`; Enter sends `@<active_app>: <item.action>`; Escape closes dropdown
- [ ] T063 [US5] In `supervisor/src/mouse_input.rs` — detect right mouse button (button 2); if cursor not in any window: render desktop context menu at cursor; items: "New Note" → `@notes: new`, "Open Settings" → spawn settings app
- [ ] T064 [US5] Implement `ContextMenu::render(fb, items, x, y)` in `supervisor/src/chrome.rs` — same panel style as dropdown; dismiss on outside left-click
- [ ] T065 [US5] Extend `supervisor/src/toast.rs` banner layout — 320×60px panel using `fill_rect_r` (dark, 90% alpha, radius=10); `draw_image` for 16×16 app icon; `draw_glyph` 13pt bold for title; `draw_glyph` 11pt regular for body (max 120 chars)
- [ ] T066 [US5] Implement banner queue in `supervisor/src/toast.rs` — max 3 concurrent banners stacked 64px apart from top-right; each auto-dismisses after 4000ms; slide-in 200ms animation
- [ ] T067 [US5] In `supervisor/src/ipc_handlers.rs` — parse `@supervisor: notify <title>|<body>` IPC message; extract sender app name for icon lookup; enqueue `NotificationBanner`
- [ ] T068 [US5] Run `make unit-test`; test menu + notification manually via `make run-gui`; commit `feat(ui): menu bar dropdowns, context menus, notification banners`

---

## Phase 8: US6 — Form Factor Layout Profiles (P5)

**Story goal**: Six named display profiles (desktop/phone/tablet/watch/tv/vision); each boots with appropriate shell chrome and layout.

**Independent test**: Set `display_profile = "phone"` in platform profile — no dock, no menu bar, one app fills screen. Set `display_profile = "tv"` — focus ring visible on arrow-key navigation.

- [ ] T069 [P] [US6] Add `display_profile`, `show_dock`, `show_menu_bar`, `windowed_mode`, `focus_ring`, `home_gesture`, `crown_scroll`, `spatial_layout` fields to `ProfileConfig` struct in `supervisor/src/profile/loader.rs`
- [ ] T070 [P] [US6] Create `supervisor/src/profile/profiles/phone.toml`:
  ```toml
  [display]
  profile = "phone"
  show_dock = false
  show_menu_bar = false
  windowed_mode = false
  home_gesture = true
  ```
- [ ] T071 [P] [US6] Create `supervisor/src/profile/profiles/tablet.toml` (`show_dock=true`, `show_menu_bar=true`, `windowed_mode=true`)
- [ ] T072 [P] [US6] Create `supervisor/src/profile/profiles/watch.toml` (`show_dock=false`, `show_menu_bar=false`, `windowed_mode=false`, `crown_scroll=true`)
- [ ] T073 [P] [US6] Create `supervisor/src/profile/profiles/tv.toml` (`show_dock=false`, `show_menu_bar=false`, `windowed_mode=false`, `focus_ring=true`)
- [ ] T074 [P] [US6] Create `supervisor/src/profile/profiles/vision.toml` (`show_dock=false`, `show_menu_bar=false`, `windowed_mode=true`, `spatial_layout=true`, `focus_ring=true`)
- [ ] T075 [US6] Gate dock rendering in `supervisor/src/chrome.rs` behind `profile.show_dock`
- [ ] T076 [US6] Gate menu-bar rendering in `supervisor/src/chrome.rs` behind `profile.show_menu_bar`
- [ ] T077 [US6] In `supervisor/src/app_threads.rs` — when `!profile.windowed_mode`: expand first auto-started app window to full screen (`x=0, y=0, w=screen_w, h=screen_h`); hide chrome
- [ ] T078 [US6] In `supervisor/src/chrome.rs` — when `profile.focus_ring`: draw 3px rounded-rect highlight around the hovered/focused window or element on each repaint
- [ ] T079 [US6] Emit `VYOMA_SYSTEM:display_profile:<kind>` to each app at startup in `supervisor/src/app_threads.rs`
- [ ] T080 [US6] Run `make unit-test`; boot with `phone` profile to verify layout; commit `feat(profile): six Apple form-factor display profiles`

---

## Phase 9: Polish & Integration

**Purpose**: End-to-end verification, regression checks, binary size validation.

- [ ] T081 Run `make build` — verify zero new `cargo check` warnings across all crates
- [ ] T082 Run `make unit-test` — all tests pass; no previously passing tests regressed
- [ ] T083 Check supervisor binary size: `ls -lh supervisor/target/x86_64-unknown-linux-musl/release/supervisor` — must be ≤ 2.5 MB
- [ ] T084 Run `make run-gui DISPLAY_BACKEND=cocoa` (or sdl) — visual checklist:
  - Inter font visible (smooth edges) in Notes and shell
  - PNG icons in desktop grid and dock
  - Rounded window corners + traffic-light buttons
  - Drop shadows under windows
  - Spotlight frosted overlay (wallpaper visible behind panel)
  - Notification banner slides in with icon + title
  - Window open animation (scale 0.85→1.0)
- [ ] T085 Run `make smoke` — confirm `SMOKE: PASS`
- [ ] T086 Run `make check-manifests` — all manifests valid after `icon` and `menu_items` additions
- [ ] T087 Commit: `chore: Apple UI fidelity — integration verified, all checks pass`

---

## Dependencies

```
T001–T008 (Setup)
  └──► T009–T018 (Foundation: compositor + draw command parser)
         └──► Phase 3 (US1: fonts)
         └──► Phase 4 (US2: images) [parallel with US1]
         └──► Phase 5 (US3: alpha + shadows) [parallel with US1, US2]
         └──► Phase 8 (US6: profiles) [independent — no rendering deps]
                └──► Phase 6 (US4: chrome polish) [needs US1 + US3]
                └──► Phase 7 (US5: menus + banners) [needs US1 + US2 + US3]
                       └──► Phase 9 (Integration)
```

**Parallel opportunities**:
- T026, T027, T028 (chrome text migration) run in parallel
- T036 (icon PNGs per app) runs in parallel with T030–T034
- T069–T074 (six profile TOML files) all run in parallel

---

## Implementation Strategy

**MVP (US1 alone)**: Add fontdue, ship Inter font, implement `draw_glyph`, migrate chrome text. Deliverable: all text in VyomaOS is anti-aliased Inter. Verifiable without US2–US6.

**Increment 2 (+ US2)**: Add lodepng, ship icons, show them in desktop/dock. Deliverable: recognizable app icons.

**Increment 3 (+ US3)**: Alpha compositor, drop shadows, frosted Spotlight. Deliverable: visual depth — OS looks layered.

**Increment 4 (+ US6)**: Profile TOMLs, profile-conditional chrome. Deliverable: can boot in phone/watch/tv/vision layout.

**Increment 5 (+ US4)**: Rounded corners, traffic lights, animations. Deliverable: macOS-like window chrome.

**Full feature (+ US5)**: Menus, right-click, notifications. Deliverable: complete Apple-platform interaction vocabulary.
