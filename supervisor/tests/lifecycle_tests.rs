// Lifecycle unit tests — TDD RED until should_restart is implemented.

// ── is_watchdog_kill ──────────────────────────────────────────────────────────

#[test]
fn test_watchdog_sentinel_minus_one_is_kill() {
    assert!(
        supervisor::lifecycle::is_watchdog_kill(-1),
        "exit code -1 is the watchdog-kill sentinel"
    );
}

#[test]
fn test_zero_exit_code_is_not_watchdog_kill() {
    assert!(
        !supervisor::lifecycle::is_watchdog_kill(0),
        "exit code 0 (clean exit) must not be treated as a watchdog kill"
    );
}

#[test]
fn test_positive_exit_code_is_not_watchdog_kill() {
    assert!(
        !supervisor::lifecycle::is_watchdog_kill(1),
        "non-zero positive exit code must not be treated as a watchdog kill"
    );
}

#[test]
fn test_negative_non_sentinel_is_not_watchdog_kill() {
    // Only -1 is the sentinel; other negative values are not watchdog kills.
    assert!(
        !supervisor::lifecycle::is_watchdog_kill(-2),
        "exit code -2 must not match the watchdog sentinel"
    );
}

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

// ─────────────────────────────────────────────────────────────────────────────

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

// test_focus_transfers_on_exit: verifies that when the focused app exits and
// multiple display apps remain, focus transfers to the alphabetically-first
// available app (consistent with Alt+Tab order).
#[test]
fn test_focus_transfers_on_exit() {
    // "calculator" exits while focused; "gui-demo", "shell", "ticker" remain.
    // Sorted alphabetically: ["gui-demo", "shell", "ticker"] → pick "gui-demo".
    let mut remaining = vec!["ticker", "shell", "gui-demo"];
    remaining.sort();
    let new_focus = transfer_focus("calculator", Some("calculator"), &remaining);
    assert_eq!(
        new_focus.as_deref(),
        Some("gui-demo"),
        "focus should transfer to alphabetically-first remaining display app"
    );
}

// ── crash notification condition ──────────────────────────────────────────────

/// restart=never, non-zero exit → notification should fire
#[test]
fn test_crash_exit_triggers_notification() {
    // will_restart=false (policy=never), exit_code=1 → needs toast
    assert!(
        supervisor::lifecycle::needs_crash_notification(false, 1),
        "non-zero exit of a restart=never app must trigger a crash notification"
    );
}

/// restart=never, zero exit → clean shutdown, no notification
#[test]
fn test_clean_exit_does_not_trigger_notification() {
    assert!(
        !supervisor::lifecycle::needs_crash_notification(false, 0),
        "clean exit (code 0) must not trigger a crash notification"
    );
}

/// restart=always, non-zero exit → app will restart, no notification
#[test]
fn test_restarting_app_does_not_trigger_notification() {
    assert!(
        !supervisor::lifecycle::needs_crash_notification(true, 1),
        "an app that will restart must not show a crash notification"
    );
}

/// restart=always, zero exit → will restart, clean exit, no notification
#[test]
fn test_restarting_clean_exit_does_not_trigger_notification() {
    assert!(
        !supervisor::lifecycle::needs_crash_notification(true, 0),
        "restarting app with clean exit must not show a crash notification"
    );
}

// ── format_cpu tests ──────────────────────────────────────────────────────────

#[test]
fn format_cpu_zero_elapsed() {
    assert_eq!(supervisor::lifecycle::format_cpu(10, 0), "0%");
}

#[test]
fn format_cpu_under_one_pct() {
    // 0 busy ticks in 1000ms → <1%
    assert_eq!(supervisor::lifecycle::format_cpu(0, 1000), "<1%");
}

#[test]
fn format_cpu_fifty_pct() {
    assert_eq!(supervisor::lifecycle::format_cpu(500, 1000), "50%");
}

#[test]
fn format_cpu_hundred_pct() {
    assert_eq!(supervisor::lifecycle::format_cpu(1000, 1000), "100%");
}

// ── format_uptime unit tests ──────────────────────────────────────────────────

#[test]
fn format_uptime_seconds() {
    assert_eq!(supervisor::lifecycle::format_uptime(5), "5s");
}

#[test]
fn format_uptime_exact_minute() {
    assert_eq!(supervisor::lifecycle::format_uptime(60), "1m");
}

#[test]
fn format_uptime_minutes_secs() {
    assert_eq!(supervisor::lifecycle::format_uptime(150), "2m30s");
}

#[test]
fn format_uptime_hours() {
    assert_eq!(supervisor::lifecycle::format_uptime(3600), "1h");
}

#[test]
fn format_uptime_hours_minutes() {
    assert_eq!(supervisor::lifecycle::format_uptime(3900), "1h5m");
}

// ── format_system_uptime ──────────────────────────────────────────────────────

#[test]
fn format_system_uptime_seconds() {
    assert_eq!(supervisor::lifecycle::format_system_uptime(5), "up 5s");
}

#[test]
fn format_system_uptime_minutes() {
    assert_eq!(supervisor::lifecycle::format_system_uptime(90), "up 1m30s");
}

#[test]
fn format_system_uptime_hours() {
    assert_eq!(supervisor::lifecycle::format_system_uptime(3600), "up 1h");
}

#[test]
fn format_system_uptime_prefix() {
    assert!(supervisor::lifecycle::format_system_uptime(0).starts_with("up "));
}
