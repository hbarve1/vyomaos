// Lifecycle unit tests — TDD RED until should_restart is implemented.

// ── format_ps_line tests ──────────────────────────────────────────────────────

#[test]
fn test_format_ps_line_zero_restarts_omits_field() {
    let line = supervisor::lifecycle::format_ps_line("my-app", 1234, "running", 0);
    assert!(
        !line.contains("restarts"),
        "restarts field should be omitted when count is 0, got: {line}"
    );
    assert!(line.contains("pid=1234"), "should include pid");
    assert!(line.contains("state=running"), "should include state");
}

#[test]
fn test_format_ps_line_nonzero_restarts_includes_field() {
    let line = supervisor::lifecycle::format_ps_line("my-app", 5678, "running", 3);
    assert!(
        line.contains("restarts=3"),
        "restarts=N should appear when count > 0, got: {line}"
    );
}

#[test]
fn test_format_ps_line_one_restart_shows_count() {
    let line = supervisor::lifecycle::format_ps_line("ticker", 42, "running", 1);
    assert!(
        line.contains("restarts=1"),
        "restarts=1 should appear after a single restart, got: {line}"
    );
}

#[test]
fn test_format_ps_line_stopped_state() {
    let line = supervisor::lifecycle::format_ps_line("worker", 0, "stopped(-1)", 7);
    assert!(line.contains("state=stopped(-1)"), "should include stopped state");
    assert!(line.contains("restarts=7"), "should include restart count");
}

#[test]
fn test_format_ps_line_name_padded() {
    let line = supervisor::lifecycle::format_ps_line("hi", 1, "running", 0);
    // Name field is padded to 20 chars; line should start with "hi" followed by spaces
    assert!(line.starts_with("hi"), "line should start with the app name");
}

// (1) restart_policy "never" → should not restart
#[test]
fn test_never_policy_does_not_restart() {
    assert!(!supervisor::lifecycle::should_restart("never"),
        "restart=never should return false");
}

// (2) restart_policy "always" → should restart
#[test]
fn test_always_policy_restarts() {
    assert!(supervisor::lifecycle::should_restart("always"),
        "restart=always should return true");
}

// ── auto_transfer_focus logic ─────────────────────────────────────────────────

// Pure-logic helper: given which app exited, the current focused name, and a
// sorted list of remaining running display apps, return the new focused name.
fn transfer_focus<'a>(
    exiting: &str,
    current_focused: Option<&'a str>,
    remaining_display: &[&'a str],
) -> Option<String> {
    if current_focused != Some(exiting) {
        return current_focused.map(str::to_string);
    }
    remaining_display.first().map(|s| s.to_string())
}

#[test]
fn test_focus_transfers_on_focused_app_exit() {
    let new_focus = transfer_focus("gui-demo", Some("gui-demo"), &["shell", "ticker"]);
    assert_eq!(new_focus.as_deref(), Some("shell"), "should pick first remaining display app");
}

#[test]
fn test_focus_unchanged_when_non_focused_app_exits() {
    let new_focus = transfer_focus("ticker", Some("gui-demo"), &["gui-demo"]);
    assert_eq!(new_focus.as_deref(), Some("gui-demo"), "focused app unchanged when non-focused exits");
}

#[test]
fn test_no_focus_when_last_app_exits() {
    let new_focus = transfer_focus("gui-demo", Some("gui-demo"), &[]);
    assert!(new_focus.is_none(), "focus becomes None when the last display app exits");
}

#[test]
fn test_focus_unchanged_when_no_focused_app() {
    let new_focus = transfer_focus("gui-demo", None, &["ticker"]);
    assert!(new_focus.is_none(), "no focus to transfer when focused is None");
}
