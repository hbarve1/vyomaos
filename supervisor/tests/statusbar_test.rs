// Tests for the per-window status bar helper.

use supervisor::statusbar::format_status_text;

#[test]
fn format_status_text_basic() {
    assert_eq!(format_status_text("shell", 42), "[shell]  up 42s");
}

#[test]
fn format_status_text_zero_uptime() {
    assert_eq!(format_status_text("gui-demo", 0), "[gui-demo]  up 0s");
}

#[test]
fn format_status_text_large_uptime() {
    assert_eq!(format_status_text("http-server", 3661), "[http-server]  up 3661s");
}
