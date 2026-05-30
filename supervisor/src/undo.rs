// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Undo/redo system for reversible supervisor IPC commands.
//!
//! Maintains an undo stack and a redo stack.  When a reversible command is
//! executed, the caller pushes an `UndoEntry` via `push_undo`.  The
//! `@supervisor: undo` / `redo` / `undo-history` IPC commands operate on
//! the global `UNDO_STATE`.

use std::sync::{Mutex, OnceLock};

// ── UndoEntry ───────────────────────────────────────────────────────────────

/// A single reversible action on the undo/redo stack.
#[derive(Debug, Clone)]
pub struct UndoEntry {
    /// The IPC command that was originally executed (e.g. `"kill my-app"`).
    pub command: String,
    /// The IPC command that reverses `command` (e.g. `"run /apps/my-app/vyoma.toml"`).
    pub undo_command: String,
    /// Timestamp (ms since UNIX epoch) when the action was recorded.
    pub timestamp_ms: u64,
}

// ── UndoState ───────────────────────────────────────────────────────────────

/// Maximum number of entries kept on the undo stack.
const MAX_UNDO: usize = 50;

/// Combined undo + redo stacks.
pub struct UndoState {
    pub undo_stack: Vec<UndoEntry>,
    pub redo_stack: Vec<UndoEntry>,
}

impl UndoState {
    fn new() -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
        }
    }
}

/// Global undo state singleton.
static UNDO_STATE: OnceLock<Mutex<UndoState>> = OnceLock::new();

/// Return (or initialise) the global undo state.
pub fn state() -> &'static Mutex<UndoState> {
    UNDO_STATE.get_or_init(|| Mutex::new(UndoState::new()))
}

// ── Tracked previous-state for stateful commands ────────────────────────────

/// Last wallpaper RGBA value (used to build the undo command).
static LAST_WALLPAPER: OnceLock<Mutex<u32>> = OnceLock::new();

/// Get the last wallpaper colour, defaulting to the VyomaOS dark background.
pub fn last_wallpaper() -> u32 {
    *LAST_WALLPAPER.get_or_init(|| Mutex::new(0x1C1C1EFF)).lock().unwrap()
}

/// Update the tracked wallpaper value (call *after* recording undo).
pub fn set_last_wallpaper(rgba: u32) {
    *LAST_WALLPAPER.get_or_init(|| Mutex::new(0x1C1C1EFF)).lock().unwrap() = rgba;
}

// ── Public helpers ──────────────────────────────────────────────────────────

/// Current time in milliseconds since the UNIX epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Record a reversible action.  Clears the redo stack (new branch).
/// Enforces `MAX_UNDO` by removing the oldest entry when full.
pub fn push_undo(command: &str, undo_command: &str) {
    let mut s = state().lock().unwrap();
    s.redo_stack.clear();
    if s.undo_stack.len() >= MAX_UNDO {
        s.undo_stack.remove(0);
    }
    s.undo_stack.push(UndoEntry {
        command: command.to_string(),
        undo_command: undo_command.to_string(),
        timestamp_ms: now_ms(),
    });
}

/// Pop the most recent action from the undo stack.
/// Returns `Some(entry)` (already moved to redo stack) or `None`.
pub fn pop_undo() -> Option<UndoEntry> {
    let mut s = state().lock().unwrap();
    let entry = s.undo_stack.pop()?;
    s.redo_stack.push(entry.clone());
    Some(entry)
}

/// Pop the most recent action from the redo stack.
/// Returns `Some(entry)` (already moved back to undo stack) or `None`.
pub fn pop_redo() -> Option<UndoEntry> {
    let mut s = state().lock().unwrap();
    let entry = s.redo_stack.pop()?;
    s.undo_stack.push(entry.clone());
    Some(entry)
}

/// Return (up to) the last `n` entries on the undo stack, most-recent first.
pub fn recent_history(n: usize) -> Vec<UndoEntry> {
    let s = state().lock().unwrap();
    s.undo_stack.iter().rev().take(n).cloned().collect()
}

// ── Capture helpers for stateful commands ────────────────────────────────────

/// Build undo for `kill <app>` — needs the manifest path to re-run later.
pub fn capture_kill(app_name: &str, manifest: &str) {
    let cmd = format!("kill {app_name}");
    let undo = format!("run {manifest}");
    push_undo(&cmd, &undo);
}

/// Build undo for `run <manifest>` — reverse is `kill <app_name>`.
pub fn capture_run(app_name: &str, manifest: &str) {
    let cmd = format!("run {manifest}");
    let undo = format!("kill {app_name}");
    push_undo(&cmd, &undo);
}

/// Build undo for `volume <new>` — saves old value.
pub fn capture_volume(old_volume: u8, new_volume: u8) {
    let cmd = format!("volume {new_volume}");
    let undo = format!("volume {old_volume}");
    push_undo(&cmd, &undo);
}

/// Build undo for `mute`/`unmute` toggle.
pub fn capture_mute(was_muted: bool) {
    if was_muted {
        push_undo("unmute", "mute");
    } else {
        push_undo("mute", "unmute");
    }
}

/// Build undo for `theme <new>` — saves old theme name.
pub fn capture_theme(old_name: &str, new_name: &str) {
    let cmd = format!("theme {new_name}");
    let undo = format!("theme {old_name}");
    push_undo(&cmd, &undo);
}

/// Build undo for `workspace <new>` — saves old workspace index.
pub fn capture_workspace(old_idx: usize, new_idx: usize) {
    let cmd = format!("workspace {new_idx}");
    let undo = format!("workspace {old_idx}");
    push_undo(&cmd, &undo);
}

/// Build undo for `wallpaper <new_rgba>` — saves old rgba value.
pub fn capture_wallpaper(old_rgba: u32, new_rgba: u32) {
    let cmd = format!("wallpaper {new_rgba}");
    let undo = format!("wallpaper {old_rgba}");
    push_undo(&cmd, &undo);
}

// ── IPC command handler ─────────────────────────────────────────────────────

use crate::{send_reply, Inbox};

/// Handle `@supervisor: undo`, `redo`, and `undo-history` commands.
/// Returns `true` if handled.
pub fn handle_undo_command(
    verb:   &str,
    sender: &str,
    inbox:  &Inbox,
) -> bool {
    match verb {
        "undo" => {
            match pop_undo() {
                Some(entry) => {
                    send_reply(sender, &format!("REPLY:undo executing: {}", entry.undo_command), inbox);
                }
                None => {
                    send_reply(sender, "REPLY:undo stack empty", inbox);
                }
            }
            true
        }
        "redo" => {
            match pop_redo() {
                Some(entry) => {
                    send_reply(sender, &format!("REPLY:redo executing: {}", entry.command), inbox);
                }
                None => {
                    send_reply(sender, "REPLY:redo stack empty", inbox);
                }
            }
            true
        }
        "undo-history" => {
            let entries = recent_history(10);
            if entries.is_empty() {
                send_reply(sender, "REPLY:undo-history empty", inbox);
            } else {
                let lines: Vec<String> = entries.iter().map(|e| {
                    format!("[{}] {} (undo: {})", e.timestamp_ms, e.command, e.undo_command)
                }).collect();
                send_reply(sender, &format!("REPLY:undo-history|{}", lines.join("|")), inbox);
            }
            true
        }
        _ => false,
    }
}

/// Retrieve the command string to execute for an undo operation.
/// Note: `pop_undo` already moved the entry to redo, so peek at redo stack.
pub fn undo_command_to_execute() -> Option<String> {
    let s = state().lock().unwrap();
    s.redo_stack.last().map(|e| e.undo_command.clone())
}

/// Retrieve the command string to execute for a redo operation.
pub fn redo_command_to_execute() -> Option<String> {
    let s = state().lock().unwrap();
    s.undo_stack.last().map(|e| e.command.clone())
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    /// Serialize all undo tests since they share global state.
    static TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    fn test_lock() -> &'static Mutex<()> {
        TEST_LOCK.get_or_init(|| Mutex::new(()))
    }

    /// Reset the global state for test isolation.
    fn reset() -> std::sync::MutexGuard<'static, ()> {
        let guard = test_lock().lock().unwrap_or_else(|e| e.into_inner());
        let mut s = state().lock().unwrap();
        s.undo_stack.clear();
        s.redo_stack.clear();
        guard
    }

    #[test]
    fn push_and_pop_undo() {
        let _g = reset();
        push_undo("kill foo", "run /apps/foo/vyoma.toml");
        push_undo("volume 80", "volume 50");

        let entry = pop_undo().unwrap();
        assert_eq!(entry.command, "volume 80");
        assert_eq!(entry.undo_command, "volume 50");

        let entry = pop_undo().unwrap();
        assert_eq!(entry.command, "kill foo");
        assert_eq!(entry.undo_command, "run /apps/foo/vyoma.toml");

        assert!(pop_undo().is_none());
    }

    #[test]
    fn redo_after_undo() {
        let _g = reset();
        push_undo("mute", "unmute");

        let entry = pop_undo().unwrap();
        assert_eq!(entry.command, "mute");

        let redo_entry = pop_redo().unwrap();
        assert_eq!(redo_entry.command, "mute");
        assert_eq!(redo_entry.undo_command, "unmute");

        let again = pop_undo().unwrap();
        assert_eq!(again.command, "mute");
    }

    #[test]
    fn new_action_clears_redo() {
        let _g = reset();
        push_undo("kill foo", "run /apps/foo/vyoma.toml");
        let _ = pop_undo();
        push_undo("theme light", "theme dark");
        assert!(pop_redo().is_none());
    }

    #[test]
    fn stack_limit_enforced() {
        let _g = reset();
        for i in 0..(MAX_UNDO + 10) {
            push_undo(&format!("cmd {i}"), &format!("undo {i}"));
        }
        let s = state().lock().unwrap();
        assert_eq!(s.undo_stack.len(), MAX_UNDO);
        assert_eq!(s.undo_stack.last().unwrap().command, "cmd 59");
        assert_eq!(s.undo_stack.first().unwrap().command, "cmd 10");
    }

    #[test]
    fn recent_history_order() {
        let _g = reset();
        push_undo("a", "undo_a");
        push_undo("b", "undo_b");
        push_undo("c", "undo_c");

        let hist = recent_history(2);
        assert_eq!(hist.len(), 2);
        assert_eq!(hist[0].command, "c");
        assert_eq!(hist[1].command, "b");
    }

    #[test]
    fn capture_helpers() {
        let _g = reset();

        capture_kill("my-app", "/apps/my-app/vyoma.toml");
        let e = pop_undo().unwrap();
        assert_eq!(e.command, "kill my-app");
        assert_eq!(e.undo_command, "run /apps/my-app/vyoma.toml");

        capture_run("my-app", "/apps/my-app/vyoma.toml");
        let e = pop_undo().unwrap();
        assert_eq!(e.command, "run /apps/my-app/vyoma.toml");
        assert_eq!(e.undo_command, "kill my-app");

        capture_volume(50, 80);
        let e = pop_undo().unwrap();
        assert_eq!(e.command, "volume 80");
        assert_eq!(e.undo_command, "volume 50");

        capture_mute(false);
        let e = pop_undo().unwrap();
        assert_eq!(e.command, "mute");
        assert_eq!(e.undo_command, "unmute");

        capture_mute(true);
        let e = pop_undo().unwrap();
        assert_eq!(e.command, "unmute");
        assert_eq!(e.undo_command, "mute");

        capture_theme("dark", "light");
        let e = pop_undo().unwrap();
        assert_eq!(e.command, "theme light");
        assert_eq!(e.undo_command, "theme dark");

        capture_workspace(0, 2);
        let e = pop_undo().unwrap();
        assert_eq!(e.command, "workspace 2");
        assert_eq!(e.undo_command, "workspace 0");

        capture_wallpaper(0x0D1117FF, 0xFF0000FF);
        let e = pop_undo().unwrap();
        assert_eq!(e.command, format!("wallpaper {}", 0xFF0000FFu32));
        assert_eq!(e.undo_command, format!("wallpaper {}", 0x0D1117FFu32));
    }
}
