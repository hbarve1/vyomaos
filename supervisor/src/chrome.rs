// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! macOS-inspired UI chrome: title bars, menu bar, status bar, Z-order helpers.

use std::sync::Mutex;

use crate::{AppRegistry, AppStatus, FocusedApp, HOVERED_APP, Z_ORDER, BOOT_INSTANT};
use supervisor::logging::Subsystem;

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

// ── macOS-inspired chrome constants ──────────────────────────────────────────

pub const MENUBAR_H:    u32 = 24;   // global menu bar height
pub const TITLEBAR_H:   u32 = 28;   // per-window title bar height
pub const STATUS_H:     u32 = 16;   // per-window status strip height (bottom of window)
const TL_DOT:           u32 = 12;   // traffic-light dot size (px)

const MAC_MENUBAR:      u32 = 0x1C1C1EFF; // system background (menubar)
const MAC_TITLE_ACT:    u32 = 0x3A3A3CFF; // active window title bar
const MAC_TITLE_INACT:  u32 = 0x2C2C2EFF; // inactive window title bar
const MAC_TITLE_HOVER:  u32 = 0x444C56FF; // hovered title bar (midpoint between active and inactive)
const MAC_SEP:          u32 = 0x48484AFF; // separator line
const MAC_LABEL:        u32 = 0xFFFFFFFF; // primary label (white)
const MAC_LABEL2:       u32 = 0x8E8E93FF; // secondary label (gray)
const TL_CLOSE:         u32 = 0xFF5F57FF; // traffic light red
const TL_MINIMIZE:      u32 = 0xFEBC2EFF; // traffic light yellow
const TL_MAXIMIZE:      u32 = 0x28C840FF; // traffic light green
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
    fb.fill_rect(wx, wy, ww, TITLEBAR_H, bg);
    fb.fill_rect(wx, wy + TITLEBAR_H - 1, ww, 1, MAC_SEP);

    // 2px focus border — top edge only (full frame drawn at flush time when wh is known)
    let bc = display::border_color(is_focused);
    fb.fill_rect(wx, wy, ww, 2, bc);

    // Traffic lights — 12×12, left-aligned, vertically centered
    let tl_y = wy + (TITLEBAR_H - TL_DOT) / 2;
    let (c1, c2, c3) = if is_focused {
        (TL_CLOSE, TL_MINIMIZE, TL_MAXIMIZE)
    } else {
        (TL_GRAY, TL_GRAY, TL_GRAY)
    };
    fb.fill_rect(wx + 8,  tl_y, TL_DOT, TL_DOT, c1);
    fb.fill_rect(wx + 24, tl_y, TL_DOT, TL_DOT, c2);
    fb.fill_rect(wx + 40, tl_y, TL_DOT, TL_DOT, c3);

    // Accent color dot — deterministic per-app identity marker (12×12 at x+60)
    let accent = display::app_accent_color(name);
    fb.fill_rect(wx + 60, tl_y, TL_DOT, TL_DOT, accent);

    // App name centered — 13pt bold Inter
    let nlen = name.len().min(20);
    let display_name = &name[..nlen];
    // Estimate width: ~7px per char at 13pt for centering heuristic
    let name_w_est = nlen as u32 * 7;
    if ww > name_w_est + 60 {
        let nx = (wx + (ww - name_w_est) / 2) as i32;
        let ny = (wy + TITLEBAR_H / 2) as i32;
        let col = if is_focused { MAC_LABEL } else { MAC_LABEL2 };
        draw_glyph_str(fb, display_name, nx, ny, col, 13, true, false);
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
        let name_w_est = nlen as u32 * 7;
        let nx = sw.saturating_sub(name_w_est) / 2;
        draw_glyph_str(fb, display_name, nx as i32, ty, MAC_LABEL, 12, false, false);
    }

    // Right: elapsed clock HH:MM:SS — 12pt regular
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;
    let clock = format!("{h:02}:{m:02}:{s:02}");
    let cw = clock.len() as u32 * 7; // ~7px/char at 12pt
    if sw > cw + 20 {
        draw_glyph_str(fb, &clock, (sw - cw - 12) as i32, ty, MAC_LABEL2, 12, false, false);
    }
}

/// Draw a 16px status strip at the very bottom of a window's chrome.
/// Background: `0x161B22FF` (dark).  Text: `0x8B949EFF` (grey).
/// Shows `[name]  up <uptime_secs>s` in small font, left-aligned with 4px inset.
#[cfg(target_os = "linux")]
pub fn draw_statusbar(
    fb:          &mut display::Framebuffer,
    name:        &str,
    uptime_secs: u64,
    wx:          u32,
    wy:          u32,
    ww:          u32,
    wh:          u32,
) {
    const STATUS_BG: u32 = 0x161B22FF; // very dark navy background
    const STATUS_FG: u32 = 0x8B949EFF; // muted grey text

    // Fill the status strip
    let sy = wy + wh - STATUS_H;
    fb.fill_rect(wx, sy, ww, STATUS_H, STATUS_BG);

    // Build and draw the label — 11pt regular Inter, vertically centered
    let label = supervisor::statusbar::format_status_text(name, uptime_secs);
    let text_y = (sy + STATUS_H / 2) as i32;
    draw_glyph_str(fb, &label, (wx + 4) as i32, text_y, STATUS_FG, 11, false, false);
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

/// Recompute tiled regions for all running display apps and write them into
/// the registry. Called on every display-app spawn or exit.
#[allow(dead_code)]
pub fn apply_tiling_layout(registry: &AppRegistry) {
    use supervisor::windows::compute_tiling_with_hints;

    let apps: Vec<(String, u32, u32)> = {
        let reg = registry.lock().unwrap();
        let mut v: Vec<(String, u32, u32)> = reg.iter()
            .filter_map(|(name, st)| {
                let st = st.lock().unwrap();
                if st.has_display && matches!(st.status, AppStatus::Running) {
                    Some((name.clone(), st.min_size.0, st.min_size.1))
                } else {
                    None
                }
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    };

    if apps.is_empty() { return; }

    const DEFAULT_SCREEN_W: u32 = 1440;
    const DEFAULT_SCREEN_H: u32 = 900;
    #[cfg(target_os = "linux")]
    let (sw, sh) = crate::display::screen_size().unwrap_or((DEFAULT_SCREEN_W, DEFAULT_SCREEN_H));
    #[cfg(not(target_os = "linux"))]
    let (sw, sh) = (DEFAULT_SCREEN_W, DEFAULT_SCREEN_H);

    let min_sizes: Vec<(u32, u32)> = apps.iter().map(|(_, mw, mh)| (*mw, *mh)).collect();
    let usable_h = sh.saturating_sub(MENUBAR_H);
    let regions: Vec<(u32, u32, u32, u32)> = compute_tiling_with_hints(apps.len(), sw, usable_h, &min_sizes)
        .into_iter()
        .map(|(x, y, w, h)| (x, y + MENUBAR_H, w, h))
        .collect();

    {
        let reg = registry.lock().unwrap();
        for (i, (name, _, _)) in apps.iter().enumerate() {
            if let Some(st) = reg.get(name) {
                if let Some(&region) = regions.get(i) {
                    let mut st = st.lock().unwrap();
                    st.win_region = Some(region);
                    crate::log_info!(Subsystem::Display, Some(name.as_str()),
                        "tiling: assigned ({},{},{},{})", region.0, region.1, region.2, region.3);
                }
            }
        }
    }

    let n = apps.len();
    crate::log_info!(Subsystem::Display, None, "layout reflow: {n} display app(s) tiled");
}
