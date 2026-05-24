// Lifecycle module — pure functions for app restart policy decisions.

/// Returns true if the given restart policy string means the app should restart.
pub fn should_restart(policy: &str) -> bool {
    policy == "always"
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
