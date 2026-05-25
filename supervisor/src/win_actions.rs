// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Window action helpers: drag state, drag update, drag-release snap, minimize/restore.
//! Extracted from mouse_input.rs to respect the 500-line file limit (spec-042).

use std::sync::{Mutex, OnceLock};

use crate::{log_info, AppRegistry, AppStatus, FocusedApp};
use crate::chrome::{draw_titlebar, TITLEBAR_H};
use supervisor::logging::Subsystem;

#[cfg(target_os = "linux")]
use crate::display;

// ── Drag state ────────────────────────────────────────────────────────────────

pub struct DragState {
    pub app_name:     String,
    pub cursor_start: (i32, i32),
    pub win_start:    (u32, u32, u32, u32),
}

static DRAG_STATE: OnceLock<Mutex<Option<DragState>>> = OnceLock::new();
pub fn drag_state() -> &'static Mutex<Option<DragState>> {
    DRAG_STATE.get_or_init(|| Mutex::new(None))
}

// ── Drag update (called on every motion event while left button held) ─────────

/// Apply the current drag delta to the registry and repaint the title bar at
/// the new position.  Returns early without painting if no drag is in progress.
#[cfg(target_os = "linux")]
pub fn apply_drag_update(
    cx: i32,
    cy: i32,
    screen_w: i32,
    screen_h: i32,
    registry: &AppRegistry,
    focused: &FocusedApp,
) {
    let ds_opt = {
        let guard = drag_state().lock().unwrap();
        guard.as_ref().map(|ds| DragState {
            app_name:     ds.app_name.clone(),
            cursor_start: ds.cursor_start,
            win_start:    ds.win_start,
        })
    };
    let Some(ds) = ds_opt else { return };

    let (dx, dy) = supervisor::windows::drag_delta(
        ds.cursor_start.0, ds.cursor_start.1, cx, cy,
    );
    let (wx, wy, ww, wh) = ds.win_start;
    let new_x = (wx as i32 + dx)
        .clamp(0, (screen_w - ww as i32).max(0)) as u32;
    let new_y = (wy as i32 + dy)
        .clamp(crate::chrome::MENUBAR_H as i32,
               (screen_h - wh as i32).max(crate::chrome::MENUBAR_H as i32)) as u32;
    let new_region = (new_x, new_y, ww, wh);
    {
        let reg = registry.lock().unwrap();
        if let Some(st_arc) = reg.get(&ds.app_name) {
            st_arc.lock().unwrap().win_region = Some(new_region);
        }
    }
    if let Some(fb_lock) = display::get() {
        let focused_name = focused.lock().unwrap().clone();
        let is_focused = focused_name.as_deref() == Some(ds.app_name.as_str());
        let mut fb = fb_lock.lock().unwrap();
        fb.fill_rect(wx, wy, ww, TITLEBAR_H, 0x0D1117FF);
        draw_titlebar(&mut *fb, new_x, new_y, ww, is_focused, false, &ds.app_name);
        fb.flush();
    }
    log_info!(Subsystem::Input, None, "drag: ({wx},{wy}) -> ({new_x},{new_y})");
}

// ── Drag release + snap-back (called on left-button release) ─────────────────

/// Finalise a drag: clear DragState and snap to the nearest tiled slot if
/// within 40 px Chebyshev distance.
#[cfg(target_os = "linux")]
pub fn finish_drag_snap(
    screen_w: i32,
    screen_h: i32,
    registry: &AppRegistry,
) {
    let ds_finished = drag_state().lock().unwrap().take();
    let Some(ds) = ds_finished else { return };

    let (wx, wy, ww, wh) = {
        let reg = registry.lock().unwrap();
        reg.get(&ds.app_name)
            .and_then(|st| st.lock().unwrap().win_region)
            .unwrap_or(ds.win_start)
    };
    let n_display = {
        let reg = registry.lock().unwrap();
        reg.values().filter(|st| {
            let st = st.lock().unwrap();
            st.has_display && matches!(st.status, AppStatus::Running)
        }).count()
    };
    let snap = supervisor::windows::nearest_tiled_slot(
        (wx, wy), n_display,
        screen_w as u32, screen_h as u32,
        crate::chrome::MENUBAR_H,
    );
    if let Some(snapped) = snap {
        let reg = registry.lock().unwrap();
        if let Some(st_arc) = reg.get(&ds.app_name) {
            st_arc.lock().unwrap().win_region = Some(snapped);
        }
        log_info!(Subsystem::Input, Some(ds.app_name.as_str()),
            "drag: snapped to tiled slot at ({},{},{},{})",
            snapped.0, snapped.1, snapped.2, snapped.3);
    }
    let _ = (ww, wh);
}

// ── Minimize (called on TrafficLight::Minimize click) ────────────────────────

/// Collapse a window to a 240×28 strip at the screen bottom and repaint.
#[cfg(target_os = "linux")]
pub fn do_minimize(
    name: &str,
    registry: &AppRegistry,
    focused: &FocusedApp,
) {
    let (scr_w, scr_h) = display::screen_size().unwrap_or((1440, 900)); // DEFAULT_SCREEN_W/H fallback
    let strip_pos: Option<(u32, u32)> = {
        let reg = registry.lock().unwrap();
        let minimized_count = reg.values()
            .filter(|st| st.lock().unwrap().minimized)
            .count() as u32;
        let strip_x_raw = minimized_count * 240;
        let row = strip_x_raw / scr_w;
        let col_x = strip_x_raw % scr_w;
        let strip_y = scr_h - 28 - row * 28;
        reg.get(name).map(|st| {
            let mut st = st.lock().unwrap();
            st.pre_minimize_region = st.win_region;
            st.minimized = true;
            st.win_region = Some((col_x, strip_y, 240, 28));
            (col_x, strip_y)
        })
    };
    if let Some((sx, sy)) = strip_pos {
        if let Some(fb_lock) = display::get() {
            let is_focused = focused.lock().unwrap().as_deref() == Some(name);
            let mut fb = fb_lock.lock().unwrap();
            draw_titlebar(&mut *fb, sx, sy, 240, is_focused, false, name);
            fb.flush();
        }
        log_info!(Subsystem::Display, Some(name), "minimized to strip ({sx},{sy})");
    }
}

// ── Strip restore (called on click landing on a minimized strip) ──────────────

/// Restore a minimized app from its strip to its pre-minimize region.
/// Returns `true` if a restore was performed (caller should `return` early).
#[cfg(target_os = "linux")]
pub fn try_restore_minimized(
    cx: i32,
    cy: i32,
    registry: &AppRegistry,
    focused: &FocusedApp,
) -> bool {
    let restore_name: Option<String> = {
        let reg = registry.lock().unwrap();
        reg.iter().find_map(|(n, st)| {
            let st = st.lock().unwrap();
            if !st.minimized { return None; }
            let Some((sx, sy, sw, sh)) = st.win_region else { return None };
            if cx >= sx as i32 && cy >= sy as i32
                && cx < (sx + sw) as i32 && cy < (sy + sh) as i32
            {
                Some(n.clone())
            } else {
                None
            }
        })
    };
    let Some(ref rname) = restore_name else { return false };

    let prev_region: Option<(u32, u32, u32, u32)> = {
        let reg = registry.lock().unwrap();
        reg.get(rname).and_then(|st| {
            let mut st = st.lock().unwrap();
            let prev = st.pre_minimize_region.take();
            if prev.is_some() {
                st.minimized = false;
                st.win_region = prev;
            }
            prev
        })
    };
    if let Some((wx, wy, ww, _wh)) = prev_region {
        if let Some(fb_lock) = display::get() {
            let is_focused = focused.lock().unwrap().as_deref() == Some(rname.as_str());
            let mut fb = fb_lock.lock().unwrap();
            draw_titlebar(&mut *fb, wx, wy, ww, is_focused, false, rname);
            fb.flush();
        }
        log_info!(Subsystem::Display, Some(rname.as_str()), "restored from minimized strip");
    }
    true
}
