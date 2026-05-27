// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VYOMA_DRAW command dispatcher (P09T01).

use std::sync::Mutex;

use crate::{log_error, log_info, log_warn, AppRegistry, FocusedApp};
use crate::chrome::{draw_chrome_onto, TITLEBAR_H, Z_DOCK};
use crate::APP_DIRTY;
use crate::flush_counts;
use supervisor::logging::Subsystem;

#[cfg(target_os = "linux")]
use crate::{display, font};

/// Get the Surface Arc for `sender` if initialized.
#[cfg(target_os = "linux")]
fn get_surface(sender: &str, app_registry: &AppRegistry)
    -> Option<std::sync::Arc<std::sync::Mutex<crate::display::Surface>>>
{
    app_registry.lock().unwrap().get(sender)
        .and_then(|st| st.lock().unwrap().surface.clone())
}

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
    let win_z = app_registry.lock().unwrap().get(sender)
        .map(|st| st.lock().unwrap().win_z).unwrap_or(10u32);
    let is_system = win_z >= Z_DOCK;
    let chrome_h = if is_system { 0u32 } else { TITLEBAR_H };

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
        let mut fb = fb_lock.lock().unwrap();
        let (fb_w, fb_h) = (fb.width, fb.height);

        // ── Compositor pass ──────────────────────────────────────────────
        // 1. Clear to desktop background
        fb.fill_rect(0, 0, fb_w, fb_h, 0x1C1C1EFF);

        // 2. Blit all app surfaces in Z-order
        let apps_with_z: Vec<(u32, String, (u32, u32, u32, u32))> = {
            let reg = app_registry.lock().unwrap();
            reg.iter()
                .filter_map(|(name, st)| {
                    let st = st.lock().unwrap();
                    st.win_region.map(|r| (st.win_z, name.clone(), r))
                })
                .collect()
        };
        let mut apps_sorted = apps_with_z;
        apps_sorted.sort_by_key(|(z, _, _)| *z);
        let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
        for (z, name, (wx, wy, ww, _wh)) in &apps_sorted {
            let surface_arc = {
                let reg = app_registry.lock().unwrap();
                reg.get(name.as_str()).and_then(|st| st.lock().unwrap().surface.clone())
            };
            if let Some(arc) = surface_arc {
                let surface = arc.lock().unwrap();
                let is_sys = *z >= Z_DOCK;
                let blit_y = if is_sys { *wy } else { wy + TITLEBAR_H };
                if *ww > 0 {
                    display::blit_surface(&mut fb.back, &surface, *wx, blit_y, 255, fs, fw, fh);
                }
            }
        }

        // 3. Draw chrome on top of composited surfaces
        draw_chrome_onto(&mut *fb, app_registry, focused);

        fb.flush();

        // FPS tracking
        {
            let mut map = flush_counts().lock().unwrap();
            let e = map.entry(sender.to_string()).or_insert((0u64, std::time::Instant::now()));
            e.0 += 1;
            let ms = e.1.elapsed().as_millis() as u64;
            if ms >= 5_000 {
                log_info!(Subsystem::Display, Some(sender), "fps: {}", display::format_fps(e.0, ms));
                *e = (0, std::time::Instant::now());
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
                if let Some(surface_arc) = get_surface(sender, app_registry) {
                    surface_arc.lock().unwrap().fill_rect(lx, ly, w, h, rgba);
                } else if let Some((wx, wy, ww, wh)) = win {
                    let content_wy = wy + chrome_h;
                    let ax = wx + lx; let ay = content_wy + ly;
                    let win_right  = wx + ww;
                    let win_bottom = wy + wh;
                    if ax < win_right && ay < win_bottom {
                        let aw = w.min(win_right - ax); let ah = h.min(win_bottom - ay);
                        if aw > 0 && ah > 0 { fb_lock.lock().unwrap().fill_rect(ax, ay, aw, ah, rgba); }
                    }
                } else { fb_lock.lock().unwrap().fill_rect(lx, ly, w, h, rgba); }
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
            if let Some(surface_arc) = get_surface(sender, app_registry) {
                surface_arc.lock().unwrap().draw_text_bitmap(lx, ly, text, rgba);
            } else if let Some((wx, wy, _, _)) = win {
                fb_lock.lock().unwrap().draw_text(wx + lx, wy + chrome_h + ly, text, rgba, size);
            } else {
                fb_lock.lock().unwrap().draw_text(lx, ly, text, rgba, size);
            }
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
                if let Some(surface_arc) = get_surface(sender, app_registry) {
                    surface_arc.lock().unwrap().rect_border(lx, ly, w, h, rgba);
                } else if let Some((wx, wy, ww, wh)) = win {
                    let content_wy = wy + chrome_h;
                    let ax = wx + lx; let ay = content_wy + ly;
                    let win_right  = wx + ww;
                    let win_bottom = wy + wh;
                    if ax < win_right && ay < win_bottom {
                        let aw = w.min(win_right - ax); let ah = h.min(win_bottom - ay);
                        if aw > 0 && ah > 0 { fb_lock.lock().unwrap().rect_border(ax, ay, aw, ah, rgba); }
                    }
                } else { fb_lock.lock().unwrap().rect_border(lx, ly, w, h, rgba); }
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
                if let Some(surface_arc) = get_surface(sender, app_registry) {
                    surface_arc.lock().unwrap().clear_region(lx, ly, w, h);
                } else if let Some((wx, wy, ww, wh)) = win {
                    let content_wy = wy + chrome_h;
                    let ax = wx + lx; let ay = content_wy + ly;
                    let win_right  = wx + ww; let win_bottom = wy + wh;
                    if ax < win_right && ay < win_bottom {
                        let aw = w.min(win_right - ax); let ah = h.min(win_bottom - ay);
                        if aw > 0 && ah > 0 { fb_lock.lock().unwrap().clear_region(ax, ay, aw, ah); }
                    }
                } else { fb_lock.lock().unwrap().clear_region(lx, ly, w, h); }
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
                        let content_wy = wy + chrome_h;
                        let ax = wx + lx;
                        let ay = content_wy + ly;
                        let content_bottom = wy + wh;
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

                #[cfg(target_os = "linux")]
                if let Some(surface_arc) = get_surface(sender, app_registry) {
                    let mut surface = surface_arc.lock().unwrap();
                    let (sw, sh, ss) = (surface.width, surface.height, surface.stride);
                    let mut cursor_x = lx as i32;
                    for ch in text.chars() {
                        let (gw, gh, adv, bear_y, cov) = {
                            let mut fc = crate::font_cache().lock().unwrap();
                            let g = fc.rasterize(ch, pt, bold, mono);
                            (g.width, g.height, g.advance_x, g.bearing_y, g.coverage.clone())
                        };
                        let glyph_y = ly as i32 - bear_y as i32;
                        display::composite_glyph(&mut surface.buf, &cov, gw, gh, cursor_x, glyph_y, rgba, ss, sw, sh);
                        cursor_x += adv as i32;
                        if cursor_x >= sw as i32 { break; }
                    }
                } else {
                    let (ax, ay) = match win {
                        None => (lx as i32, ly as i32),
                        Some((wx, wy, _, _)) => ((wx as i32 + lx as i32), (wy as i32 + chrome_h as i32 + ly as i32)),
                    };
                    let mut fb = fb_lock.lock().unwrap();
                    let (fb_stride, fb_width, fb_height) = (fb.stride, fb.width, fb.height);
                    let mut cursor_x = ax;
                    for ch in text.chars() {
                        let (gw, gh, adv, bear_y, cov) = {
                            let mut fc = crate::font_cache().lock().unwrap();
                            let g = fc.rasterize(ch, pt, bold, mono);
                            (g.width, g.height, g.advance_x, g.bearing_y, g.coverage.clone())
                        };
                        display::composite_glyph(&mut fb.back, &cov, gw, gh, cursor_x, ay - bear_y as i32, rgba, fb_stride, fb_width, fb_height);
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
                #[cfg(target_os = "linux")]
                {
                    let mut ic = crate::image_cache().lock().unwrap();
                    if let Some(img) = ic.get(path) {
                        let (iw2, ih2, img_rgba) = (img.width, img.height, img.rgba.clone());
                        drop(ic);
                        if let Some(surface_arc) = get_surface(sender, app_registry) {
                            let mut surface = surface_arc.lock().unwrap();
                            let (sw, sh, ss) = (surface.width, surface.height, surface.stride);
                            display::blit_image(&mut surface.buf, &img_rgba, iw2, ih2, lx, ly, iw, ih, ss, sw, sh);
                        } else {
                            let (ax, ay) = win.map(|(wx, wy, _, _)| (wx + lx, wy + chrome_h + ly)).unwrap_or((lx, ly));
                            let mut fb = fb_lock.lock().unwrap();
                            let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
                            display::blit_image(&mut fb.back, &img_rgba, iw2, ih2, ax, ay, iw, ih, fs, fw, fh);
                        }
                    }
                }
                #[cfg(not(target_os = "linux"))]
                let _ = (lx, ly, iw, ih, path);
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
                #[cfg(target_os = "linux")] {
                    if let Some(surface_arc) = get_surface(sender, app_registry) {
                        let mut surface = surface_arc.lock().unwrap();
                        let (sw, sh, ss) = (surface.width, surface.height, surface.stride);
                        display::draw_rounded_rect(&mut surface.buf, lx, ly, rw, rh, rgba, radius, ss, sw, sh);
                    } else {
                        let (ax, ay, aw, ah) = match win {
                            None => (lx, ly, rw, rh),
                            Some((wx, wy, ww, wh)) => {
                                let content_wy = wy + chrome_h;
                                let ax = wx + lx; let ay = content_wy + ly;
                                let win_right  = wx + ww;
                                let win_bottom = wy + wh;
                                if ax >= win_right || ay >= win_bottom { return; }
                                (ax, ay, rw.min(win_right - ax), rh.min(win_bottom - ay))
                            }
                        };
                        let mut fb = fb_lock.lock().unwrap();
                        let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
                        display::draw_rounded_rect(&mut fb.back, ax, ay, aw, ah, rgba, radius, fs, fw, fh);
                    }
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
#[cfg(not(target_os = "linux"))] fn _dummy_non_linux() { let _ = (TITLEBAR_H, Z_DOCK); }
