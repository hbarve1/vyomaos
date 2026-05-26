// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VYOMA_DRAW command dispatcher (P09T01).

use std::sync::Mutex;

use crate::{log_error, log_info, log_warn, AppRegistry, FocusedApp};
use crate::chrome::{draw_menubar, draw_statusbar, draw_titlebar, STATUS_H, TITLEBAR_H};
use crate::{BOOT_INSTANT, LAST_MENUBAR_DRAW, APP_DIRTY, HOVERED_APP};
use crate::flush_counts;
use supervisor::logging::Subsystem;

#[cfg(target_os = "linux")]
use crate::{display, font};

/// Parse a u32 that may be decimal ("218169855") or hex ("0x0d1117ff" / "0X0D1117FF").
#[cfg(target_os = "linux")]
#[inline]
pub fn parse_color(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

#[cfg(target_os = "linux")]
pub fn handle_draw_command(
    cmd: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    focused: &FocusedApp,
    app_registry: &AppRegistry,
) {
    // Skip draw commands for minimized windows (content area is hidden).
    {
        let reg = app_registry.lock().unwrap();
        if reg.get(sender).map(|st| st.lock().unwrap().minimized).unwrap_or(false) {
            return;
        }
    }

    let Some(fb_lock) = display::get() else { return };

    // Mark this app dirty for any draw command other than flush/present.
    // The flush handler checks and clears this flag before repainting the title bar.
    if cmd != "flush" && cmd != "present" {
        APP_DIRTY
            .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
            .lock()
            .unwrap()
            .insert(sender.to_string(), true);
    }

    if cmd == "flush" || cmd == "present" {
        let focused_name = focused.lock().unwrap().clone();
        let is_focused = focused_name.as_deref() == Some(sender);
        let mut fb = fb_lock.lock().unwrap();

        // Per-window macOS-style title bar — only repaint if this app has
        // issued draw commands since its last flush (dirty flag).
        let is_dirty = {
            let mut dirty_map = APP_DIRTY
                .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
                .lock()
                .unwrap();
            let dirty = dirty_map.get(sender).copied().unwrap_or(false);
            if dirty {
                dirty_map.insert(sender.to_string(), false);
            }
            dirty
        };

        if is_dirty {
            if let Some((wx, wy, ww, wh)) = win {
                if ww >= 60 {
                    let is_hovered = HOVERED_APP
                        .get_or_init(|| Mutex::new(None))
                        .lock()
                        .unwrap()
                        .as_deref() == Some(sender);
                    draw_titlebar(&mut *fb, wx, wy, ww, is_focused, is_hovered, sender);
                }
                // Per-window status strip — redrawn on every flush so uptime advances.
                if ww > 0 && wh > STATUS_H {
                    let uptime_secs: u64 = {
                        let reg = app_registry.lock().unwrap();
                        reg.get(sender)
                            .map(|st| st.lock().unwrap().start_time.elapsed().as_secs())
                            .unwrap_or(0)
                    };
                    draw_statusbar(&mut *fb, sender, uptime_secs, wx, wy, ww, wh);
                }
            }
        }

        // Global menu bar — only repaint when ≥1 second has elapsed since the
        // last draw OR the focused app name has changed.
        let sw = fb.width;
        let elapsed = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
        let should_draw_menubar = {
            let mut last = LAST_MENUBAR_DRAW
                .get_or_init(|| {
                    Mutex::new((std::time::Instant::now() - std::time::Duration::from_secs(2), None))
                })
                .lock()
                .unwrap();
            let (ref mut last_instant, ref mut last_focused) = *last;
            let time_elapsed = last_instant.elapsed() >= std::time::Duration::from_secs(1);
            let focus_changed = last_focused.as_deref() != focused_name.as_deref();
            if time_elapsed || focus_changed {
                *last_instant = std::time::Instant::now();
                *last_focused = focused_name.clone();
                true
            } else {
                false
            }
        };

        if should_draw_menubar {
            let display_apps: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut v: Vec<String> = reg.iter()
                    .filter_map(|(name, st)| {
                        let st = st.lock().unwrap();
                        if st.has_display && matches!(st.status, crate::AppStatus::Running) {
                            Some(name.clone())
                        } else {
                            None
                        }
                    })
                    .collect();
                v.sort();
                v
            };
            draw_menubar(&mut *fb, sw, elapsed, focused_name.as_deref(), &display_apps);
        }

        fb.flush();

        // ── FPS tracking ─────────────────────────────────────────────────────
        {
            const FPS_WINDOW_MS: u64 = 5_000;
            let mut map = flush_counts().lock().unwrap();
            let entry = map
                .entry(sender.to_string())
                .or_insert_with(|| (0, std::time::Instant::now()));
            entry.0 += 1;
            let elapsed_ms = entry.1.elapsed().as_millis() as u64;
            if elapsed_ms >= FPS_WINDOW_MS {
                let fps_str = display::format_fps(entry.0, elapsed_ms);
                log_info!(Subsystem::Display, Some(sender), "fps: {fps_str}");
                *entry = (0, std::time::Instant::now());
            }
        }

        return;
    }

    if let Some(args) = cmd.strip_prefix("fill_rect:") {
        let p: Vec<&str> = args.splitn(5, ',').collect();
        if let [lx_s, ly_s, w_s, h_s, rgba_s] = p.as_slice() {
            if let (Ok(lx), Ok(ly), Ok(w), Ok(h), Some(rgba)) = (
                lx_s.parse::<u32>(), ly_s.parse::<u32>(),
                w_s.parse::<u32>(),  h_s.parse::<u32>(),
                parse_color(rgba_s),
            ) {
                let (ax, ay, aw, ah) = match win {
                    None => (lx, ly, w, h),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        let win_right  = wx + ww;
                        // Clip content bottom to exclude the status strip.
                        let win_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = w.min(win_right  - ax);
                        let ah = h.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                fb_lock.lock().unwrap().fill_rect(ax, ay, aw, ah, rgba);
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad fill_rect args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad fill_rect args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text:") {
        // Try new format first: x,y,rgba,size,text  (5 comma-fields, size = s/m/l)
        // Fall back to legacy:   x,y,rgba,text       (4 comma-fields, size = Medium)
        let parts5: Vec<&str> = args.splitn(5, ',').collect();
        let parts4: Vec<&str> = args.splitn(4, ',').collect();

        let parsed = if parts5.len() == 5 {
            if let (Ok(lx), Ok(ly), Some(rgba), Some(sz)) = (
                parts5[0].parse::<u32>(),
                parts5[1].parse::<u32>(),
                parse_color(parts5[2]),
                font::parse_size(parts5[3]),
            ) {
                Some((lx, ly, rgba, sz, parts5[4]))
            } else {
                None
            }
        } else {
            None
        };

        let parsed = parsed.or_else(|| {
            if parts4.len() == 4 {
                if let (Ok(lx), Ok(ly), Some(rgba)) = (
                    parts4[0].parse::<u32>(),
                    parts4[1].parse::<u32>(),
                    parse_color(parts4[2]),
                ) {
                    Some((lx, ly, rgba, font::FontSize::Medium, parts4[3]))
                } else {
                    None
                }
            } else {
                None
            }
        });

        if let Some((lx, ly, rgba, size, text)) = parsed {
            let (ax, ay) = match win {
                None => (lx, ly),
                Some((wx, wy, ww, wh)) => {
                    let content_wy = wy + TITLEBAR_H;
                    let ax = wx + lx;
                    let ay = content_wy + ly;
                    // Clip content bottom to exclude the status strip.
                    let content_bottom = (wy + wh).saturating_sub(STATUS_H);
                    if ax >= wx + ww || ay >= content_bottom { return; }
                    (ax, ay)
                }
            };
            fb_lock.lock().unwrap().draw_text(ax, ay, text, rgba, size);
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad draw_text args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("rect_border:") {
        let parts: Vec<&str> = args.splitn(5, ',').collect();
        if parts.len() == 5 {
            if let (Ok(lx), Ok(ly), Ok(w), Ok(h), Some(rgba)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parts[3].parse::<u32>(),
                parse_color(parts[4]),
            ) {
                let (ax, ay, aw, ah) = match win {
                    None => (lx, ly, w, h),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        let win_right  = wx + ww;
                        // Clip content bottom to exclude the status strip.
                        let win_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = w.min(win_right  - ax);
                        let ah = h.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                fb_lock.lock().unwrap().rect_border(ax, ay, aw, ah, rgba);
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad rect_border args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad rect_border args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("clear_region:") {
        let parts: Vec<&str> = args.splitn(4, ',').collect();
        if parts.len() == 4 {
            if let (Ok(lx), Ok(ly), Ok(w), Ok(h)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parts[3].parse::<u32>(),
            ) {
                let (ax, ay, aw, ah) = match win {
                    None => (lx, ly, w, h),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        let win_right  = wx + ww;
                        // Clip content bottom to exclude the status strip.
                        let win_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = w.min(win_right  - ax);
                        let ah = h.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                fb_lock.lock().unwrap().clear_region(ax, ay, aw, ah);
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad clear_region args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad clear_region args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text_wrap:") {
        // Format: x,y,max_w,rgba,size,text  (splitn 6)
        let parts: Vec<&str> = args.splitn(6, ',').collect();
        if parts.len() == 6 {
            if let (Ok(lx), Ok(ly), Ok(max_w), Some(rgba), Some(size)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parse_color(parts[3]),
                font::parse_size(parts[4]),
            ) {
                let text = parts[5];
                let (ax, ay, effective_max_w) = match win {
                    None => (lx, ly, max_w),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        // Clip content bottom to exclude the status strip.
                        let content_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= wx + ww || ay >= content_bottom { return; }
                        let effective_max_w = max_w.min(ww.saturating_sub(lx));
                        if effective_max_w == 0 { return; }
                        (ax, ay, effective_max_w)
                    }
                };
                fb_lock.lock().unwrap().draw_text_wrap(ax, ay, effective_max_w, text, rgba, size);
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad draw_text_wrap args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad draw_text_wrap args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_glyph:") {
        // Format: x,y,rgba,pt,weight,text  (splitn 6)
        let parts: Vec<&str> = args.splitn(6, ',').collect();
        if parts.len() == 6 {
            if let (Ok(lx), Ok(ly), Some(rgba), Ok(pt)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parse_color(parts[2]),
                parts[3].parse::<u32>(),
            ) {
                let weight = parts[4];
                let text = parts[5];
                let bold = weight == "bold";
                let mono = weight == "mono";

                let (ax, ay) = match win {
                    None => (lx as i32, ly as i32),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx as i32 + lx as i32;
                        let ay = content_wy as i32 + ly as i32;
                        if ax >= (wx + ww) as i32 || ay >= (wy + wh) as i32 { return; }
                        (ax, ay)
                    }
                };

                #[cfg(target_os = "linux")]
                {
                    let mut fb = fb_lock.lock().unwrap();
                    let fb_stride = fb.stride;
                    let fb_width  = fb.width;
                    let fb_height = fb.height;
                    let mut cursor_x = ax;
                    for ch in text.chars() {
                        let (gw, gh, adv, bear_y, cov) = {
                            let mut fc = crate::font_cache().lock().unwrap();
                            let g = fc.rasterize(ch, pt, bold, mono);
                            (g.width, g.height, g.advance_x, g.bearing_y, g.coverage.clone())
                        };
                        let glyph_y = ay - bear_y as i32;
                        display::composite_glyph(
                            &mut fb.back, &cov, gw, gh,
                            cursor_x, glyph_y, rgba,
                            fb_stride, fb_width, fb_height,
                        );
                        cursor_x += adv as i32;
                        if cursor_x >= fb_width as i32 { break; }
                    }
                }
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad draw_glyph args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad draw_glyph args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_image:") {
        // Format: x,y,w,h,path  (splitn 5)
        let parts: Vec<&str> = args.splitn(5, ',').collect();
        if parts.len() == 5 {
            if let (Ok(lx), Ok(ly), Ok(iw), Ok(ih)) = (parts[0].parse::<u32>(),
                parts[1].parse::<u32>(), parts[2].parse::<u32>(), parts[3].parse::<u32>()) {
                let path = parts[4];
                let (ax, ay) = match win {
                    None => (lx, ly),
                    Some((wx, wy, _, _)) => (wx + lx, wy + TITLEBAR_H + ly),
                };
                #[cfg(target_os = "linux")]
                {
                    let mut fb = fb_lock.lock().unwrap();
                    let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
                    let mut ic = crate::image_cache().lock().unwrap();
                    if let Some(img) = ic.get(path) {
                        let (iw2, ih2, rgba) = (img.width, img.height, img.rgba.clone());
                        drop(ic);
                        display::blit_image(&mut fb.back, &rgba, iw2, ih2, ax, ay, iw, ih, fs, fw, fh);
                    }
                }
                #[cfg(not(target_os = "linux"))]
                let _ = (ax, ay, iw, ih, path);
            } else { log_error!(Subsystem::Display, Some(sender), "bad draw_image args: {args}"); }
        } else { log_error!(Subsystem::Display, Some(sender), "bad draw_image args: {args}"); }
        return;
    }

    if let Some(args) = cmd.strip_prefix("fill_rect_r:") {
        // Format: x,y,w,h,rgba,radius  (splitn 6)
        let parts: Vec<&str> = args.splitn(6, ',').collect();
        if parts.len() == 6 {
            if let (Ok(lx), Ok(ly), Ok(rw), Ok(rh), Some(rgba), Ok(radius)) = (
                parts[0].parse::<u32>(),
                parts[1].parse::<u32>(),
                parts[2].parse::<u32>(),
                parts[3].parse::<u32>(),
                parse_color(parts[4]),
                parts[5].parse::<u32>(),
            ) {
                let (ax, ay, aw, ah) = match win {
                    None => (lx, ly, rw, rh),
                    Some((wx, wy, ww, wh)) => {
                        let content_wy = wy + TITLEBAR_H;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        let win_right  = wx + ww;
                        let win_bottom = (wy + wh).saturating_sub(STATUS_H);
                        if ax >= win_right || ay >= win_bottom { return; }
                        let aw = rw.min(win_right  - ax);
                        let ah = rh.min(win_bottom - ay);
                        if aw == 0 || ah == 0 { return; }
                        (ax, ay, aw, ah)
                    }
                };
                #[cfg(target_os = "linux")] {
                    let mut fb = fb_lock.lock().unwrap();
                    let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
                    display::draw_rounded_rect(&mut fb.back, ax, ay, aw, ah, rgba, radius, fs, fw, fh);
                }
            } else {
                log_error!(Subsystem::Display, Some(sender), "bad fill_rect_r args: {args}");
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad fill_rect_r args: {args}");
        }
        return;
    }

    if let Some(args) = cmd.strip_prefix("set_layer_alpha:") {
        if let Ok(alpha) = args.trim().parse::<u8>() {
            // TODO US4: apply animation alpha to compositor layer
            let _ = alpha;
        }
        return;
    }

    log_warn!(Subsystem::Display, Some(sender), "unknown command: {cmd}");
}

// Satisfy unused-import warnings on non-Linux builds.
#[cfg(not(target_os = "linux"))] fn _dummy_non_linux() { let _ = (STATUS_H, TITLEBAR_H); }
