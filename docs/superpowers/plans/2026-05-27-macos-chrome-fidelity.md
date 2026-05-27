# macOS Chrome Fidelity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Align VyomaOS chrome colors, geometry, and element set with macOS Sonoma dark mode — fixing the palette, removing three non-Apple elements, and correcting traffic light dimensions/positions.

**Architecture:** All changes are in-place edits to existing files. No new files, no new infrastructure. The three tasks are ordered so each compiles and passes tests independently. `mouse_test.rs` mirrors color constants from `chrome.rs` and must be kept in sync.

**Tech Stack:** Rust stable, `supervisor` crate. Tests run via `cargo test --target x86_64-unknown-linux-musl` inside Docker (`make unit-test`) or locally if musl toolchain is available.

---

## File Map

| File | Change |
|------|--------|
| `supervisor/src/chrome.rs` | Update 6 color constants; delete focus border, accent dot, `draw_statusbar`, `STATUS_H`; fix traffic light positions/sizes/colors/weight; fix corner radius |
| `supervisor/src/display/helpers.rs` | `border_color` and `app_accent_color` kept as public API (still tested); no deletion needed |
| `supervisor/src/draw_cmd.rs` | Update desktop bg clear color; remove `STATUS_H` from import + 5 call sites; update `_dummy_non_linux` |
| `supervisor/src/main.rs` | Update startup desktop fill color |
| `supervisor/src/app_threads.rs` | Remove `STATUS_H` from import; drop `status_h` variable in `init_surface_for_region` |
| `supervisor/tests/mouse_test.rs` | Update 3 mirrored color constants to match new values |

---

## Task 1: Color Palette

**Files:**
- Modify: `supervisor/src/chrome.rs` — 6 constant lines
- Modify: `supervisor/tests/mouse_test.rs` — 3 mirrored constant lines
- Modify: `supervisor/src/draw_cmd.rs` — 1 line (flush desktop bg)
- Modify: `supervisor/src/main.rs` — 1 line (startup desktop bg)
- Modify: `supervisor/src/app_threads.rs` — 1 line (exit desktop bg clear)

- [ ] **Step 1: Update the mirrored color constants in `mouse_test.rs` first (TDD — these are the test assertions)**

In `supervisor/tests/mouse_test.rs`, lines 203–205, replace the three constants:

```rust
const MAC_TITLE_ACT:   u32 = 0x323232FF; // active / focused
const MAC_TITLE_INACT: u32 = 0x282828FF; // inactive / unfocused
const MAC_TITLE_HOVER: u32 = 0x383838FF; // hovered (slightly lighter than active)
```

- [ ] **Step 2: Run the titlebar-color tests to confirm they now FAIL (constants changed but source not yet)**

```bash
cd supervisor && cargo test titlebar_color --target x86_64-unknown-linux-musl 2>&1 | tail -20
```

Expected: 4 failures — `titlebar_color_hovered_returns_hover_color`, `titlebar_color_focused_not_hovered_returns_active_color`, `titlebar_color_unfocused_not_hovered_returns_inactive_color`, `titlebar_color_hover_wins_over_focus`.

- [ ] **Step 3: Update the 6 color constants in `supervisor/src/chrome.rs`**

Lines 30–41, replace the constant block:

```rust
const MAC_MENUBAR:      u32 = 0x2A2A2AFF; // macOS Sonoma dark menu bar
const MAC_TITLE_ACT:    u32 = 0x323232FF; // active window title bar
const MAC_TITLE_INACT:  u32 = 0x282828FF; // inactive window title bar
const MAC_TITLE_HOVER:  u32 = 0x383838FF; // hovered title bar
const MAC_SEP:          u32 = 0x3A3A3CFF; // separator line
const MAC_LABEL:        u32 = 0xEBEBEBFF; // primary label (off-white, not pure white)
const MAC_LABEL2:       u32 = 0x8E8E93FF; // secondary label (unchanged)
const TL_CLOSE:         u32 = 0xFF6159FF; // traffic light red
const TL_MINIMIZE:      u32 = 0xFFBD2EFF; // traffic light yellow
const TL_MAXIMIZE:      u32 = 0x28C941FF; // traffic light green
const TL_GRAY:          u32 = 0x4D4D4DFF; // inactive traffic lights (unchanged)
```

- [ ] **Step 4: Run titlebar-color tests — expect PASS**

```bash
cd supervisor && cargo test titlebar_color --target x86_64-unknown-linux-musl 2>&1 | tail -10
```

Expected: 4 tests pass.

- [ ] **Step 5: Update the desktop background clear color in `supervisor/src/draw_cmd.rs`**

Line 76, change:

```rust
fb.fill_rect(0, 0, fb_w, fb_h, 0x0D1117FF);
```

to:

```rust
fb.fill_rect(0, 0, fb_w, fb_h, 0x1C1C1EFF);
```

- [ ] **Step 6: Update the startup desktop fill in `supervisor/src/main.rs`**

Line 279, change:

```rust
fb.fill_rect(0, 0, w, h, 0x0D1117FF);
```

to:

```rust
fb.fill_rect(0, 0, w, h, 0x1C1C1EFF);
```

- [ ] **Step 7: Update the app-exit region clear in `supervisor/src/app_threads.rs`**

Line 398, change:

```rust
fb.fill_rect(wx, wy, ww, wh, 0x0D1117FF);
```

to:

```rust
fb.fill_rect(wx, wy, ww, wh, 0x1C1C1EFF);
```

- [ ] **Step 8: Run full test suite — zero failures**

```bash
cd supervisor && cargo test --target x86_64-unknown-linux-musl 2>&1 | tail -15
```

Expected: all tests pass, zero warnings from changed lines.

- [ ] **Step 9: Commit**

```bash
git add supervisor/src/chrome.rs supervisor/src/draw_cmd.rs \
        supervisor/src/main.rs supervisor/src/app_threads.rs \
        supervisor/tests/mouse_test.rs
git commit -m "fix(chrome): update color palette to macOS Sonoma dark — menu bar, title bars, separator, label, bg"
```

---

## Task 2: Remove Non-Apple Chrome Elements

**Files:**
- Modify: `supervisor/src/chrome.rs` — delete focus border block, accent dot block, `draw_statusbar` fn, `STATUS_H` constant
- Modify: `supervisor/src/draw_cmd.rs` — remove `STATUS_H` from import + 5 `win_bottom` sites + `_dummy_non_linux`
- Modify: `supervisor/src/app_threads.rs` — remove `STATUS_H` from import + `status_h` variable

- [ ] **Step 1: Delete the focus border block from `draw_titlebar` in `supervisor/src/chrome.rs`**

Remove these 4 lines (around line 188–191):

```rust
    // 2px focus border — top edge only (full frame drawn at flush time when wh is known)
    let bc = display::border_color(is_focused);
    fb.fill_rect(wx, wy, ww, 2, bc);

```

(The blank line after the block should also go. `border_color` in `display/helpers.rs` remains — it is a public function with its own tests.)

- [ ] **Step 2: Delete the accent dot block from `draw_titlebar` in `supervisor/src/chrome.rs`**

Remove these 9 lines (around line 208–216):

```rust
    // Accent color dot — circle, deterministic per-app identity marker (12×12 at x+74)
    let accent = display::app_accent_color(name);
    {
        use crate::display::draw_rounded_rect;
        let r = TL_DOT / 2;
        let (sw, sh, fs) = (fb.width, fb.height, fb.stride);
        draw_rounded_rect(&mut fb.back, wx + 74, tl_y, TL_DOT, TL_DOT, accent, r, fs, sw, sh);
    }

```

(`app_accent_color` in `display/helpers.rs` remains — public, tested.)

- [ ] **Step 3: Delete `STATUS_H` constant and `draw_statusbar` function from `supervisor/src/chrome.rs`**

Remove line 27:

```rust
pub const STATUS_H:     u32 = 16;   // per-window status strip height (bottom of window)
```

Remove the entire `draw_statusbar` function (lines 290–313, approximately):

```rust
/// Draw a 16px status strip at the very bottom of a window's chrome.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub fn draw_statusbar(
    fb:          &mut display::Framebuffer,
    name:        &str,
    uptime_secs: u64,
    wx:          u32,
    wy:          u32,
    ww:          u32,
    wh:          u32,
) {
    const STATUS_BG: u32 = 0x161B22FF;
    const STATUS_FG: u32 = 0x8B949EFF;
    let sy = wy + wh - STATUS_H;
    fb.fill_rect(wx, sy, ww, STATUS_H, STATUS_BG);
    let label = supervisor::statusbar::format_status_text(name, uptime_secs);
    let text_y = (sy + STATUS_H / 2) as i32;
    draw_glyph_str(fb, &label, (wx + 4) as i32, text_y, STATUS_FG, 11, false, false);
}
```

- [ ] **Step 4: Remove `STATUS_H` from the import in `supervisor/src/draw_cmd.rs`**

Line 9, change:

```rust
use crate::chrome::{draw_chrome_onto, STATUS_H, TITLEBAR_H, Z_DOCK};
```

to:

```rust
use crate::chrome::{draw_chrome_onto, TITLEBAR_H, Z_DOCK};
```

- [ ] **Step 5: Replace the 5 `STATUS_H` win_bottom expressions in `supervisor/src/draw_cmd.rs`**

Each occurrence of:

```rust
let win_bottom = if is_system { wy + wh } else { (wy + wh).saturating_sub(STATUS_H) };
```

Replace with:

```rust
let win_bottom = wy + wh;
```

Run this search to find all 5:

```bash
grep -n "saturating_sub(STATUS_H)" supervisor/src/draw_cmd.rs
```

Expected: 5 matches (lines ~139, ~221, ~250, ~283, ~415). Replace all 5.

The `content_bottom` variant on ~line 283 reads:

```rust
let content_bottom = if is_system { wy + wh } else { (wy + wh).saturating_sub(STATUS_H) };
```

Replace with:

```rust
let content_bottom = wy + wh;
```

- [ ] **Step 6: Update `_dummy_non_linux` in `supervisor/src/draw_cmd.rs`**

The last line of the file reads:

```rust
#[cfg(not(target_os = "linux"))] fn _dummy_non_linux() { let _ = (STATUS_H, TITLEBAR_H); }
```

Change to:

```rust
#[cfg(not(target_os = "linux"))] fn _dummy_non_linux() { let _ = (TITLEBAR_H, Z_DOCK); }
```

- [ ] **Step 7: Remove `STATUS_H` from `supervisor/src/app_threads.rs`**

Line 34, change:

```rust
use crate::chrome::{TITLEBAR_H, STATUS_H, Z_DOCK};
```

to:

```rust
use crate::chrome::{TITLEBAR_H, Z_DOCK};
```

Lines 37–39, replace:

```rust
    let is_system = st.win_z >= Z_DOCK;
    let chrome_h = if is_system { 0u32 } else { TITLEBAR_H };
    let status_h = if is_system { 0u32 } else { STATUS_H };
    let content_h = wh.saturating_sub(chrome_h + status_h);
```

with:

```rust
    let is_system = st.win_z >= Z_DOCK;
    let chrome_h = if is_system { 0u32 } else { TITLEBAR_H };
    let content_h = wh.saturating_sub(chrome_h);
```

- [ ] **Step 8: Verify line counts stay under 500**

```bash
wc -l supervisor/src/chrome.rs supervisor/src/draw_cmd.rs supervisor/src/app_threads.rs
```

Expected: all three under 500.

- [ ] **Step 9: Run full test suite — zero failures**

```bash
cd supervisor && cargo test --target x86_64-unknown-linux-musl 2>&1 | tail -15
```

Expected: all pass. The `border_color_*` tests and `app_accent_color_*` tests still pass because those functions still exist in `display/helpers.rs`.

- [ ] **Step 10: Commit**

```bash
git add supervisor/src/chrome.rs supervisor/src/draw_cmd.rs supervisor/src/app_threads.rs
git commit -m "fix(chrome): remove focus border, accent dot, and per-window status bar (non-Apple elements)"
```

---

## Task 3: Traffic Light Geometry + Title Bar Polish

**Files:**
- Modify: `supervisor/src/chrome.rs` — `TL_DOT` size, 3 x-positions, corner radius, title font weight + color; update `measure_str` call to match

- [ ] **Step 1: Update `TL_DOT` size and traffic light colors in `supervisor/src/chrome.rs`**

Line 28, change:

```rust
const TL_DOT:           u32 = 12;   // traffic-light dot size (px)
```

to:

```rust
const TL_DOT:           u32 = 13;   // traffic-light dot size (px) — Apple spec
```

(Colors were already updated in Task 1 Step 3.)

- [ ] **Step 2: Fix the corner radius in `draw_titlebar`**

Find the `draw_rounded_rect` call that draws the title bar background (the one with `bg` as the color argument). Change the radius argument from `12` to `10`:

```rust
draw_rounded_rect(&mut fb.back, wx, wy, ww, TITLEBAR_H, bg, 10, fs, sw, sh);
```

- [ ] **Step 3: Fix the three traffic light x-positions**

In `draw_titlebar`, find the three `draw_rounded_rect` calls that draw the traffic lights. Change from:

```rust
draw_rounded_rect(&mut fb.back, wx + 14, tl_y, TL_DOT, TL_DOT, c1, r, fs, sw, sh);
draw_rounded_rect(&mut fb.back, wx + 34, tl_y, TL_DOT, TL_DOT, c2, r, fs, sw, sh);
draw_rounded_rect(&mut fb.back, wx + 54, tl_y, TL_DOT, TL_DOT, c3, r, fs, sw, sh);
```

to:

```rust
draw_rounded_rect(&mut fb.back, wx +  9, tl_y, TL_DOT, TL_DOT, c1, r, fs, sw, sh);
draw_rounded_rect(&mut fb.back, wx + 30, tl_y, TL_DOT, TL_DOT, c2, r, fs, sw, sh);
draw_rounded_rect(&mut fb.back, wx + 51, tl_y, TL_DOT, TL_DOT, c3, r, fs, sw, sh);
```

(9px left margin, 13px dot, 8px gap → second at 9+13+8=30, third at 30+13+8=51.)

- [ ] **Step 4: Fix title font weight and measurement in `draw_titlebar`**

Find the title rendering block (two lines that call `measure_str` and `draw_glyph_str` with `13, true, false`). Change both `true` (bold) arguments to `false` (regular weight):

```rust
    let name_w_est = crate::font_cache().lock().unwrap()
        .measure_str(display_name, 13, false, false);
    if ww > name_w_est + 60 {
        let nx = (wx + (ww - name_w_est) / 2) as i32;
        let ny = (wy + TITLEBAR_H / 2) as i32;
        let col = if is_focused { MAC_LABEL } else { MAC_LABEL2 };
        draw_glyph_str(fb, display_name, nx, ny, col, 13, false, false);
    }
```

(`MAC_LABEL` was updated to `0xEBEBEBFF` in Task 1, so focused title text is now correct off-white.)

- [ ] **Step 5: Run full test suite — zero failures**

```bash
cd supervisor && cargo test --target x86_64-unknown-linux-musl 2>&1 | tail -15
```

Expected: all tests pass.

- [ ] **Step 6: Verify acceptance criteria**

```bash
grep -n "TL_DOT\|wx + 9\|wx + 30\|wx + 51\|0x1C1C1E\|0x2A2A2A\|0x323232\|0x282828" \
  supervisor/src/chrome.rs supervisor/src/draw_cmd.rs supervisor/src/main.rs
```

Expected: each new value appears in the expected file.

```bash
grep -n "STATUS_H\|0x0D1117\|wx + 14\|wx + 34\|wx + 54\|accent_color\|border_color\|fill_rect.*wx.*wy.*2" \
  supervisor/src/chrome.rs supervisor/src/draw_cmd.rs supervisor/src/app_threads.rs
```

Expected: none of these appear in those files (all removed).

- [ ] **Step 7: Commit**

```bash
git add supervisor/src/chrome.rs
git commit -m "fix(chrome): Apple-spec traffic lights (13px, 9/30/51px), corner radius 10px, title medium weight"
```

---

## Acceptance Checklist

After all three tasks, verify all ten spec criteria:

```bash
# 1. Desktop bg = #1C1C1E
grep "0x1C1C1EFF" supervisor/src/draw_cmd.rs supervisor/src/main.rs supervisor/src/app_threads.rs

# 2. Menu bar = #2A2A2A, separator = #3A3A3C
grep "0x2A2A2AFF\|0x3A3A3CFF" supervisor/src/chrome.rs

# 3. Title active = #323232, inactive = #282828
grep "0x323232FF\|0x282828FF" supervisor/src/chrome.rs

# 4-5. No focus border, no accent dot
grep -n "border_color\|app_accent_color\|wx + 74" supervisor/src/chrome.rs
# Expected: zero matches (functions still exist in helpers.rs but not called here)

# 6. No status strip
grep -n "STATUS_H\|draw_statusbar" supervisor/src/chrome.rs supervisor/src/draw_cmd.rs supervisor/src/app_threads.rs
# Expected: zero matches

# 7. Traffic lights: 13px, 9/30/51
grep -n "TL_DOT.*13\|wx +  9\|wx + 30\|wx + 51" supervisor/src/chrome.rs

# 8. Corner radius 10
grep -n "TITLEBAR_H, bg, 10" supervisor/src/chrome.rs

# 9. Title medium weight
grep -n "draw_glyph_str.*13, false" supervisor/src/chrome.rs

# 10. Tests pass
cd supervisor && cargo test --target x86_64-unknown-linux-musl 2>&1 | grep -E "^test result"
```
