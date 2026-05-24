//! Per-window status bar helpers.
//!
//! This module contains pure, platform-independent helpers so that
//! unit tests can run on the host (macOS / Linux) without a framebuffer.

/// Format the status-bar label string shown at the bottom of every window.
///
/// # Examples
/// ```
/// use supervisor::statusbar::format_status_text;
/// assert_eq!(format_status_text("shell", 42), "[shell]  up 42s");
/// ```
pub fn format_status_text(name: &str, uptime_secs: u64) -> String {
    format!("[{}]  up {}s", name, uptime_secs)
}
