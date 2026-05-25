// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Mouse hardware discovery and mouse-event dispatch (P22, P32, P33).

use std::sync::Mutex;

use crate::{log_info, log_error, AppRegistry, AppStatus, FocusedApp, Inbox};
use crate::{HOVERED_APP, Z_ORDER};
use crate::chrome::{
    draw_titlebar, repaint_all_borders, z_order_push_front, TITLEBAR_H,
};
use crate::win_actions::{drag_state, DragState};
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

// ── Mouse device discovery ────────────────────────────────────────────────────

/// Find the first /dev/input/eventN that supports pointer events (EV_REL or EV_ABS).
/// Uses EVIOCGBIT(0, 1) ioctl — returns 1 byte of event-type capability bitmask.
/// Bit 2 = EV_REL (relative mouse), bit 3 = EV_ABS (absolute pointer, virtio-mouse-pci).
/// Returns None on headless boot where no pointer input devices exist.
#[cfg(target_os = "linux")]
pub fn open_mouse_device() -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    // EVIOCGBIT(0, 1) = _IOC(_IOC_READ=2, 'E'=0x45, nr=0x20, size=1)
    //                 = (2<<30)|(0x45<<8)|0x20|(1<<16) = 0x80014520
    // libc::Ioctl is i32 on musl/x86-64 and u64 on glibc — use `as _` to coerce.
    const EVIOCGBIT_TYPE: u32 = 0x80014520u32;
    for i in 0..8u32 {
        let path = format!("/dev/input/event{i}");
        let Ok(f) = std::fs::File::open(&path) else { continue };
        let mut bits = 0u8;
        let ret = unsafe {
            libc::ioctl(
                f.as_raw_fd(),
                EVIOCGBIT_TYPE as _,
                &mut bits as *mut u8 as *mut libc::c_void,
            )
        };
        if ret >= 0 && (bits & (1 << 2) != 0 || bits & (1 << 3) != 0) {
            log_info!(Subsystem::Input, None, "mouse-input: using {path}");
            return Some(f);
        }
    }
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

/// P22: body of the mouse-input thread — /dev/input/eventN → dispatch.
///
/// Only compiled on Linux.
#[cfg(target_os = "linux")]
pub fn run_mouse_input(inbox: Inbox, focused: FocusedApp, registry: AppRegistry) {
    use std::io::Read;

    let Some(mut dev) = open_mouse_device() else {
        log_info!(Subsystem::Input, None, "mouse-input: no pointer device found, disabling");
        return;
    };

    display::enable_cursor();

    const EV_SYN: u16    = 0;
    const EV_KEY: u16    = 1;
    const EV_REL: u16    = 2;
    const EV_ABS: u16    = 3;
    const REL_X: u16     = 0;
    const REL_Y: u16     = 1;
    const ABS_X: u16     = 0;
    const ABS_Y: u16     = 1;
    const BTN_LEFT: u16  = 0x110;
    const BTN_RIGHT: u16 = 0x111;
    const BTN_MID: u16   = 0x112;
    const ABS_MAX: i64   = 32768;

    let (sw, sh) = display::screen_size().unwrap_or((1440, 900)); // DEFAULT_SCREEN_W/H fallback
    let screen_w: i32 = sw as i32;
    let screen_h: i32 = sh as i32;

    let mut cx: i32 = screen_w / 2;
    let mut cy: i32 = screen_h / 2;
    let mut pending_click_mask:   u8 = 0;
    let mut pending_release_mask: u8 = 0;
    let mut btn_held:             u8 = 0;
    let mut pending_abs_x: Option<i32> = None;
    let mut pending_abs_y: Option<i32> = None;
    let mut pending_dx:    i32 = 0;
    let mut pending_dy:    i32 = 0;

    let mut buf = [0u8; 24];
    loop {
        if dev.read_exact(&mut buf).is_err() {
            log_error!(Subsystem::Input, None, "mouse-input: device read error, exiting");
            break;
        }
        let ev_type = u16::from_ne_bytes([buf[16], buf[17]]);
        let code    = u16::from_ne_bytes([buf[18], buf[19]]);
        let value   = i32::from_ne_bytes([buf[20], buf[21], buf[22], buf[23]]);

        match ev_type {
            EV_REL => match code {
                REL_X => pending_dx += value,
                REL_Y => pending_dy += value,
                _     => {}
            },
            EV_ABS => match code {
                ABS_X => pending_abs_x = Some(value),
                ABS_Y => pending_abs_y = Some(value),
                _     => {}
            },
            EV_KEY => {
                let bit = match code {
                    BTN_LEFT  => 1u8,
                    BTN_RIGHT => 2u8,
                    BTN_MID   => 4u8,
                    _         => 0u8,
                };
                if value == 1 { pending_click_mask |= bit; }
                else if value == 0 && bit != 0 { pending_release_mask |= bit; }
            }
            EV_SYN => {
                let mut pos_changed = false;
                if pending_dx != 0 || pending_dy != 0 {
                    let new_cx = (cx + pending_dx).clamp(0, screen_w - 1);
                    let new_cy = (cy + pending_dy).clamp(0, screen_h - 1);
                    pos_changed = new_cx != cx || new_cy != cy;
                    cx = new_cx; cy = new_cy;
                    pending_dx = 0; pending_dy = 0;
                }
                if let Some(ax) = pending_abs_x.take() {
                    let new_cx = (ax as i64 * screen_w as i64 / ABS_MAX) as i32;
                    if new_cx != cx { pos_changed = true; cx = new_cx; }
                }
                if let Some(ay) = pending_abs_y.take() {
                    let new_cy = (ay as i64 * screen_h as i64 / ABS_MAX) as i32;
                    if new_cy != cy { pos_changed = true; cy = new_cy; }
                }
                display::set_cursor_pos(cx, cy);
                if pending_click_mask != 0 {
                    if pending_click_mask & 1 != 0 {
                        *crate::mouse_drag_start().lock().unwrap() = Some((cx, cy));
                        // Initiate drag: find title-bar app under cursor (excluding traffic lights)
                        let z_snap: Vec<String> = crate::Z_ORDER.get()
                            .map(|m| m.lock().unwrap().clone()).unwrap_or_default();
                        let dragged: Option<(String, (u32, u32, u32, u32))> = {
                            let reg = registry.lock().unwrap();
                            let mut found = None;
                            for name in &z_snap {
                                let Some(st_arc) = reg.get(name) else { continue };
                                let st = st_arc.lock().unwrap();
                                let Some((wx, wy, ww, _wh)) = st.win_region else { continue };
                                if cy >= wy as i32 && cy < (wy + TITLEBAR_H) as i32
                                    && cx >= wx as i32 && cx < (wx + ww) as i32
                                    && traffic_light_hit(cx, cy, wx, wy).is_none()
                                {
                                    found = Some((name.clone(), st.win_region.unwrap()));
                                    break;
                                }
                            }
                            found
                        };
                        if let Some((name, region)) = dragged {
                            *drag_state().lock().unwrap() = Some(DragState {
                                app_name: name, cursor_start: (cx, cy), win_start: region,
                            });
                        }
                    }
                    btn_held |= pending_click_mask;
                    dispatch_mouse(cx, cy, pending_click_mask, &inbox, &registry, &focused);
                    pending_click_mask = 0;
                } else if pos_changed {
                    if btn_held & 1 != 0 {
                        crate::win_actions::apply_drag_update(
                            cx, cy, screen_w, screen_h, &registry, &focused,
                        );
                    }
                    dispatch_mouse(cx, cy, 0, &inbox, &registry, &focused);
                }
                if pending_release_mask != 0 {
                    if pending_release_mask & 1 != 0 {
                        *crate::mouse_drag_start().lock().unwrap() = None;
                        // Drag release: snap to nearest tiled slot if within 40 px
                        crate::win_actions::finish_drag_snap(screen_w, screen_h, &registry);
                    }
                    btn_held &= !pending_release_mask;
                    pending_release_mask = 0;
                }
            }
            _ => {}
        }
    }
}

// Satisfy unused-import warnings on non-Linux builds.
#[cfg(not(target_os = "linux"))]
fn _unused_log_error_import() {
    let _: fn(&str, i32, u32, u32) -> Option<TrafficLight> = traffic_light_hit;
    let _ = log_error!(Subsystem::Input, None, "placeholder");
}
