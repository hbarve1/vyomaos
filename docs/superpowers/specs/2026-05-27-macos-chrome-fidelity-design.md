# Design: macOS Chrome Fidelity — Colors, Non-Apple Removal, Traffic Light Geometry

**Branch**: `046-apple-ui-fidelity`
**Date**: 2026-05-27
**Scope**: `supervisor/src/chrome.rs`, `supervisor/src/display/mod.rs` (background clear), `supervisor/src/draw_cmd.rs` (status bar removal)

---

## Problem

The current VyomaOS chrome does not read as macOS. Three categories of issues:

1. **Wrong color palette** — desktop background uses GitHub dark (`#0D1117`), menu bar and title bar tones don't match macOS Sonoma dark system materials.
2. **Non-Apple chrome elements** — a blue focus border, a 4th "accent dot" circle, and a per-window status strip are present. None of these exist in macOS; each one immediately breaks the platform illusion.
3. **Wrong traffic light geometry** — dots are 12×12px at 14px left margin with 10px gap, and the title text is bold weight. Apple spec is 13×13px at 9px margin with 8px gap and medium (500) weight title text.

---

## Out of Scope

- Frosted glass / vibrancy (CPU-intensive, deferred)
- Font face changes (Inter already in place; no change)
- Layout, tiling, or animation changes
- App-level drawing changes

---

## Design

### Area A — Color Palette

All color constants live in `supervisor/src/chrome.rs`. Replace the following:

| Constant | Current | Proposed | Rationale |
|----------|---------|----------|-----------|
| `MAC_MENUBAR` | `0x1C1C1EFF` | `0x2A2A2AFF` | macOS Sonoma dark menu bar material |
| `MAC_TITLE_ACT` | `0x3A3A3CFF` | `0x323232FF` | macOS active window title bar |
| `MAC_TITLE_INACT` | `0x2C2C2EFF` | `0x282828FF` | macOS inactive window title bar |
| `MAC_TITLE_HOVER` | `0x444C56FF` | `0x383838FF` | hover = slightly lighter than active (affordance) |
| `MAC_SEP` | `0x48484AFF` | `0x3A3A3CFF` | subtler separator, matches Apple HIG |
| `MAC_LABEL` (focused title) | `0xFFFFFFFF` | `0xEBEBEBFF` | macOS title text is off-white, not pure white |

Desktop background clear color in `draw_cmd.rs` flush path:

| Location | Current | Proposed |
|----------|---------|----------|
| `fill_rect(0,0,fb_w,fb_h,...)` in flush | `0x0D1117FF` | `0x1C1C1EFF` |
| Initial desktop fill in `main.rs` startup | `0x0D1117FF` | `0x1C1C1EFF` |

### Area B — Remove Non-Apple Chrome Elements

Three elements to delete, all in `supervisor/src/chrome.rs::draw_titlebar`:

**1. Focus border** — the `fb.fill_rect(wx, wy, ww, 2, bc)` line that paints a 2px `#89B4FA` stripe across the top of every focused window. Delete this line and its preceding `border_color` call.

**2. Accent dot** — the fourth `draw_rounded_rect` call at `wx + 74` that paints a per-app colored circle. Delete this block (the `app_accent_color` call and the `draw_rounded_rect` call). The `app_accent_color` helper in `display/helpers.rs` becomes unused; mark it `#[allow(dead_code)]` or delete it.

**3. Per-window status bar** — `draw_statusbar` is already marked `#[allow(dead_code)]` in `chrome.rs` and is not currently called from `repaint_all_borders` or `draw_chrome_onto` (it was gated out during an earlier refactor). Delete the `draw_statusbar` function body and the `STATUS_H: u32 = 16` constant. Any code that uses `STATUS_H` to clip or offset content must be updated to treat the window bottom as `wy + wh` (no status strip subtracted).

`STATUS_H` is referenced in:
- `chrome.rs` (definition + `draw_statusbar`)
- `draw_cmd.rs` (content clip: `win_bottom = (wy+wh).saturating_sub(STATUS_H)`)
- `app_threads.rs` (`init_surface_for_region`: `content_h = wh - chrome_h - status_h`)

In `draw_cmd.rs`, the `win_bottom` clip becomes simply `wy + wh` for all apps (system and regular).  
In `app_threads.rs`, `status_h` becomes `0` unconditionally.

### Area C — Traffic Light Geometry

In `draw_titlebar`, three changes:

**1. Dot size**: `TL_DOT: u32 = 12` → `TL_DOT: u32 = 13`

**2. Left margin**: first dot position `wx + 14` → `wx + 9`. Subsequent dots follow automatically since they use `gap = TL_DOT + 8` layout:
- Close: `wx + 9`
- Minimize: `wx + 9 + 13 + 8 = wx + 30`
- Maximize: `wx + 9 + 13 + 8 + 13 + 8 = wx + 51`

**3. Traffic light colors** (minor correction to match Apple exactly):

| Button | Current | Proposed |
|--------|---------|----------|
| Close | `0xFF5F57FF` | `0xFF6159FF` |
| Minimize | `0xFEBC2EFF` | `0xFFBD2EFF` |
| Maximize | `0x28C840FF` | `0x28C941FF` |

**4. Title font weight**: `draw_glyph_str(..., 13, true, false)` → `draw_glyph_str(..., 13, false, false)` (bold=false uses Inter Regular, which reads as medium weight at 13pt — matching macOS).

**5. Corner radius**: `draw_rounded_rect(..., 12, ...)` → `draw_rounded_rect(..., 10, ...)` in `draw_titlebar`.

---

## Files Changed

| File | Change | Net lines |
|------|--------|-----------|
| `supervisor/src/chrome.rs` | Update 6 color constants; delete focus border, accent dot, status bar call sites; fix TL positions/sizes/colors/weight/radius | −20 to −30 |
| `supervisor/src/draw_cmd.rs` | Update desktop bg clear color; remove `STATUS_H` content clip | −2 |
| `supervisor/src/main.rs` | Update startup desktop fill color | −0 (1 constant change) |
| `supervisor/src/app_threads.rs` | `status_h = 0` in `init_surface_for_region` | −1 |

All files remain well under the 500-line limit.

---

## Tests

No new test files required. Existing tests that assert on:
- `titlebar_color_for_state` return values — update expected hex values
- Any test checking `STATUS_H` or `STATUS_H`-derived offsets — update to remove the subtraction
- `chrome_metrics_test.rs` — unaffected (tests measure_str)
- `surface_test.rs`, `animator_test.rs` — unaffected

Run `make unit-test` to verify zero regressions.

---

## Acceptance Criteria

1. Desktop background is `#1C1C1E`, not `#0D1117`.
2. Menu bar background is `#2A2A2A`, separator is `#3A3A3C`.
3. Active title bar is `#323232`; inactive is `#282828`.
4. Title bar has no blue border at the top.
5. Title bar has exactly 3 traffic light circles (no 4th dot).
6. Traffic lights are 13×13px, starting at x+9 from window left edge, with 8px gaps.
7. No status strip at the bottom of any window.
8. Window corner radius is 10px (was 12px).
9. Title text uses regular weight (not bold) and color `#EBEBEB` when focused.
10. `make unit-test` passes with zero failures.
