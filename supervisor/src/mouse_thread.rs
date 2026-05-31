// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Mouse hardware discovery and the P22 mouse-input thread.
//! Extracted from mouse_input.rs to keep that file under 500 lines.

use crate::{log_info, log_error, AppRegistry, FocusedApp, Inbox};
use crate::mouse_input::{dispatch_mouse, traffic_light_hit};
use crate::win_actions::{drag_state, DragState};
use crate::chrome::TITLEBAR_H;
use supervisor::logging::Subsystem;

#[cfg(target_os = "linux")]
use crate::display;

/// Find the first /dev/input/eventN that supports pointer events (EV_REL or EV_ABS).
/// Uses EVIOCGBIT(0, 1) ioctl — returns 1 byte of event-type capability bitmask.
/// Bit 2 = EV_REL (relative mouse), bit 3 = EV_ABS (absolute pointer, virtio-mouse-pci).
/// Returns None on headless boot where no pointer input devices exist.
#[cfg(target_os = "linux")]
pub fn open_mouse_device() -> Option<std::fs::File> {
    use std::os::unix::io::AsRawFd;
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

    let (sw, sh) = display::screen_size().unwrap_or((1440, 900));
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
                        crate::win_actions::finish_drag_snap(screen_w, screen_h, &registry, &focused);
                    }
                    btn_held &= !pending_release_mask;
                    pending_release_mask = 0;
                }
            }
            _ => {}
        }
    }
}

// Non-Linux stub to avoid unused-import warnings.
#[cfg(not(target_os = "linux"))]
fn _unused() {
    let _ = log_error!(Subsystem::Input, None, "placeholder");
}
