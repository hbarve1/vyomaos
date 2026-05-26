// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Crash / watchdog notification toasts and focus transfer on app exit.

use std::thread;

use crate::{log_info, AppRegistry, AppStatus, FocusedApp};
use supervisor::logging::Subsystem;

#[cfg(target_os = "linux")]
use crate::{display, font};

/// Show a crash-notification toast for 3 seconds in the top-right corner.
pub fn show_crash_toast(name: &str, code: i32) {
    let title = format!("{name} crashed");
    let msg = if supervisor::lifecycle::is_watchdog_kill(code) {
        "killed by watchdog (no output)".to_string()
    } else {
        format!("exited with code {code}")
    };
    #[cfg(target_os = "linux")]
    {
        const NX: u32 = 1020;
        const NY: u32 = 10;
        const NW: u32 = 400;
        const NH: u32 = 60;
        if let Some(fb_lock) = display::get() {
            let mut fb = fb_lock.lock().unwrap();
            fb.fill_rect(NX, NY, NW, NH, 0x21262DFF);
            fb.rect_border(NX, NY, NW, NH, 0x58A6FFFF);
            fb.draw_text(NX + 8, NY + 8, &title, 0xFFFFFFFF, font::FontSize::Medium);
            fb.draw_text(NX + 8, NY + 28, &msg, 0x8B949EFF, font::FontSize::Medium);
            fb.flush();
        }
        thread::spawn(move || {
            thread::sleep(std::time::Duration::from_secs(3));
            if let Some(fb_lock) = display::get() {
                let mut fb = fb_lock.lock().unwrap();
                fb.fill_rect(NX, NY, NW, NH, 0x0D1117FF);
                fb.flush();
            }
        });
    }
    log_info!(Subsystem::Display, None, "crash toast: {title:?} — {msg:?}");
}

/// Transfer keyboard focus away from `exiting` to the next available display app.
///
/// No-op if `exiting` does not currently hold focus.
pub fn auto_transfer_focus(exiting: &str, app_registry: &AppRegistry, focused: &FocusedApp) {
    let is_focused = focused.lock().unwrap().as_deref() == Some(exiting);
    if !is_focused { return; }

    // Find first other Running display-or-shell app (sorted for determinism)
    let next = {
        let reg = app_registry.lock().unwrap();
        let mut names: Vec<String> = reg.iter()
            .filter(|(name, state_arc)| {
                if name.as_str() == exiting { return false; }
                let st = state_arc.lock().unwrap();
                matches!(st.status, AppStatus::Running) && st.has_display
            })
            .map(|(name, _)| name.clone())
            .collect();
        names.sort();
        names.into_iter().next()
    };

    *focused.lock().unwrap() = next;
}

// Satisfy the borrow checker on non-Linux builds where `display` is not imported.
#[cfg(not(target_os = "linux"))]
fn _unused_mutex<T>(_: &Mutex<T>) {}

// ── T065–T066: Notification banner queue ──────────────────────────────────────

use std::collections::VecDeque;
use std::sync::OnceLock;

/// A notification banner displayed in the top-right corner.
pub struct NotificationBanner {
    pub app_name: String,
    pub icon_path: Option<String>,
    pub title: String,
    pub body: String,
    pub created_ms: u64,
    pub dismiss_after_ms: u32,
}

static BANNER_QUEUE: OnceLock<std::sync::Mutex<VecDeque<NotificationBanner>>> = OnceLock::new();

fn banner_queue() -> &'static std::sync::Mutex<VecDeque<NotificationBanner>> {
    BANNER_QUEUE.get_or_init(|| std::sync::Mutex::new(VecDeque::new()))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Enqueue a notification banner for display.
pub fn enqueue_banner(app_name: &str, title: &str, body: &str) {
    let mut q = banner_queue().lock().unwrap();
    if q.len() >= 3 { q.pop_front(); }
    q.push_back(NotificationBanner {
        app_name: app_name.to_string(),
        icon_path: None,
        title: title.to_string(),
        body: body[..body.len().min(120)].to_string(),
        created_ms: now_ms(),
        dismiss_after_ms: 4000,
    });
}

/// Draw and auto-dismiss all pending notification banners.
/// Call this from the display flush path.
#[cfg(target_os = "linux")]
pub fn render_banners(fb: &mut crate::display::Framebuffer) {
    use crate::display::draw_rounded_rect;
    let now = now_ms();
    let mut q = banner_queue().lock().unwrap();
    q.retain(|b| now.saturating_sub(b.created_ms) < b.dismiss_after_ms as u64);
    let (sw, sh, fs) = (fb.width, fb.height, fb.stride);
    let (bw, bh) = (320u32, 64u32);
    let bx = sw.saturating_sub(bw + 12);
    for (i, banner) in q.iter().enumerate() {
        let by = 32 + i as u32 * (bh + 8);
        if by + bh > sh { break; }
        draw_rounded_rect(&mut fb.back, bx, by, bw, bh, 0x1C1C1EE5_u32, 10, fs, sw, sh);
        if let Some(ref icon_path) = banner.icon_path {
            let mut ic = crate::image_cache().lock().unwrap();
            if let Some(img) = ic.get(icon_path) {
                let (iw, ih, rgba) = (img.width, img.height, img.rgba.clone());
                drop(ic);
                crate::display::blit_image(&mut fb.back, &rgba, iw, ih, bx + 8, by + 8, 16, 16, fs, sw, sh);
            }
        }
        let tx = (bx + 30) as i32;
        crate::chrome::draw_glyph_str_pub(fb, &banner.title, tx, (by + 20) as i32, 0xFFFFFFFF, 13, true, false);
        crate::chrome::draw_glyph_str_pub(fb, &banner.body,  tx, (by + 40) as i32, 0xAAAAAFFF, 11, false, false);
    }
}
