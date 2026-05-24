// Lifecycle module — pure functions for app restart policy decisions.

/// Returns true if the given restart policy string means the app should restart.
pub fn should_restart(policy: &str) -> bool {
    policy == "always"
}

/// Returns true when a crash toast notification should be shown:
/// the app did not restart (`should_restart = false`) and the exit code is
/// non-zero (unexpected crash rather than a clean shutdown).
pub fn needs_crash_notification(will_restart: bool, exit_code: i32) -> bool {
    !will_restart && exit_code != 0
}

/// Returns true if the exit code is the watchdog-kill sentinel (-1).
///
/// The supervisor uses -1 as a distinguished sentinel to indicate an app was
/// killed by the watchdog (silent for too long), as opposed to a real exit code.
pub fn is_watchdog_kill(exit_code: i32) -> bool {
    exit_code == -1
}

/// Format a single `ps` output line for an app.
///
/// When `restarts` is 0 the `restarts=N` field is omitted to keep the output
/// clean for apps that have never been restarted.
pub fn format_ps_line(name: &str, pid: u32, state: &str, restarts: u32) -> String {
    if restarts == 0 {
        format!("{:<20} pid={:<6} state={}", name, pid, state)
    } else {
        format!("{:<20} pid={:<6} state={} restarts={}", name, pid, state, restarts)
    }
}

/// Returns CPU usage as a percentage string, e.g. "12%" or "<1%".
/// ticks: number of busy ticks observed; elapsed_ms: total elapsed milliseconds.
/// Assumes each tick represents 1ms of work (supervisor loop tick).
pub fn format_cpu(ticks: u64, elapsed_ms: u64) -> String {
    if elapsed_ms == 0 {
        return "0%".to_string();
    }
    let pct = (ticks * 100) / elapsed_ms;
    if pct == 0 {
        "<1%".to_string()
    } else {
        format!("{pct}%")
    }
}
