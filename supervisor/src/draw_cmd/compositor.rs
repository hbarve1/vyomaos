// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Compositor functions: surface blitting, wallpaper rendering, repaint loop.

use crate::{AppRegistry, FocusedApp};
use crate::chrome::{draw_chrome_onto, TITLEBAR_H, Z_DESKTOP, Z_DOCK};
use crate::lock_or_recover;

#[cfg(target_os = "linux")]
use crate::display;

/// Render the desktop wallpaper using the cached buffer.
/// When `fb.wallpaper_dirty` is true, re-render into `fb.wallpaper_cache`
/// then mark clean. Always memcpy the cache into `fb.back` (fast ~3ms).
#[cfg(target_os = "linux")]
pub fn render_wallpaper(fb: &mut display::Framebuffer, fb_w: u32, fb_h: u32) {
    if fb.wallpaper_dirty {
        // Render wallpaper into cache buffer (expensive, done once or on change).
        match crate::wallpaper::current() {
            crate::wallpaper::Wallpaper::SolidColor(rgba) => {
                let a = (rgba & 0xFF) as u8;
                let r = ((rgba >> 24) & 0xFF) as u8;
                let g = ((rgba >> 16) & 0xFF) as u8;
                let b = ((rgba >>  8) & 0xFF) as u8;
                let pixel = if a == 255 { [b, g, r, 0xFF] } else { [b, g, r, a] };
                for off in (0..fb.wallpaper_cache.len()).step_by(4) {
                    fb.wallpaper_cache[off..off + 4].copy_from_slice(&pixel);
                }
            }
            crate::wallpaper::Wallpaper::Gradient(ref stops) => {
                let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
                display::draw_multi_gradient(&mut fb.wallpaper_cache, 0, 0, fw, fh, stops, fs, fw, fh);
            }
            crate::wallpaper::Wallpaper::Image(ref path) => {
                // Fill with dark fallback first.
                let pixel: [u8; 4] = [0x1E, 0x1C, 0x1C, 0xFF]; // BGRA for 0x1C1C1EFF
                for off in (0..fb.wallpaper_cache.len()).step_by(4) {
                    fb.wallpaper_cache[off..off + 4].copy_from_slice(&pixel);
                }
                let mut ic = lock_or_recover(&crate::image_cache());
                if let Some(img) = ic.get(path) {
                    let (iw, ih, rgba_data) = (img.width, img.height, img.rgba.clone());
                    drop(ic);
                    let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
                    display::blit_image(&mut fb.wallpaper_cache, &rgba_data, iw, ih, 0, 0, fw, fh, fs, fw, fh);
                }
            }
        }
        fb.wallpaper_dirty = false;
    }
    // Fast memcpy of pre-rendered wallpaper into back-buffer.
    let len = fb.wallpaper_cache.len().min(fb.back.len());
    fb.back[..len].copy_from_slice(&fb.wallpaper_cache[..len]);
    let _ = (fb_w, fb_h); // suppress unused warnings
}

/// Blit all app surfaces in Z-order onto the framebuffer back-buffer.
/// Only surfaces for apps visible on the current workspace are composited.
#[cfg(target_os = "linux")]
pub fn blit_all_surfaces(fb: &mut display::Framebuffer, registry: &AppRegistry) {
    // Collect app info and workspace visibility in a single lock to avoid
    // deadlock (is_visible also locks registry).
    let current_ws = lock_or_recover(&crate::workspace::manager()).current;
    let mut apps_sorted: Vec<(u32, String, (u32, u32, u32, u32))> = {
        let reg = lock_or_recover(&registry);
        let ws_mgr = lock_or_recover(&crate::workspace::manager());
        reg.iter()
            .filter_map(|(name, st)| {
                let st = lock_or_recover(&st);
                st.win_region.map(|r| (st.win_z, name.clone(), r))
            })
            .filter(|(z, name, _)| {
                // System apps (dock, overlay, desktop) visible on all workspaces
                if *z >= Z_DOCK || *z == Z_DESKTOP { return true; }
                let app_ws = ws_mgr.app_workspace.get(name.as_str()).copied().unwrap_or(0);
                app_ws == current_ws
            })
            .collect()
    };
    apps_sorted.sort_by_key(|(z, _, _)| *z);
    let (fw, fh, fs) = (fb.width, fb.height, fb.stride);
    for (z, name, (wx, wy, ww, _wh)) in &apps_sorted {
        let (surface_arc, alpha) = {
            let reg = lock_or_recover(&registry);
            let st_opt = reg.get(name.as_str());
            let surface = st_opt.and_then(|st| lock_or_recover(&st).surface.clone());
            let a = st_opt.map(|st| crate::chrome::sample_and_clear_anim(&mut lock_or_recover(&st).pending_anim)).unwrap_or(255);
            (surface, a)
        };
        if let Some(arc) = surface_arc {
            let surface = lock_or_recover(&arc);
            let blit_y = if *z >= Z_DOCK || *z == Z_DESKTOP { *wy } else { wy + TITLEBAR_H };
            if *ww > 0 {
                display::blit_surface(&mut fb.back, &surface, *wx, blit_y, alpha, fs, fw, fh);
            }
        }
    }
}

/// Full compositor pass for supervisor-side repaints (drag, snap) that bypass
/// the normal app `flush` path.
/// Pipeline: wallpaper -> surfaces -> chrome -> overlays -> cursor -> flush
#[cfg(target_os = "linux")]
pub fn force_repaint(registry: &AppRegistry, focused: &FocusedApp) {
    let Some(fb_lock) = display::get() else { return };
    let mut fb = lock_or_recover(&fb_lock);
    let (fb_w, fb_h) = (fb.width, fb.height);
    render_wallpaper(&mut *fb, fb_w, fb_h);
    blit_all_surfaces(&mut *fb, registry);
    draw_chrome_onto(&mut *fb, registry, focused);
    crate::chrome::render_dropdown_if_open(&mut *fb);
    crate::chrome::render_context_menu_if_open(&mut *fb);
    crate::toast::render_banners(&mut *fb);
    // Cursor is the absolute LAST step before flush (no save/restore needed).
    fb.draw_cursor();
    fb.flush_no_cursor();
}

/// P31: deadline-aware compositor tick loop -- polls `frame_ready` flags and
/// recomposites. Achieves true 60fps by sleeping only the remaining budget
/// after work completes (16ms - elapsed), instead of a fixed 16ms sleep.
#[cfg(target_os = "linux")]
pub fn run_compositor_tick(registry: &AppRegistry, focused: &FocusedApp) {
    loop {
        let tick_start = std::time::Instant::now();
        let any_dirty = {
            let reg = lock_or_recover(&registry);
            reg.values().any(|st| {
                let s = lock_or_recover(&st);
                s.frame_ready || s.pending_anim.is_some()
            })
        };
        if any_dirty {
            {
                let reg = lock_or_recover(&registry);
                for st in reg.values() { lock_or_recover(&st).frame_ready = false; }
            }
            force_repaint(registry, focused);
        }
        // Sleep for the remaining frame budget (true 60fps).
        let elapsed = tick_start.elapsed();
        let remaining = std::time::Duration::from_millis(16).saturating_sub(elapsed);
        if !remaining.is_zero() { std::thread::sleep(remaining); }
    }
}
