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
