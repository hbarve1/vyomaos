// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Crash / watchdog notification toasts and focus transfer on app exit.

use std::sync::Mutex;
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
