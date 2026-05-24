// Lifecycle module — pure functions for app restart policy decisions.

/// Returns true if the given restart policy string means the app should restart.
pub fn should_restart(policy: &str) -> bool {
    policy == "always"
}

/// Format a duration in seconds as a compact human-readable string.
/// Examples: 5 → "5s", 90 → "1m30s", 3600 → "1h", 3661 → "1h1m1s".
fn format_uptime(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    match (h, m, s) {
        (0, 0, s) => format!("{s}s"),
        (0, m, 0) => format!("{m}m"),
        (0, m, s) => format!("{m}m{s}s"),
        (h, 0, 0) => format!("{h}h"),
        (h, m, 0) => format!("{h}h{m}m"),
        (h, 0, s) => format!("{h}h{s}s"),
        (h, m, s) => format!("{h}h{m}m{s}s"),
    }
}

/// Return a human-readable system uptime string prefixed with "up ".
/// This is a pure function suitable for use in IPC reply formatting.
pub fn format_system_uptime(secs: u64) -> String {
    format!("up {}", format_uptime(secs))
}
