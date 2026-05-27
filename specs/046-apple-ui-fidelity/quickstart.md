# Quickstart: Apple-Platform UI Fidelity

## Test Scenario 1 — Scalable Font (P1, smoke test)

**Goal**: Verify `draw_glyph` renders readable text at multiple sizes.

1. Add `fontdue` to `supervisor/Cargo.toml` and confirm `cargo check` passes
2. In `apps/notes`, replace all `VYOMA_DRAW:draw_text` calls with `VYOMA_DRAW:draw_glyph` using pt sizes
3. Run `make build && make run-gui`
4. Open Notes — text should be smooth at 14pt; zoom QEMU window to 2× — no pixel staircase

**Pass criteria**: Notes text is visibly anti-aliased. `make unit-test` still passes.

---

## Test Scenario 2 — App Icons (P2, visual check)

**Goal**: Desktop grid and dock show PNG icons.

1. Add `icon.png` (48×48) to `apps/settings/` and declare `icon = "icon.png"` in `vyoma.toml`
2. Run `make build && make run-gui`
3. Observe desktop grid cell for Settings — should show the icon image, not a text box

**Pass criteria**: Distinct icon visible in grid and dock. Screenshot shows non-solid-color pixels in icon area.

---

## Test Scenario 3 — Alpha Compositing (P3, visual check)

**Goal**: Spotlight overlay shows dimmed background.

1. Enable alpha compositing in display compositor
2. Set Spotlight panel background to `fill_rect_r` with `rgba = 0x1E1E2ECC` (80% opaque)
3. Open Spotlight over the desktop wallpaper
4. Observe: area behind panel should show wallpaper through semi-transparent overlay

**Pass criteria**: Wallpaper visible but dimmed behind the panel. Not a solid block.

---

## Test Scenario 4 — Window Animation (P4, timing test)

**Goal**: Window open animation completes in ≤300ms.

1. Instrument `apps/shell` open with `AnimKind::Open` (scale 0.85→1.0, alpha 0→255)
2. Run `make run-gui` and open shell from dock
3. Time the animation with a stopwatch or frame counter in supervisor log

**Pass criteria**: Animation completes between 200ms and 300ms. No visible freeze or snap.

---

## Test Scenario 5 — Form Factor Profile (P5, layout check)

**Goal**: Phone profile boots without dock/menu bar.

1. Set `display_profile = "phone"` in the active platform profile TOML
2. Run `make build && make run-gui`
3. Observe: no dock strip at bottom, no menu bar at top; one app fills screen

**Pass criteria**: Serial log shows `[profile] active: phone`. Screenshot shows no dock/menu bar chrome.

---

## Development Commands

```bash
# Build and boot with GUI
make build && make run-gui DISPLAY_BACKEND=cocoa  # macOS
make build && make run-gui DISPLAY_BACKEND=sdl    # Linux

# Run unit tests (includes font and compositing tests)
make unit-test

# Check binary size of supervisor after adding fontdue + lodepng
ls -lh supervisor/target/x86_64-unknown-linux-musl/release/supervisor

# Validate manifests after adding icon/menu_items fields
make check-manifests
```
