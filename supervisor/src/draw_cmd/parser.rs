// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Individual VYOMA_DRAW command parsers (fill_rect, draw_text, draw_glyph, etc.).

use std::sync::Mutex;
use crate::lock_or_recover;

use crate::{log_error, AppRegistry};
use supervisor::logging::Subsystem;

#[cfg(target_os = "linux")]
use crate::{display, font};

/// Parse a u32 that may be decimal ("218169855") or hex ("0x0d1117ff" / "0X0D1117FF").
///
/// Delegates to the platform-independent `supervisor::parse_color` in the library crate.
#[cfg(target_os = "linux")]
#[inline]
pub fn parse_color(s: &str) -> Option<u32> {
    supervisor::parse_color(s)
}

/// Get the Surface Arc for `sender` if initialized.
#[cfg(target_os = "linux")]
fn get_surface(sender: &str, app_registry: &AppRegistry)
    -> Option<std::sync::Arc<Mutex<display::Surface>>>
{
    lock_or_recover(&app_registry).get(sender)
        .and_then(|st| lock_or_recover(&st).surface.clone())
}

#[cfg(target_os = "linux")]
pub fn parse_fill_rect(
    args: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    chrome_h: u32,
    fb_lock: &Mutex<display::Framebuffer>,
    app_registry: &AppRegistry,
) {
    let p: Vec<&str> = args.splitn(5, ',').collect();
    if let [lx_s, ly_s, w_s, h_s, rgba_s] = p.as_slice() {
        if let (Ok(lx), Ok(ly), Ok(w), Ok(h), Some(rgba)) = (
            lx_s.parse::<u32>(), ly_s.parse::<u32>(),
            w_s.parse::<u32>(),  h_s.parse::<u32>(),
            parse_color(rgba_s),
        ) {
            if let Some(surface_arc) = get_surface(sender, app_registry) {
                lock_or_recover(&surface_arc).fill_rect(lx, ly, w, h, rgba);
            } else if let Some((wx, wy, ww, wh)) = win {
                let content_wy = wy + chrome_h;
                let ax = wx + lx; let ay = content_wy + ly;
                let win_right  = wx + ww;
                let win_bottom = wy + wh;
                if ax < win_right && ay < win_bottom {
                    let aw = w.min(win_right - ax); let ah = h.min(win_bottom - ay);
                    if aw > 0 && ah > 0 { lock_or_recover(&fb_lock).fill_rect(ax, ay, aw, ah, rgba); }
                }
            } else { lock_or_recover(&fb_lock).fill_rect(lx, ly, w, h, rgba); }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad fill_rect args: {args}");
        }
    } else {
        log_error!(Subsystem::Display, Some(sender), "bad fill_rect args: {args}");
    }
}

#[cfg(target_os = "linux")]
pub fn parse_draw_text(
    args: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    chrome_h: u32,
    fb_lock: &Mutex<display::Framebuffer>,
    app_registry: &AppRegistry,
) {
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
            lock_or_recover(&surface_arc).draw_text_bitmap(lx, ly, text, rgba);
        } else if let Some((wx, wy, _, _)) = win {
            lock_or_recover(&fb_lock).draw_text(wx + lx, wy + chrome_h + ly, text, rgba, size);
        } else {
            lock_or_recover(&fb_lock).draw_text(lx, ly, text, rgba, size);
        }
    } else {
        log_error!(Subsystem::Display, Some(sender), "bad draw_text args: {args}");
    }
}

#[cfg(target_os = "linux")]
pub fn parse_rect_border(
    args: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    chrome_h: u32,
    fb_lock: &Mutex<display::Framebuffer>,
    app_registry: &AppRegistry,
) {
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
                lock_or_recover(&surface_arc).rect_border(lx, ly, w, h, rgba);
            } else if let Some((wx, wy, ww, wh)) = win {
                let content_wy = wy + chrome_h;
                let ax = wx + lx; let ay = content_wy + ly;
                let win_right  = wx + ww;
                let win_bottom = wy + wh;
                if ax < win_right && ay < win_bottom {
                    let aw = w.min(win_right - ax); let ah = h.min(win_bottom - ay);
                    if aw > 0 && ah > 0 { lock_or_recover(&fb_lock).rect_border(ax, ay, aw, ah, rgba); }
                }
            } else { lock_or_recover(&fb_lock).rect_border(lx, ly, w, h, rgba); }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad rect_border args: {args}");
        }
    } else {
        log_error!(Subsystem::Display, Some(sender), "bad rect_border args: {args}");
    }
}

#[cfg(target_os = "linux")]
pub fn parse_clear_region(
    args: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    chrome_h: u32,
    fb_lock: &Mutex<display::Framebuffer>,
    app_registry: &AppRegistry,
) {
    let parts: Vec<&str> = args.splitn(4, ',').collect();
    if parts.len() == 4 {
        if let (Ok(lx), Ok(ly), Ok(w), Ok(h)) = (
            parts[0].parse::<u32>(),
            parts[1].parse::<u32>(),
            parts[2].parse::<u32>(),
            parts[3].parse::<u32>(),
        ) {
            if let Some(surface_arc) = get_surface(sender, app_registry) {
                lock_or_recover(&surface_arc).clear_region(lx, ly, w, h);
            } else if let Some((wx, wy, ww, wh)) = win {
                let content_wy = wy + chrome_h;
                let ax = wx + lx; let ay = content_wy + ly;
                let win_right  = wx + ww; let win_bottom = wy + wh;
                if ax < win_right && ay < win_bottom {
                    let aw = w.min(win_right - ax); let ah = h.min(win_bottom - ay);
                    if aw > 0 && ah > 0 { lock_or_recover(&fb_lock).clear_region(ax, ay, aw, ah); }
                }
            } else { lock_or_recover(&fb_lock).clear_region(lx, ly, w, h); }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad clear_region args: {args}");
        }
    } else {
        log_error!(Subsystem::Display, Some(sender), "bad clear_region args: {args}");
    }
}

#[cfg(target_os = "linux")]
pub fn parse_draw_text_wrap(
    args: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    chrome_h: u32,
    fb_lock: &Mutex<display::Framebuffer>,
    app_registry: &AppRegistry,
) {
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
            if let Some(surface_arc) = get_surface(sender, app_registry) {
                // Route to surface: word-wrap then draw each line with bitmap font.
                let max_chars = (max_w / font::GLYPH_W) as usize;
                let lines = display::wrap_words(text, max_chars);
                let mut surface = lock_or_recover(&surface_arc);
                for (i, line) in lines.iter().enumerate() {
                    let line_y = ly.saturating_add(i as u32 * font::GLYPH_H);
                    if line_y >= surface.height { break; }
                    surface.draw_text_bitmap(lx, line_y, line, rgba);
                }
            } else {
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
                lock_or_recover(&fb_lock).draw_text_wrap(ax, ay, effective_max_w, text, rgba, size);
            }
        } else {
            log_error!(Subsystem::Display, Some(sender), "bad draw_text_wrap args: {args}");
        }
    } else {
        log_error!(Subsystem::Display, Some(sender), "bad draw_text_wrap args: {args}");
    }
}

#[cfg(target_os = "linux")]
pub fn parse_draw_glyph(
    args: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    chrome_h: u32,
    fb_lock: &Mutex<display::Framebuffer>,
    app_registry: &AppRegistry,
) {
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
                let mut surface = lock_or_recover(&surface_arc);
                let (sw, sh, ss) = (surface.width, surface.height, surface.stride);
                let mut cursor_x = lx as i32;
                for ch in text.chars() {
                    let (gw, gh, adv, bear_y, cov) = {
                        let mut fc = lock_or_recover(&crate::font_cache());
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
                let mut fb = lock_or_recover(&fb_lock);
                let (fb_stride, fb_width, fb_height) = (fb.stride, fb.width, fb.height);
                let mut cursor_x = ax;
                for ch in text.chars() {
                    let (gw, gh, adv, bear_y, cov) = {
                        let mut fc = lock_or_recover(&crate::font_cache());
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
}

#[cfg(target_os = "linux")]
pub fn parse_draw_image(
    args: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    chrome_h: u32,
    fb_lock: &Mutex<display::Framebuffer>,
    app_registry: &AppRegistry,
) {
    // Format: x,y,w,h,path  (splitn 5)
    let parts: Vec<&str> = args.splitn(5, ',').collect();
    if parts.len() == 5 {
        if let (Ok(lx), Ok(ly), Ok(iw), Ok(ih)) = (parts[0].parse::<u32>(),
            parts[1].parse::<u32>(), parts[2].parse::<u32>(), parts[3].parse::<u32>()) {
            let path = parts[4];
            #[cfg(target_os = "linux")]
            {
                let mut ic = lock_or_recover(&crate::image_cache());
                if let Some(img) = ic.get(path) {
                    let (iw2, ih2, img_rgba) = (img.width, img.height, img.rgba.clone());
                    drop(ic);
                    if let Some(surface_arc) = get_surface(sender, app_registry) {
                        let mut surface = lock_or_recover(&surface_arc);
                        let (sw, sh, ss) = (surface.width, surface.height, surface.stride);
                        display::blit_image(&mut surface.buf, &img_rgba, iw2, ih2, lx, ly, iw, ih, ss, sw, sh);
                    } else {
                        let (ax, ay) = win.map(|(wx, wy, _, _)| (wx + lx, wy + chrome_h + ly)).unwrap_or((lx, ly));
                        let mut fb = lock_or_recover(&fb_lock);
                        let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
                        display::blit_image(&mut fb.back, &img_rgba, iw2, ih2, ax, ay, iw, ih, fs, fw, fh);
                    }
                }
            }
            #[cfg(not(target_os = "linux"))]
            let _ = (lx, ly, iw, ih, path);
        } else { log_error!(Subsystem::Display, Some(sender), "bad draw_image args: {args}"); }
    } else { log_error!(Subsystem::Display, Some(sender), "bad draw_image args: {args}"); }
}

#[cfg(target_os = "linux")]
pub fn parse_fill_rect_r(
    args: &str,
    sender: &str,
    win: Option<(u32, u32, u32, u32)>,
    chrome_h: u32,
    fb_lock: &Mutex<display::Framebuffer>,
    app_registry: &AppRegistry,
) {
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
                    let mut surface = lock_or_recover(&surface_arc);
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
                    let mut fb = lock_or_recover(&fb_lock);
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
}
