// Lifecycle module — pure functions for app restart policy decisions.

/// Returns true if the given restart policy string means the app should restart.
pub fn should_restart(policy: &str) -> bool {
    policy == "always"
}

/// Format a duration given in whole seconds as a compact human-readable string.
///
/// Examples:
/// - `5`    → `"5s"`
/// - `60`   → `"1m"`
/// - `150`  → `"2m30s"`
/// - `3600` → `"1h"`
/// - `3900` → `"1h5m"`
pub fn format_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        let m = secs / 60;
        let s = secs % 60;
        if s == 0 { format!("{m}m") } else { format!("{m}m{s}s") }
    } else {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        if m == 0 { format!("{h}h") } else { format!("{h}h{m}m") }
    }
}
