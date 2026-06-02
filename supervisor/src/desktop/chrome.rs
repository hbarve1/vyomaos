// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! macOS-inspired UI chrome: title bars, menu bar, status bar, Z-order helpers.

use std::sync::Mutex;
use crate::lock_or_recover;

use crate::{AppRegistry, AppStatus, FocusedApp, HOVERED_APP, Z_ORDER, BOOT_INSTANT};

#[cfg(target_os = "linux")]
use crate::display;

// ── Z-layer constants ─────────────────────────────────────────────────────────
#[allow(dead_code)] pub const Z_DESKTOP: u32 = 0;
#[allow(dead_code)] pub const Z_APP:     u32 = 10;
#[allow(dead_code)] pub const Z_DOCK:    u32 = 100;
#[allow(dead_code)] pub const Z_OVERLAY: u32 = 255;
const _: () = assert!(Z_DESKTOP < Z_APP && Z_APP < Z_DOCK && Z_DOCK < Z_OVERLAY);

// ── macOS-inspired chrome constants ──────────────────────────────────────────

pub const MENUBAR_H:    u32 = 24;   // global menu bar height (macOS standard)
pub const TITLEBAR_H:   u32 = 28;   // per-window title bar height (macOS standard)
const TL_DOT:           u32 = 12;   // traffic-light dot diameter (px)

fn mac_menubar()     -> u32 { crate::theme::current_theme().menubar }
fn mac_title_act()   -> u32 { crate::theme::current_theme().title_active }
fn mac_title_inact() -> u32 { crate::theme::current_theme().title_inactive }
fn mac_title_hover() -> u32 {
    // Hover is slightly lighter than active; derive by adding 0x060606 per channel.
    let t = crate::theme::current_theme().title_active;
    let r = ((t >> 24) & 0xFF).saturating_add(6);
    let g = ((t >> 16) & 0xFF).saturating_add(6);
    let b = ((t >>  8) & 0xFF).saturating_add(6);
    let a =  t & 0xFF;
    (r << 24) | (g << 16) | (b << 8) | a
}
fn mac_sep()         -> u32 { crate::theme::current_theme().border }
fn mac_label()       -> u32 { crate::theme::current_theme().text }
fn mac_label2()      -> u32 { crate::theme::current_theme().dim }
const TL_CLOSE:         u32 = 0xFF5F57FF; // traffic light red
const TL_MINIMIZE:      u32 = 0xFFBD2DFF; // traffic light yellow
const TL_MAXIMIZE:      u32 = 0x28C840FF; // traffic light green
const TL_GRAY:          u32 = 0x4D4D4DFF; // inactive traffic lights

// ── Focus helpers ─────────────────────────────────────────────────────────────

/// Return all windowed app names sorted alphabetically.
pub fn windowed_apps_sorted(registry: &AppRegistry) -> Vec<String> {
    let reg = lock_or_recover(&registry);
    let mut v: Vec<String> = reg.iter()
        .filter(|(_, st)| lock_or_recover(&st).win_region.is_some())
        .map(|(n, _)| n.clone()).collect();
    v.sort(); v
}

/// Advance focus to the next app in `names` (wrapping).
pub fn cycle_focus_forward(names: &[String], current: Option<&str>) -> Option<String> {
    if names.is_empty() { return None; }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(idx) => Some(names[(idx + 1) % names.len()].clone()),
        None => Some(names[0].clone()),
    }
}

/// Move focus to the previous app in `names` (wrapping).
pub fn cycle_focus_backward(names: &[String], current: Option<&str>) -> Option<String> {
    if names.is_empty() { return None; }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(0) => Some(names[names.len() - 1].clone()),
        Some(idx) => Some(names[idx - 1].clone()),
        None => Some(names[names.len() - 1].clone()),
    }
}

// ── Z-order helpers ────────────────────────────────────────────────────────────

pub fn z_order_push_front(name: &str) {
    if let Some(m) = Z_ORDER.get() {
        let mut v = lock_or_recover(&m);
        v.retain(|n| n != name);
        v.insert(0, name.to_string());
    }
}

pub fn z_order_push_back(name: &str) {
    if let Some(m) = Z_ORDER.get() {
        let mut v = lock_or_recover(&m);
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
            let mut fc = lock_or_recover(&crate::font_cache());
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
    if hovered      { mac_title_hover() } // hover — slightly lighter for affordance
    else if focused { mac_title_act()   } // active window
    else            { mac_title_inact() } // inactive window
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
    fb.fill_rect(wx, wy + TITLEBAR_H - 1, ww, 1, mac_sep());

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
        draw_rounded_rect(&mut fb.back, wx + 14, tl_y, TL_DOT, TL_DOT, c1, r, fs, sw, sh);
        draw_rounded_rect(&mut fb.back, wx + 34, tl_y, TL_DOT, TL_DOT, c2, r, fs, sw, sh);
        draw_rounded_rect(&mut fb.back, wx + 54, tl_y, TL_DOT, TL_DOT, c3, r, fs, sw, sh);
    }

    // App name centered — 14pt regular (macOS standard title font)
    let nlen = name.len().min(20);
    let display_name = &name[..nlen];
    let name_w_est = lock_or_recover(&crate::font_cache())
        .measure_str(display_name, 14, false, false);
    if ww > name_w_est + 60 {
        let nx = (wx + (ww - name_w_est) / 2) as i32;
        let ny = (wy + TITLEBAR_H / 2) as i32;
        let col = if is_focused { mac_label() } else { mac_label2() };
        draw_glyph_str(fb, display_name, nx, ny, col, 14, false, false);
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

    fb.fill_rect(0, 0, sw, MENUBAR_H, mac_menubar());
    fb.fill_rect(0, MENUBAR_H - 1, sw, 1, mac_sep());

    let ty = (MENUBAR_H / 2) as i32; // vertical center baseline for 15pt font

    // Left: brand — 15pt regular (scaled for 1440p)
    draw_glyph_str(fb, crate::i18n::t("brand"), 12, ty, mac_label(), 15, false, false);

    // Workspace indicator (e.g. "● ○ ○ ○") right after brand
    {
        let ws_indicator = crate::workspace::indicator_string();
        let ws_x = 100i32; // after "VyomaOS" label (wider at 15pt)
        draw_glyph_str(fb, &ws_indicator, ws_x, ty, mac_label2(), 12, false, false);
    }

    // App-switcher labels: drawn immediately after the brand name.
    // Highlighted (white) when focused, dimmed otherwise.
    let mut lx = MENUBAR_APPS_START_X as i32;
    for app in apps {
        let is_focused = focused.map_or(false, |f| f == app.as_str());
        let color = if is_focused { mac_label() } else { mac_label2() };
        draw_glyph_str(fb, app, lx + 8, ty, color, 15, false, false);
        lx += menubar_label_width(app.len()) as i32;
    }

    // Center: focused app name — 15pt regular (scaled for 1440p)
    if let Some(name) = focused {
        let nlen = name.len().min(20);
        let display_name = &name[..nlen];
        let name_w_est = lock_or_recover(&crate::font_cache())
            .measure_str(display_name, 15, false, false);
        let nx = sw.saturating_sub(name_w_est) / 2;
        draw_glyph_str(fb, display_name, nx as i32, ty, mac_label(), 15, false, false);
    }

    // Right: clock HH:MM:SS — 13pt regular (status text, scaled for 1440p)
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;
    let clock = format!("{h:02}:{m:02}:{s:02}");
    let cw = lock_or_recover(&crate::font_cache())
        .measure_str(&clock, 13, false, false);
    let clock_x = if sw > cw + 12 { sw - cw - 12 } else { 0 };
    if clock_x > 0 {
        draw_glyph_str(fb, &clock, clock_x as i32, ty, mac_label2(), 13, false, false);
    }

    // Tray indicators: rendered right-to-left, to the left of the clock
    let tray_items = crate::tray::collect_items();
    let tray_gap = 14_u32;
    let mut tray_x = clock_x.saturating_sub(tray_gap);
    for item in tray_items.iter().rev() {
        let tw = lock_or_recover(&crate::font_cache())
            .measure_str(&item.text, 13, false, false);
        tray_x = tray_x.saturating_sub(tw);
        if tray_x > 0 {
            draw_glyph_str(fb, &item.text, tray_x as i32, ty, item.color, 13, false, false);
        }
        tray_x = tray_x.saturating_sub(tray_gap);
    }
}

/// Immediately repaint title bars for all windowed apps (called on focus changes).
pub fn repaint_all_borders(registry: &AppRegistry, focused: &FocusedApp) {
    let focused_name = lock_or_recover(&focused).clone();
    let hovered_name = HOVERED_APP
        .get_or_init(|| Mutex::new(None))
        .lock()
        .unwrap()
        .clone();
    // Collect (win_z, name, region) then sort by win_z ascending so lower-z
    // windows are painted first (appear behind higher-z windows).
    let (regions, display_apps): (Vec<(u32, String, (u32, u32, u32, u32))>, Vec<String>) = {
        let reg = lock_or_recover(&registry);
        let mut regions_with_z: Vec<(u32, String, (u32, u32, u32, u32))> = Vec::new();
        let mut display_apps: Vec<String> = reg.iter()
            .filter_map(|(name, st)| {
                let st = lock_or_recover(&st);
                if st.has_display && matches!(st.status, AppStatus::Running) {
                    Some(name.clone())
                } else {
                    None
                }
            })
            .collect();
        display_apps.sort();
        for (name, st) in reg.iter() {
            let st = lock_or_recover(&st);
            if let Some(r) = st.win_region {
                regions_with_z.push((st.win_z, name.clone(), r));
            }
        }
        // Sort ascending by z so lowest-z windows are painted first (background first).
        regions_with_z.sort_by_key(|(z, _, _)| *z);
        (regions_with_z, display_apps)
    };
    #[cfg(target_os = "linux")]
    {
        let Some(fb_lock) = display::get() else { return };
        let mut fb = lock_or_recover(&fb_lock);
        for (z, name, (wx, wy, ww, wh)) in &regions {
            if *ww < 60 { continue; }
            // Skip chrome for system layers (dock/overlay) and desktop background.
            if *z >= Z_DOCK || *z == Z_DESKTOP { continue; }
            let is_focused = focused_name.as_deref() == Some(name.as_str());
            let is_hovered = hovered_name.as_deref() == Some(name.as_str());
            draw_titlebar(&mut *fb, *wx, *wy, *ww, is_focused, is_hovered, name);
            if is_focused {
                draw_focus_ring(&mut *fb, *wx, *wy, *ww, *wh);
            }
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
    let focused_name = lock_or_recover(&focused).clone();
    let hovered_name = lock_or_recover(HOVERED_APP
        .get_or_init(|| Mutex::new(None))).clone();
    let (regions, display_apps): (Vec<(String, (u32, u32, u32, u32))>, Vec<String>) = {
        let reg = lock_or_recover(&registry);
        let mut regions_with_z: Vec<(u32, String, (u32, u32, u32, u32))> = Vec::new();
        let mut display_apps: Vec<String> = reg.iter()
            .filter_map(|(name, st)| {
                let st = lock_or_recover(&st);
                if st.has_display && matches!(st.status, AppStatus::Running) {
                    Some(name.clone())
                } else { None }
            })
            .collect();
        display_apps.sort();
        for (name, st) in reg.iter() {
            let st = lock_or_recover(&st);
            if let Some(r) = st.win_region {
                regions_with_z.push((st.win_z, name.clone(), r));
            }
        }
        regions_with_z.sort_by_key(|(z, _, _)| *z);
        let regions = regions_with_z.into_iter().map(|(_, n, r)| (n, r)).collect();
        (regions, display_apps)
    };
    for (name, (wx, wy, ww, wh)) in &regions {
        if *ww < 60 { continue; }
        // Skip windows not on the current workspace (inline check to avoid
        // potential deadlock if registry is held by caller).
        {
            let ws_mgr = lock_or_recover(&crate::workspace::manager());
            let is_system_z = {
                let reg = lock_or_recover(&registry);
                reg.get(name.as_str()).map(|st| {
                    let z = lock_or_recover(&st).win_z;
                    z >= Z_DOCK || z == Z_DESKTOP
                }).unwrap_or(false)
            };
            if !is_system_z {
                let app_ws = ws_mgr.app_workspace.get(name.as_str()).copied().unwrap_or(0);
                if app_ws != ws_mgr.current { continue; }
            }
        }
        let is_focused = focused_name.as_deref() == Some(name.as_str());
        let is_hovered = hovered_name.as_deref() == Some(name.as_str());
        let app_z = {
            let reg = lock_or_recover(&registry);
            reg.get(name.as_str()).map(|st| lock_or_recover(&st).win_z).unwrap_or(Z_APP)
        };
        // Skip chrome for system layers (dock/overlay) and desktop background.
        if app_z < Z_DOCK && app_z != Z_DESKTOP {
            draw_titlebar(fb, *wx, *wy, *ww, is_focused, is_hovered, name);
        }
        // Draw focus ring for focused window (TV/Vision profiles).
        if is_focused {
            draw_focus_ring(fb, *wx, *wy, *ww, *wh);
        }
    }
    let sw = fb.width;
    let elapsed = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
    draw_menubar(fb, sw, elapsed, focused_name.as_deref(), &display_apps);
}

// ── Focus ring (TV / Vision profiles) ─────────────────────────────────────────

/// Draw a 3 px highlight border around the focused window when `FOCUS_RING` is enabled.
#[cfg(target_os = "linux")]
pub fn draw_focus_ring(fb: &mut display::Framebuffer, wx: u32, wy: u32, ww: u32, wh: u32) {
    if !crate::FOCUS_RING.get().copied().unwrap_or(false) { return; }
    const RING_W: u32 = 3;
    const RING_COLOR: u32 = 0x0A84FFFF; // system blue
    let (sw, sh, fs) = (fb.width, fb.height, fb.stride);
    if wy >= RING_W {
        crate::display::draw_rounded_rect(
            &mut fb.back, wx.saturating_sub(RING_W), wy.saturating_sub(RING_W),
            ww + RING_W * 2, RING_W, RING_COLOR, 6, fs, sw, sh,
        );
    }
    let by = wy + wh;
    if by + RING_W <= sh {
        crate::display::draw_rounded_rect(
            &mut fb.back, wx.saturating_sub(RING_W), by,
            ww + RING_W * 2, RING_W, RING_COLOR, 6, fs, sw, sh,
        );
    }
    if wx >= RING_W {
        crate::display::draw_rounded_rect(
            &mut fb.back, wx.saturating_sub(RING_W), wy,
            RING_W, wh, RING_COLOR, 6, fs, sw, sh,
        );
    }
    let rx = wx + ww;
    if rx + RING_W <= sw {
        crate::display::draw_rounded_rect(
            &mut fb.back, rx, wy,
            RING_W, wh, RING_COLOR, 6, fs, sw, sh,
        );
    }
}

// ── Animation alpha ──────────────────────────────────────────────────────────

/// Sample the current animation alpha and clear completed animations.
/// Returns 255 (fully opaque) when no animation is active.
/// When reduce-motion accessibility is enabled, animations are skipped
/// (immediately completed) and 255 is returned.
pub fn sample_and_clear_anim(anim: &mut Option<crate::display::animator::Animation>) -> u8 {
    let a = match anim.as_ref() {
        None => return 255,
        Some(a) => a,
    };
    // Accessibility: skip animations when reduce-motion is enabled.
    if !crate::accessibility::animation_enabled() {
        *anim = None;
        return 255;
    }
    let elapsed = crate::display::animator::now_ms().saturating_sub(a.start_ms);
    let state = a.sample(elapsed);
    if state.done { *anim = None; }
    state.alpha
}

/// Public wrapper for draw_glyph_str (used by toast.rs and menus.rs).
#[cfg(target_os = "linux")]
pub fn draw_glyph_str_pub(
    fb: &mut display::Framebuffer, text: &str,
    x: i32, y: i32, rgba: u32, pt: u32, bold: bool, mono: bool,
) { draw_glyph_str(fb, text, x, y, rgba, pt, bold, mono); }

// Re-export menu functions for callers using chrome::*
#[allow(unused_imports)]
pub use crate::menus::{
    is_dropdown_open, open_dropdown, close_dropdown, handle_dropdown_key_action,
    open_context_menu, close_context_menu, is_context_menu_open,
    dismiss_context_menu_if_outside,
};
#[cfg(target_os = "linux")]
pub use crate::menus::{render_dropdown_if_open, render_context_menu_if_open};
