// Supervisor library root — exposes pure modules for integration tests and tools.
pub mod manifest;
pub mod logging;
pub mod ipc;
pub mod lifecycle;
pub mod windows;
pub mod statusbar;

#[cfg(target_os = "linux")]
pub mod font;
#[cfg(target_os = "linux")]
pub mod display;

/// Draw command handling — contains testable utility functions.
#[cfg(target_os = "linux")]
pub mod draw_cmd {
    use std::collections::HashMap;

    /// Returns `true` and updates `cache` if `uptime_secs` changed since the last
    /// call for this `name`.  Pure function (no statics) — easy to unit-test.
    pub fn should_redraw_statusbar(name: &str, uptime_secs: u64, cache: &mut HashMap<String, u64>) -> bool {
        let prev = cache.get(name).copied();
        if prev == Some(uptime_secs) {
            return false;
        }
        cache.insert(name.to_string(), uptime_secs);
        true
    }
}
