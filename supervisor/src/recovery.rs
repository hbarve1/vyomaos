// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P106: Recovery mode — detects boot failures or kernel cmdline flag,
//! enters minimal shell-only mode, provides reset/repair IPC commands.

use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use crate::{log_info, log_warn, log_error, send_reply, Inbox};
use supervisor::logging::Subsystem;

/// Persistent boot counter file on the data partition.
const BOOT_COUNT_PATH: &str = "/data/boot-count";

/// Number of consecutive failed boots before auto-entering recovery.
const FAILURE_THRESHOLD: u32 = 3;

/// Kernel cmdline path.
const CMDLINE_PATH: &str = "/proc/cmdline";

/// Files deleted by `reset_to_defaults`.
const RESETTABLE_FILES: &[&str] = &[
    "/data/settings.toml",
    "/data/session.toml",
    "/data/firewall.toml",
    "/data/theme.toml",
];

/// Required directories under `/data` that `repair_filesystem` ensures exist.
const REQUIRED_DIRS: &[&str] = &[
    "/data/apps",
    "/data/logs",
    "/data/screenshots",
    "/data/trash",
    "/data/packages",
];

/// Global recovery mode flag — set once at startup.
static RECOVERY_MODE: OnceLock<bool> = OnceLock::new();

// ── Public API ──────────────────────────────────────────────────────────────

/// Returns `true` if the supervisor is running in recovery mode.
pub fn is_recovery_mode() -> bool {
    *RECOVERY_MODE.get().unwrap_or(&false)
}

/// Check all recovery triggers and latch the global flag.
/// Called once at startup, before spawning apps.
pub fn check_recovery_mode() {
    let cmdline_flag = check_cmdline();
    let boot_failures = read_boot_count() >= FAILURE_THRESHOLD;

    let recovery = cmdline_flag || boot_failures;
    let _ = RECOVERY_MODE.set(recovery);

    if recovery {
        let reason = if cmdline_flag {
            "kernel cmdline vyoma.recovery=1"
        } else {
            "3+ consecutive boot failures"
        };
        log_warn!(Subsystem::Lifecycle, None, "RECOVERY MODE activated: {reason}");
    }
}

/// Increment the persistent boot counter. Called at supervisor startup.
pub fn increment_boot_count() {
    let count = read_boot_count().saturating_add(1);
    write_boot_count(count);
    log_info!(Subsystem::Lifecycle, None, "boot count: {count}");
}

/// Clear the boot counter (reset to 0). Called after all apps have
/// spawned successfully, indicating a healthy boot.
pub fn clear_boot_count() {
    write_boot_count(0);
    log_info!(Subsystem::Lifecycle, None, "boot count cleared (healthy boot)");
}

/// In recovery mode, filter `all_entries` to keep only the shell app.
/// Returns `true` if the list was filtered.
pub fn enter_recovery_mode(
    all_entries: &mut Vec<supervisor::manifest::BootEntry>,
) -> bool {
    if !is_recovery_mode() {
        return false;
    }
    // Keep only shell manifests (apps whose manifest path contains "shell")
    all_entries.retain(|e| {
        e.manifest.contains("/shell/") || e.manifest.ends_with("/shell/vyoma.toml")
    });
    log_info!(Subsystem::Lifecycle, None,
        "recovery: loading {} app(s) (shell only)", all_entries.len());
    true
}

/// Delete user configuration files, restoring factory defaults.
pub fn reset_to_defaults() -> (usize, Vec<String>) {
    let mut deleted = 0usize;
    let mut errors = Vec::new();
    for path in RESETTABLE_FILES {
        if Path::new(path).exists() {
            match fs::remove_file(path) {
                Ok(()) => {
                    log_info!(Subsystem::Lifecycle, None, "recovery: deleted {path}");
                    deleted += 1;
                }
                Err(e) => {
                    let msg = format!("recovery: failed to delete {path}: {e}");
                    log_error!(Subsystem::Lifecycle, None, "{msg}");
                    errors.push(msg);
                }
            }
        }
    }
    (deleted, errors)
}

/// Check `/data` directory structure; recreate any missing required dirs.
pub fn repair_filesystem() -> (usize, Vec<String>) {
    let mut created = 0usize;
    let mut errors = Vec::new();
    for dir in REQUIRED_DIRS {
        if !Path::new(dir).exists() {
            match fs::create_dir_all(dir) {
                Ok(()) => {
                    log_info!(Subsystem::Lifecycle, None, "recovery: created {dir}");
                    created += 1;
                }
                Err(e) => {
                    let msg = format!("recovery: failed to create {dir}: {e}");
                    log_error!(Subsystem::Lifecycle, None, "{msg}");
                    errors.push(msg);
                }
            }
        }
    }
    (created, errors)
}

// ── IPC command handler ─────────────────────────────────────────────────────

/// Handle `@supervisor: recovery-*` IPC commands.
/// Returns `true` if the verb was recognized and handled.
pub fn handle_recovery_command(
    verb: &str,
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "recovery-status" => {
            let mode = if is_recovery_mode() { "active" } else { "inactive" };
            let count = read_boot_count();
            send_reply(
                sender,
                &format!("REPLY:recovery {mode} boot-count={count}"),
                inbox,
            );
        }
        "recovery-reset" => {
            let (deleted, errors) = reset_to_defaults();
            if errors.is_empty() {
                send_reply(
                    sender,
                    &format!("REPLY:recovery reset ok, {deleted} file(s) removed"),
                    inbox,
                );
            } else {
                send_reply(
                    sender,
                    &format!(
                        "REPLY:recovery reset partial, {deleted} removed, {} error(s)",
                        errors.len()
                    ),
                    inbox,
                );
            }
        }
        "recovery-repair" => {
            let (created, errors) = repair_filesystem();
            if errors.is_empty() {
                send_reply(
                    sender,
                    &format!("REPLY:recovery repair ok, {created} dir(s) created"),
                    inbox,
                );
            } else {
                send_reply(
                    sender,
                    &format!(
                        "REPLY:recovery repair partial, {created} created, {} error(s)",
                        errors.len()
                    ),
                    inbox,
                );
            }
        }
        "recovery-exit" => {
            clear_boot_count();
            send_reply(sender, "REPLY:recovery exit — reboot to resume normal mode", inbox);
            log_info!(Subsystem::Lifecycle, None, "recovery-exit: boot count cleared by {sender}");
        }
        _ => return false,
    }
    true
}

// ── Internal helpers ────────────────────────────────────────────────────────

/// Check `/proc/cmdline` for `vyoma.recovery=1`.
fn check_cmdline() -> bool {
    check_cmdline_content(
        &fs::read_to_string(CMDLINE_PATH).unwrap_or_default(),
    )
}

/// Testable cmdline parser (no filesystem dependency).
fn check_cmdline_content(content: &str) -> bool {
    content
        .split_whitespace()
        .any(|token| token == "vyoma.recovery=1")
}

/// Read the boot count from the persistent file. Returns 0 on any error.
fn read_boot_count() -> u32 {
    fs::read_to_string(BOOT_COUNT_PATH)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Write the boot count to the persistent file.
fn write_boot_count(count: u32) {
    if let Err(e) = fs::write(BOOT_COUNT_PATH, count.to_string()) {
        log_warn!(Subsystem::Lifecycle, None,
            "recovery: cannot write boot count to {BOOT_COUNT_PATH}: {e}");
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmdline_detects_recovery_flag() {
        assert!(check_cmdline_content("console=ttyS0 vyoma.recovery=1 quiet"));
        assert!(check_cmdline_content("vyoma.recovery=1"));
    }

    #[test]
    fn cmdline_ignores_absent_flag() {
        assert!(!check_cmdline_content("console=ttyS0 quiet"));
        assert!(!check_cmdline_content(""));
        assert!(!check_cmdline_content("vyoma.recovery=0"));
        assert!(!check_cmdline_content("other.recovery=1"));
    }

    #[test]
    fn cmdline_ignores_partial_match() {
        // Should not match substrings
        assert!(!check_cmdline_content("xvyoma.recovery=1"));
        assert!(!check_cmdline_content("vyoma.recovery=10"));
    }

    #[test]
    fn boot_count_threshold() {
        assert!(3 >= FAILURE_THRESHOLD);
        assert!(2 < FAILURE_THRESHOLD);
    }

    #[test]
    fn reset_to_defaults_on_missing_files() {
        // When no files exist, should return 0 deleted and no errors
        let (deleted, errors) = reset_to_defaults();
        assert_eq!(deleted, 0);
        assert!(errors.is_empty());
    }

    #[test]
    fn repair_filesystem_creates_dirs() {
        // In test env, /data doesn't exist so create_dir_all will fail,
        // but the function should not panic — it collects errors.
        let (_, _) = repair_filesystem();
        // Just verify it doesn't panic
    }

    #[test]
    fn resettable_files_are_valid_paths() {
        for path in RESETTABLE_FILES {
            assert!(path.starts_with("/data/"), "resettable path must be under /data: {path}");
        }
    }

    #[test]
    fn required_dirs_are_valid_paths() {
        for dir in REQUIRED_DIRS {
            assert!(dir.starts_with("/data/"), "required dir must be under /data: {dir}");
        }
    }

    #[test]
    fn is_recovery_mode_default_false() {
        // OnceLock not set in test context, should default to false
        // (uses unwrap_or(&false))
        assert!(!*RECOVERY_MODE.get().unwrap_or(&false));
    }
}
