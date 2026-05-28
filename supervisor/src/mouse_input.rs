// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Mouse hardware discovery and mouse-event dispatch (P22, P32, P33).

use std::sync::Mutex;

use crate::{log_info, AppRegistry, AppStatus, FocusedApp, Inbox};
use crate::{HOVERED_APP, Z_ORDER};
use crate::chrome::{
    draw_titlebar, repaint_all_borders, z_order_push_front, TITLEBAR_H,
};
use supervisor::logging::Subsystem;

#[cfg(target_os = "linux")]
use crate::display;

// ── Traffic-light ─────────────────────────────────────────────────────────────

/// Which traffic-light button was hit.
#[derive(Debug, PartialEq)]
pub enum TrafficLight {
    Close,
    Minimize,
    Maximize,
}

/// Return the traffic-light button hit by screen point `(cx, cy)` for a window
/// whose top-left corner is at `(wx, wy)`, or `None` if no button was hit.
///
/// Layout (matches `draw_titlebar`):
///   close    at wx+ 8, tl_y  (12×12 px)
///   minimize at wx+24, tl_y  (12×12 px)
///   maximize at wx+40, tl_y  (12×12 px)
/// where tl_y = wy + (TITLEBAR_H − TL_DOT) / 2 = wy + 8.
pub fn traffic_light_hit(cx: i32, cy: i32, wx: u32, wy: u32) -> Option<TrafficLight> {
    let tl_y = wy as i32 + 8; // (TITLEBAR_H=28 - TL_DOT=12) / 2 = 8
    if cy < tl_y || cy >= tl_y + 12 {
        return None;
    }
    let wx = wx as i32;
    if cx >= wx + 8  && cx < wx + 20  { return Some(TrafficLight::Close);    }
    if cx >= wx + 24 && cx < wx + 36  { return Some(TrafficLight::Minimize); }
    if cx >= wx + 40 && cx < wx + 52  { return Some(TrafficLight::Maximize); }
    None
}

// ── Mouse event dispatch ──────────────────────────────────────────────────────

/// Dispatch a mouse event:
///  P32 — check close-button hit for all display apps; send VYOMA_SYSTEM:window_event:close
///  P33 — on click, raise topmost window under cursor in Z-order; set keyboard focus
///         dispatch VYOMA_INPUT:mouse to the topmost mouse-capable app under cursor
#[cfg(target_os = "linux")]
pub fn dispatch_mouse(
    cx: i32,
    cy: i32,
    btn: u8,
    inbox: &Inbox,
    app_registry: &AppRegistry,
    focused: &FocusedApp,
) {
    use crate::send_reply;

    // Snapshot z-order before locking registry (avoids lock ordering issues)
    let z_snapshot: Vec<String> = Z_ORDER.get()
        .map(|m| m.lock().unwrap().clone())
        .unwrap_or_default();

    // ── Title-bar hover highlight (motion only, no button press) ─────────────
    if btn == 0 {
        // Determine which app's title bar the cursor is currently over.
        let new_hover: Option<String> = {
            let reg = app_registry.lock().unwrap();
            let mut found: Option<String> = None;
            // Check z-order first so topmost window wins.
            for name in &z_snapshot {
                let Some(state_arc) = reg.get(name) else { continue };
                let st = state_arc.lock().unwrap();
                let Some((wx, wy, ww, _wh)) = st.win_region else { continue };
                // Title bar spans y in [wy, wy + TITLEBAR_H) and x in [wx, wx + ww).
                if cx >= wx as i32 && cx < (wx + ww) as i32
                    && cy >= wy as i32 && cy < (wy + TITLEBAR_H) as i32 {
                    found = Some(name.clone());
                    break;
                }
            }
            // Fallback: apps not in z_order
            if found.is_none() {
                for (name, state_arc) in reg.iter() {
                    let st = state_arc.lock().unwrap();
                    let Some((wx, wy, ww, _wh)) = st.win_region else { continue };
                    if cx >= wx as i32 && cx < (wx + ww) as i32
                        && cy >= wy as i32 && cy < (wy + TITLEBAR_H) as i32 {
                        found = Some(name.clone());
                        break;
                    }
                }
            }
            found
        };

        let old_hover = {
            HOVERED_APP
                .get_or_init(|| Mutex::new(None))
                .lock()
                .unwrap()
                .clone()
        };

        if new_hover != old_hover {
            // Update hover state.
            *HOVERED_APP.get_or_init(|| Mutex::new(None)).lock().unwrap() = new_hover.clone();

            // Collect the regions we need to redraw (previous and new hovered window).
            let focused_name = focused.lock().unwrap().clone();
            let mut to_redraw: Vec<(String, u32, u32, u32)> = Vec::new();
            {
                let reg = app_registry.lock().unwrap();
                for candidate in old_hover.iter().chain(new_hover.iter()) {
                    if let Some(state_arc) = reg.get(candidate.as_str()) {
                        let st = state_arc.lock().unwrap();
                        if let Some((wx, wy, ww, _wh)) = st.win_region {
                            if ww >= 60 {
                                to_redraw.push((candidate.clone(), wx, wy, ww));
                            }
                        }
                    }
                }
            }

            if !to_redraw.is_empty() {
                if let Some(fb_lock) = display::get() {
                    let mut fb = fb_lock.lock().unwrap();
                    for (name, wx, wy, ww) in &to_redraw {
                        let is_focused = focused_name.as_deref() == Some(name.as_str());
                        let is_hovered = new_hover.as_deref() == Some(name.as_str());
                        draw_titlebar(&mut *fb, *wx, *wy, *ww, is_focused, is_hovered, name);
                    }
                    fb.flush();
                }
            }
        }
    }

    // Menu-bar click: if the user clicks inside the MENUBAR_H-pixel band at the
    // top of the screen on an app-name label, raise and focus that app without
    // also triggering window hit-test or mouse-event dispatch.
    if btn != 0 && cy >= 0 && cy < crate::chrome::MENUBAR_H as i32 {
        use supervisor::windows::menubar_hit_app;
        let display_apps: Vec<String> = {
            let reg = app_registry.lock().unwrap();
            let mut v: Vec<String> = reg.iter()
                .filter_map(|(name, st)| {
                    let st = st.lock().unwrap();
                    if st.has_display && matches!(st.status, AppStatus::Running) {
                        Some(name.clone())
                    } else {
                        None
                    }
                })
                .collect();
            v.sort();
            v
        };
        let app_refs: Vec<&str> = display_apps.iter().map(String::as_str).collect();
        if let Some(name) = menubar_hit_app(cx, cy, crate::chrome::MENUBAR_H, &app_refs) {
            let name = name.to_string();
            z_order_push_front(&name);
            *focused.lock().unwrap() = Some(name.clone());
            log_info!(Subsystem::Input, Some(name.as_str()), "menubar click: focus → {name}");
            repaint_all_borders(app_registry, focused);
            crate::chrome::open_dropdown(&name, vec![], cx as u32, crate::chrome::MENUBAR_H);
            return;
        }
    }

    // Minimized-strip restore: check before traffic-light hit-test.
    // If the cursor lands on a minimized app's strip, restore it and return early.
    if btn != 0 && crate::win_actions::try_restore_minimized(cx, cy, app_registry, focused) {
        return;
    }

    // Traffic-light hit-test: on any click, check if a dot was hit before
    // falling through to the focus/raise and mouse-event dispatch logic.
    if btn != 0 {
        let tl_hit = {
            let reg = app_registry.lock().unwrap();
            let mut result: Option<(String, TrafficLight)> = None;
            for name in &z_snapshot {
                let Some(state_arc) = reg.get(name) else { continue };
                let st = state_arc.lock().unwrap();
                let Some((wx, wy, _ww, _wh)) = st.win_region else { continue };
                if let Some(dot) = traffic_light_hit(cx, cy, wx, wy) {
                    result = Some((name.clone(), dot));
                    break;
                }
            }
            result
        };
        if let Some((name, dot)) = tl_hit {
            match dot {
                TrafficLight::Close => {
                    // Exit watcher in app_threads.rs handles reflow + focus transfer on app death.
                    let pid = {
                        let reg = app_registry.lock().unwrap();
                        reg.get(&name).and_then(|st| st.lock().unwrap().child_pid)
                    };
                    if let Some(pid) = pid {
                        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                        log_info!(Subsystem::Lifecycle, Some(name.as_str()), "traffic-light close: killed {name} (pid {pid})");
                    }
                }
                TrafficLight::Minimize => {
                    crate::win_actions::do_minimize(&name, app_registry, focused);
                }
                TrafficLight::Maximize => {
                    log_info!(Subsystem::Display, Some(name.as_str()), "maximize requested (stub)");
                }
            }
            return;
        }
    }

    // ── Left-click: dismiss context menu (with or without hitting an item) ───
    if btn == 1 && crate::context_menu::context_menu_is_open() {
        let hit = crate::context_menu::context_menu_hit_test(cx, cy);
        crate::context_menu::close_context_menu();
        if let Some((target_app, action)) = hit {
            send_reply(&target_app, &action, inbox);
        }
        // Continue processing the click for normal focus/raise behaviour
    }

    // ── Right-click on desktop: open context menu ────────────────────────────
    if btn == 2 {
        // Check whether the cursor is inside any window.
        let inside_window: bool = {
            let reg = app_registry.lock().unwrap();
            let mut found = false;
            for name in &z_snapshot {
                let Some(state_arc) = reg.get(name) else { continue };
                let st = state_arc.lock().unwrap();
                let Some((wx, wy, ww, wh)) = st.win_region else { continue };
                if cx >= wx as i32 && cy >= wy as i32
                    && cx < (wx + ww) as i32 && cy < (wy + wh) as i32
                {
                    found = true;
                    break;
                }
            }
            found
        };
        if !inside_window {
            crate::context_menu::open_context_menu(cx as u32, cy as u32);
        }
        return;
    }

    // On click, raise topmost window under cursor and set keyboard focus
    if btn != 0 {
        let raise_target = {
            let reg = app_registry.lock().unwrap();
            let mut found: Option<String> = None;
            for name in &z_snapshot {
                let Some(state_arc) = reg.get(name) else { continue };
                let st = state_arc.lock().unwrap();
                let Some((wx, wy, ww, wh)) = st.win_region else { continue };
                if cx >= wx as i32 && cy >= wy as i32
                    && cx < (wx + ww) as i32 && cy < (wy + wh) as i32 {
                    found = Some(name.clone());
                    break;
                }
            }
            found
        };
        if let Some(ref name) = raise_target {
            z_order_push_front(name);
            *focused.lock().unwrap() = Some(name.clone());
            repaint_all_borders(app_registry, focused);
        }
    }

    // Dispatch mouse event to topmost mouse-capable app under cursor (z-order aware)
    let target = {
        let reg = app_registry.lock().unwrap();
        let mut found: Option<(String, i32, i32)> = None;
        // Try z-order first (preserves topmost-window semantics when windows overlap)
        for name in &z_snapshot {
            let Some(state_arc) = reg.get(name) else { continue };
            let st = state_arc.lock().unwrap();
            if !st.has_mouse { continue; }
            let Some((wx, wy, ww, wh)) = st.win_region else { continue };
            if cx >= wx as i32 && cy >= wy as i32
                && cx < (wx + ww) as i32 && cy < (wy + wh) as i32 {
                let lx = cx - wx as i32;
                let ly = cy - wy as i32;
                found = Some((name.clone(), lx, ly));
                break;
            }
        }
        // Fallback: apps not in z_order (no window region) still get events
        if found.is_none() {
            for (name, state_arc) in reg.iter() {
                let st = state_arc.lock().unwrap();
                if !st.has_mouse { continue; }
                let Some((wx, wy, ww, wh)) = st.win_region else { continue };
                if cx >= wx as i32 && cy >= wy as i32
                    && cx < (wx + ww) as i32 && cy < (wy + wh) as i32 {
                    let lx = cx - wx as i32;
                    let ly = cy - wy as i32;
                    found = Some((name.clone(), lx, ly));
                    break;
                }
            }
        }
        found
    };
    if let Some((name, lx, ly)) = target {
        let msg = if btn == 0 {
            format!("VYOMA_INPUT:mouse:move:{lx},{ly}")
        } else {
            let bname = match btn { 1 => "left", 2 => "right", 4 => "middle", _ => "left" };
            format!("VYOMA_INPUT:mouse:click:{lx},{ly}:{bname}")
        };
        send_reply(&name, &msg, inbox);
    }
}

// Satisfy unused-import warnings on non-Linux builds.
#[cfg(not(target_os = "linux"))]
fn _unused_log_error_import() {
    let _: fn(i32, i32, u32, u32) -> Option<TrafficLight> = traffic_light_hit;
    let _ = log_error!(Subsystem::Input, None, "placeholder");
}
