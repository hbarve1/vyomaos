// Lifecycle module — pure functions for app restart policy decisions.

/// Returns true if the given restart policy string means the app should restart.
pub fn should_restart(policy: &str) -> bool {
    policy == "always"
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
