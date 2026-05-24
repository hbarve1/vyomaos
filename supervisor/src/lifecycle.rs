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
