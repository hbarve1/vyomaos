// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! macOS-inspired UI chrome: title bars, menu bar, status bar, Z-order helpers.

use std::sync::Mutex;

use crate::{AppRegistry, AppStatus, FocusedApp, HOVERED_APP, Z_ORDER, BOOT_INSTANT};

#[cfg(target_os = "linux")]
use crate::display;

// ── Z-layer constants ─────────────────────────────────────────────────────────
#[allow(dead_code)]
pub const Z_DESKTOP:  u32 = 0;    // desktop wallpaper — always background
#[allow(dead_code)]
pub const Z_APP:      u32 = 10;   // default app window layer
#[allow(dead_code)]
pub const Z_DOCK:     u32 = 100;  // dock — always above app windows
#[allow(dead_code)]
pub const Z_OVERLAY:  u32 = 255;  // notifications, system overlays

// Ordering invariant: background < apps < dock < overlay
const _: () = assert!(Z_DESKTOP < Z_APP && Z_APP < Z_DOCK && Z_DOCK < Z_OVERLAY);

// ── macOS-inspired chrome constants ──────────────────────────────────────────

pub const MENUBAR_H:    u32 = 24;   // global menu bar height
pub const TITLEBAR_H:   u32 = 28;   // per-window title bar height
const TL_DOT:           u32 = 13;   // traffic-light dot size (px) — Apple spec

const MAC_MENUBAR:      u32 = 0x2A2A2AFF; // system background (menubar)
const MAC_TITLE_ACT:    u32 = 0x323232FF; // active window title bar
const MAC_TITLE_INACT:  u32 = 0x282828FF; // inactive window title bar
const MAC_TITLE_HOVER:  u32 = 0x383838FF; // hovered title bar (slightly lighter than active)
const MAC_SEP:          u32 = 0x3A3A3CFF; // separator line
const MAC_LABEL:        u32 = 0xEBEBEBFF; // primary label (near-white)
const MAC_LABEL2:       u32 = 0x8E8E93FF; // secondary label (gray)
const TL_CLOSE:         u32 = 0xFF6159FF; // traffic light red
const TL_MINIMIZE:      u32 = 0xFFBD2EFF; // traffic light yellow
const TL_MAXIMIZE:      u32 = 0x28C941FF; // traffic light green
const TL_GRAY:          u32 = 0x4D4D4DFF; // inactive traffic lights

// Kept for repaint compat; unused after chrome redesign.
#[allow(dead_code)] const BORDER_FOCUSED:   u32 = 0x89B4FAFF;
#[allow(dead_code)] const BORDER_UNFOCUSED: u32 = 0x45475AFF;

// ── Focus helpers ─────────────────────────────────────────────────────────────

/// Return all windowed app names (those with `win_region = Some(_)`) sorted alphabetically.
pub fn windowed_apps_sorted(registry: &AppRegistry) -> Vec<String> {
    let reg = registry.lock().unwrap();
    let mut v: Vec<String> = reg.iter()
        .filter(|(_, st)| st.lock().unwrap().win_region.is_some())
        .map(|(n, _)| n.clone())
        .collect();
    v.sort();
    v
}

/// Advance focus to the next app in `names` (wrapping).  If `current` is None or
/// not in `names`, return the first element.
pub fn cycle_focus_forward(names: &[String], current: Option<&str>) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(idx) => Some(names[(idx + 1) % names.len()].clone()),
        None      => Some(names[0].clone()),
    }
}

/// Move focus to the previous app in `names` (wrapping).  If `current` is None or
/// not in `names`, return the last element.
pub fn cycle_focus_backward(names: &[String], current: Option<&str>) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(0)   => Some(names[names.len() - 1].clone()),
        Some(idx) => Some(names[idx - 1].clone()),
        None      => Some(names[names.len() - 1].clone()),
    }
}

// ── Z-order helpers ────────────────────────────────────────────────────────────

pub fn z_order_push_front(name: &str) {
    if let Some(m) = Z_ORDER.get() {
        let mut v = m.lock().unwrap();
        v.retain(|n| n != name);
        v.insert(0, name.to_string());
    }
}

pub fn z_order_push_back(name: &str) {
    if let Some(m) = Z_ORDER.get() {
        let mut v = m.lock().unwrap();
        v.retain(|n| n != name);
        v.push(name.to_string());
    }
}

// ── Scalable font helper ──────────────────────────────────────────────────────

/// Render a string using the scalable font cache onto the framebuffer back-buffer.
/// Falls back silently if no font is loaded (glyphs are blank).
#[cfg(target_os = "linux")]
fn draw_glyph_str(
    fb: &mut display::Framebuffer,
    text: &str,
    x: i32,
    y: i32,
    rgba: u32,
    pt: u32,
    bold: bool,
    mono: bool,
) {
    let fb_stride = fb.stride;
    let fb_width  = fb.width;
    let fb_height = fb.height;
    let mut cx = x;
    for ch in text.chars() {
        let (gw, gh, adv, bear_y, cov) = {
            let mut fc = crate::font_cache().lock().unwrap();
            let g = fc.rasterize(ch, pt, bold, mono);
            (g.width, g.height, g.advance_x, g.bearing_y, g.coverage.clone())
        };
        display::composite_glyph(
            &mut fb.back, &cov, gw, gh,
            cx, y - bear_y as i32, rgba,
            fb_stride, fb_width, fb_height,
        );
        cx += adv as i32;
        if cx >= fb_width as i32 { break; }
    }
}

// ── Chrome drawing ────────────────────────────────────────────────────────────

/// Draw a simple drop shadow for a window by compositing two semi-transparent
/// dark rounded rects slightly offset below and around the window frame.
#[cfg(target_os = "linux")]
fn draw_window_shadow(fb: &mut display::Framebuffer, wx: u32, wy: u32, ww: u32, wh: u32) {
    use crate::display::draw_rounded_rect;
    let shadow_rgba = 0x00000078_u32; // black at ~47% alpha
    let (sw, sh) = (fb.width, fb.height);
    let fs = fb.stride;
    let sx = wx.saturating_sub(2);
    let sy = wy.saturating_add(4);
    let sw2 = ww + 4;
    let sh2 = wh + 4;
    if sx + sw2 <= sw && sy + sh2 <= sh {
        draw_rounded_rect(&mut fb.back, sx, sy, sw2, sh2, shadow_rgba, 12, fs, sw, sh);
    }
}

/// Return the title bar background colour for the given window state.
///
/// Priority: hovered > focused > inactive; `hovered` wins regardless of focus.
///
/// Pure function — no side-effects, no global state reads.
pub fn titlebar_color_for_state(focused: bool, hovered: bool) -> u32 {
    if hovered      { MAC_TITLE_HOVER } // hover — slightly lighter for affordance
    else if focused { MAC_TITLE_ACT   } // active window
    else            { MAC_TITLE_INACT } // inactive window
}

/// Draw a macOS-style title bar at (wx, wy, ww, TITLEBAR_H).
/// Traffic lights are colored when focused, gray otherwise.
/// App name is centered in the bar.
/// `is_hovered` makes the background slightly lighter for mouse-hover affordance.
#[cfg(target_os = "linux")]
pub fn draw_titlebar(
    fb: &mut display::Framebuffer,
    wx: u32, wy: u32, ww: u32,
    is_focused: bool, is_hovered: bool,
    name: &str,
) {
    // Drop shadow rendered behind the window chrome
    draw_window_shadow(fb, wx, wy, ww, TITLEBAR_H);
    let bg = titlebar_color_for_state(is_focused, is_hovered);
    {
        use crate::display::draw_rounded_rect;
        let (sw, sh, fs) = (fb.width, fb.height, fb.stride);
        draw_rounded_rect(&mut fb.back, wx, wy, ww, TITLEBAR_H, bg, 10, fs, sw, sh);
    }
    fb.fill_rect(wx, wy + TITLEBAR_H - 1, ww, 1, MAC_SEP);

    // Traffic lights — circles (radius = TL_DOT/2), left-aligned, vertically centered
    let tl_y = wy + (TITLEBAR_H - TL_DOT) / 2;
    let (c1, c2, c3) = if is_focused {
        (TL_CLOSE, TL_MINIMIZE, TL_MAXIMIZE)
    } else {
        (TL_GRAY, TL_GRAY, TL_GRAY)
    };
    {
        use crate::display::draw_rounded_rect;
        let r = TL_DOT / 2;
        let (sw, sh, fs) = (fb.width, fb.height, fb.stride);
        draw_rounded_rect(&mut fb.back, wx +  9, tl_y, TL_DOT, TL_DOT, c1, r, fs, sw, sh);
        draw_rounded_rect(&mut fb.back, wx + 30, tl_y, TL_DOT, TL_DOT, c2, r, fs, sw, sh);
        draw_rounded_rect(&mut fb.back, wx + 51, tl_y, TL_DOT, TL_DOT, c3, r, fs, sw, sh);
    }

    // App name centered — 13pt regular Inter
    let nlen = name.len().min(20);
    let display_name = &name[..nlen];
    let name_w_est = crate::font_cache().lock().unwrap()
        .measure_str(display_name, 13, false, false);
    if ww > name_w_est + 60 {
        let nx = (wx + (ww - name_w_est) / 2) as i32;
        let ny = (wy + TITLEBAR_H / 2) as i32;
        let col = if is_focused { MAC_LABEL } else { MAC_LABEL2 };
        draw_glyph_str(fb, display_name, nx, ny, col, 13, false, false);
    }
}

/// Draw the global menu bar at y=0 across the full screen width.
/// Shows "VyomaOS" on the left, app-switcher labels after it, focused app in
/// the center, and clock on the right.
///
/// `apps` is an ordered slice of display-app names drawn as clickable labels
/// immediately after the brand name.  The same ordering is used by
/// [`supervisor::windows::menubar_hit_app`] for click-to-focus hit detection.
#[cfg(target_os = "linux")]
pub fn draw_menubar(
    fb: &mut display::Framebuffer,
    sw: u32,
    elapsed_secs: u64,
    focused: Option<&str>,
    apps: &[String],
) {
    if !crate::SHOW_MENU_BAR.get().copied().unwrap_or(true) { return; }
    use supervisor::windows::{MENUBAR_APPS_START_X, menubar_label_width};

    fb.fill_rect(0, 0, sw, MENUBAR_H, MAC_MENUBAR);
    fb.fill_rect(0, MENUBAR_H - 1, sw, 1, MAC_SEP);

    let ty = (MENUBAR_H / 2) as i32; // vertical center baseline for 12pt font

    // Left: brand — 12pt regular
    draw_glyph_str(fb, "VyomaOS", 12, ty, MAC_LABEL, 12, false, false);

    // App-switcher labels: drawn immediately after the brand name.
    // Highlighted (white) when focused, dimmed otherwise.
    let mut lx = MENUBAR_APPS_START_X as i32;
    for app in apps {
        let is_focused = focused.map_or(false, |f| f == app.as_str());
        let color = if is_focused { MAC_LABEL } else { MAC_LABEL2 };
        draw_glyph_str(fb, app, lx + 8, ty, color, 12, false, false);
        lx += menubar_label_width(app.len()) as i32;
    }

    // Center: focused app name — 12pt regular
    if let Some(name) = focused {
        let nlen = name.len().min(20);
        let display_name = &name[..nlen];
        let name_w_est = crate::font_cache().lock().unwrap()
            .measure_str(display_name, 12, false, false);
        let nx = sw.saturating_sub(name_w_est) / 2;
        draw_glyph_str(fb, display_name, nx as i32, ty, MAC_LABEL, 12, false, false);
    }

    // Right: elapsed clock HH:MM:SS — 12pt regular
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;
    let clock = format!("{h:02}:{m:02}:{s:02}");
    let cw = crate::font_cache().lock().unwrap()
        .measure_str(&clock, 12, false, false);
    if sw > cw + 20 {
        draw_glyph_str(fb, &clock, (sw - cw - 12) as i32, ty, MAC_LABEL2, 12, false, false);
    }
}

/// Immediately repaint title bars for all windowed apps (called on focus changes).
pub fn repaint_all_borders(registry: &AppRegistry, focused: &FocusedApp) {
    let focused_name = focused.lock().unwrap().clone();
    let hovered_name = HOVERED_APP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .clone();
    // Collect (win_z, name, region) then sort by win_z ascending so lower-z
    // windows are painted first (appear behind higher-z windows).
    let (regions, display_apps): (Vec<(String, (u32, u32, u32, u32))>, Vec<String>) = {
        let reg = registry.lock().unwrap();
        let mut regions_with_z: Vec<(u32, String, (u32, u32, u32, u32))> = Vec::new();
        let mut display_apps: Vec<String> = reg.iter()
            .filter_map(|(name, st)| {
                let st = st.lock().unwrap();
                if st.has_display && matches!(st.status, AppStatus::Running) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
        display_apps.sort();
        for (name, st) in reg.iter() {
            let st = st.lock().unwrap();
            if let Some(r) = st.win_region {
                regions_with_z.push((st.win_z, name.clone(), r));
            }
        }
        // Sort ascending by z so lowest-z windows are painted first (background first).
        regions_with_z.sort_by_key(|(z, _, _)| *z);
        let regions = regions_with_z.into_iter().map(|(_, n, r)| (n, r)).collect();
        (regions, display_apps)
    };
    #[cfg(target_os = "linux")]
    {
        let Some(fb_lock) = display::get() else { return };
        let mut fb = fb_lock.lock().unwrap();
        for (name, (wx, wy, ww, _wh)) in &regions {
            if *ww < 60 { continue; }
            let is_focused = focused_name.as_deref() == Some(name.as_str());
            let is_hovered = hovered_name.as_deref() == Some(name.as_str());
            draw_titlebar(&mut *fb, *wx, *wy, *ww, is_focused, is_hovered, name);
        }
        let sw = fb.width;
        let elapsed = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
        draw_menubar(&mut *fb, sw, elapsed, focused_name.as_deref(), &display_apps);
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = regions;
        let _ = display_apps;
        let _ = focused_name;
        let _ = hovered_name;
    }
}

/// Draw chrome (title bars + menu bar) onto a framebuffer that is already locked.
/// Call from within code that holds the `fb_lock` — avoids deadlock with repaint_all_borders.
#[cfg(target_os = "linux")]
pub fn draw_chrome_onto(
    fb: &mut display::Framebuffer,
    registry: &AppRegistry,
    focused: &FocusedApp,
) {
    let focused_name = focused.lock().unwrap().clone();
    let hovered_name = HOVERED_APP
        .get_or_init(|| Mutex::new(None))
        .lock().unwrap().clone();
    let (regions, display_apps): (Vec<(String, (u32, u32, u32, u32))>, Vec<String>) = {
        let reg = registry.lock().unwrap();
        let mut regions_with_z: Vec<(u32, String, (u32, u32, u32, u32))> = Vec::new();
        let mut display_apps: Vec<String> = reg.iter()
            .filter_map(|(name, st)| {
                let st = st.lock().unwrap();
                if st.has_display && matches!(st.status, AppStatus::Running) {
                    Some(name.clone())
                } else { None }
            })
            .collect();
        display_apps.sort();
        for (name, st) in reg.iter() {
            let st = st.lock().unwrap();
            if let Some(r) = st.win_region {
                regions_with_z.push((st.win_z, name.clone(), r));
            }
        }
        regions_with_z.sort_by_key(|(z, _, _)| *z);
        let regions = regions_with_z.into_iter().map(|(_, n, r)| (n, r)).collect();
        (regions, display_apps)
    };
    let dock_visible = crate::SHOW_DOCK.get().copied().unwrap_or(true);
    for (name, (wx, wy, ww, _wh)) in &regions {
        if *ww < 60 { continue; }
        let is_focused = focused_name.as_deref() == Some(name.as_str());
        let is_hovered = hovered_name.as_deref() == Some(name.as_str());
        let is_system = {
            let reg = registry.lock().unwrap();
            reg.get(name.as_str()).map(|st| st.lock().unwrap().win_z >= Z_DOCK).unwrap_or(false)
        };
        if is_system {
            // Dock-layer windows: only render chrome when dock is enabled.
            if !dock_visible { continue; }
        } else {
            draw_titlebar(fb, *wx, *wy, *ww, is_focused, is_hovered, name);
        }
    }
    let sw = fb.width;
    let elapsed = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
    draw_menubar(fb, sw, elapsed, focused_name.as_deref(), &display_apps);
}

// ── T056: Animation alpha stub ────────────────────────────────────────────────

/// Sample the current animation alpha for an app state.
/// Returns 255 (fully opaque) when no animation is active.
/// Full layer-alpha blending requires a compositor not yet built;
/// this helper is the integration point for future frame-tick use.
#[allow(dead_code)]
pub fn sample_anim_alpha(anim: &Option<crate::display::animator::Animation>) -> u8 {
    match anim {
        None => 255,
        Some(a) => {
            let elapsed = crate::display::animator::now_ms()
                .saturating_sub(a.start_ms);
            a.sample(elapsed).alpha
        }
    }
}

// ── T059–T064: Dropdown menu state ───────────────────────────────────────────

struct DropdownState { open: bool, selected: usize, items: Vec<(String, String)>, anchor_x: u32, anchor_y: u32, app_name: String }
static DROPDOWN_STATE: std::sync::OnceLock<Mutex<DropdownState>> = std::sync::OnceLock::new();
fn dropdown_state() -> &'static Mutex<DropdownState> {
    DROPDOWN_STATE.get_or_init(|| Mutex::new(DropdownState { open: false, selected: 0, items: vec![], anchor_x: 0, anchor_y: 0, app_name: String::new() }))
}

/// Open the app-name dropdown at anchor position.
pub fn open_dropdown(app: &str, items: Vec<(String, String)>, ax: u32, ay: u32) {
    let mut s = dropdown_state().lock().unwrap();
    s.open = true; s.selected = 0; s.items = items; s.anchor_x = ax; s.anchor_y = ay; s.app_name = app.to_string();
}
/// Close the dropdown without dispatching any action.
#[allow(dead_code)]
pub fn close_dropdown() { dropdown_state().lock().unwrap().open = false; }

/// Return true when the dropdown is currently open.
///
/// Used by the keyboard router in `input_keys` to intercept navigation
/// keys before they are forwarded to the focused app's stdin.
pub fn dropdown_is_open() -> bool {
    dropdown_state().lock().unwrap().open
}

/// Handle a keyboard navigation event while the dropdown is open.
///
/// Key byte mapping (spec T062):
///   `0x42` — ArrowDown  — move selection down (clamp at last item)
///   `0x41` — ArrowUp    — move selection up   (clamp at first item)
///   `0x0A` — Enter      — confirm selection, close dropdown,
///                          returns `Some((app_name, action))` for IPC dispatch
///   `0x1B` — Escape     — close dropdown without action, returns `None`
///
/// All other keys are ignored.  When the dropdown is closed this is a no-op.
///
/// # Return value
/// `Some((app_name, action))` when Enter is pressed and the dropdown was open;
/// the caller must send `@<app_name>: <action>` via the IPC inbox.
/// `None` in all other cases.
pub fn handle_dropdown_key(key: u8) -> Option<(String, String)> {
    let mut s = match dropdown_state().try_lock() { Ok(s) => s, Err(_) => return None };
    if !s.open { return None; }
    match key {
        0x42 => { // ArrowDown
            if s.selected + 1 < s.items.len() { s.selected += 1; }
            None
        }
        0x41 => { // ArrowUp
            if s.selected > 0 { s.selected -= 1; }
            None
        }
        0x0A => { // Enter — confirm and close
            let result = s.items.get(s.selected)
                .map(|(_, action)| (s.app_name.clone(), action.clone()));
            s.open = false;
            result
        }
        0x1B => { // Escape — discard and close
            s.open = false;
            None
        }
        _ => None,
    }
}
/// Public wrapper for draw_glyph_str (used by toast.rs banner renderer).
#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub fn draw_glyph_str_pub(fb: &mut display::Framebuffer, text: &str, x: i32, y: i32, rgba: u32, pt: u32, bold: bool, mono: bool) {
    draw_glyph_str(fb, text, x, y, rgba, pt, bold, mono);
}
/// Render the dropdown menu if open.
#[cfg(target_os = "linux")]
#[allow(dead_code)]
pub fn render_dropdown_if_open(fb: &mut display::Framebuffer) {
    let s = dropdown_state().lock().unwrap();
    if !s.open || s.items.is_empty() { return; }
    let (ax, ay, items, selected) = (s.anchor_x, s.anchor_y, s.items.clone(), s.selected);
    drop(s);
    use crate::display::draw_rounded_rect;
    let row_h: u32 = 22; let (pw, fs, fw, fh) = (200u32, fb.stride, fb.width, fb.height);
    draw_rounded_rect(&mut fb.back, ax, ay, pw, row_h * items.len() as u32 + 8, 0x1C1C1EE8_u32, 8, fs, fw, fh);
    for (i, (label, _)) in items.iter().enumerate() {
        let ry = ay + 4 + i as u32 * row_h;
        if i == selected { draw_rounded_rect(&mut fb.back, ax + 4, ry, pw - 8, row_h - 2, 0x0A84FFFF_u32, 4, fs, fw, fh); }
        draw_glyph_str(fb, label, (ax + 12) as i32, (ry + row_h / 2) as i32, 0xFFFFFFFF, 13, false, false);
    }
}
