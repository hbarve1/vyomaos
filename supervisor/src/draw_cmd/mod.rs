// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VYOMA_DRAW command dispatcher (P09T01).

mod parser;
mod compositor;

use std::sync::Mutex;

use crate::{log_info, log_warn, AppRegistry, FocusedApp};
use crate::chrome::{TITLEBAR_H, Z_DOCK};
use crate::APP_DIRTY;
use crate::flush_counts;
use supervisor::logging::Subsystem;

#[cfg(target_os = "linux")]
use crate::display;

// Re-export public API used by other modules.
#[cfg(target_os = "linux")]
pub use compositor::{force_repaint, run_compositor_tick};

#[cfg(target_os = "linux")]
pub use parser::parse_color;

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
    if cmd != "flush" && cmd != "present" {
        APP_DIRTY.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
            .lock().unwrap().insert(sender.to_string(), true);
    }

    if cmd == "flush" || cmd == "present" {
        // P31: mark frame as ready for the compositor tick.
        if let Some(st) = app_registry.lock().unwrap().get(sender) {
            st.lock().unwrap().frame_ready = true;
        }
        let mut fb = fb_lock.lock().unwrap();
        let (fb_w, fb_h) = (fb.width, fb.height);

        // -- Compositor pass --
        compositor::render_wallpaper(&mut *fb, fb_w, fb_h);
        compositor::blit_all_surfaces(&mut *fb, app_registry);
        crate::chrome::draw_chrome_onto(&mut *fb, app_registry, focused);

        // Draw overlay menus (dropdown, context menu, banners) on top of chrome
        crate::chrome::render_dropdown_if_open(&mut *fb);
        crate::chrome::render_context_menu_if_open(&mut *fb);
        crate::toast::render_banners(&mut *fb);

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
        parser::parse_fill_rect(args, sender, win, chrome_h, &fb_lock, app_registry);
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text:") {
        parser::parse_draw_text(args, sender, win, chrome_h, &fb_lock, app_registry);
        return;
    }

    if let Some(args) = cmd.strip_prefix("rect_border:") {
        parser::parse_rect_border(args, sender, win, chrome_h, &fb_lock, app_registry);
        return;
    }

    if let Some(args) = cmd.strip_prefix("clear_region:") {
        parser::parse_clear_region(args, sender, win, chrome_h, &fb_lock, app_registry);
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_text_wrap:") {
        parser::parse_draw_text_wrap(args, sender, win, chrome_h, &fb_lock, app_registry);
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_glyph:") {
        parser::parse_draw_glyph(args, sender, win, chrome_h, &fb_lock, app_registry);
        return;
    }

    if let Some(args) = cmd.strip_prefix("draw_image:") {
        parser::parse_draw_image(args, sender, win, chrome_h, &fb_lock, app_registry);
        return;
    }

    if let Some(args) = cmd.strip_prefix("fill_rect_r:") {
        parser::parse_fill_rect_r(args, sender, win, chrome_h, &fb_lock, app_registry);
        return;
    }

    // set_layer_alpha: animation alpha applied automatically by blit_all_surfaces.
    if cmd.starts_with("set_layer_alpha:") { return; }

    log_warn!(Subsystem::Display, Some(sender), "unknown command: {cmd}");
}

#[cfg(not(target_os = "linux"))] fn _dummy_non_linux() { let _ = (TITLEBAR_H, Z_DOCK); }
