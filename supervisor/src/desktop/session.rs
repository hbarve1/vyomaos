// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Session save/restore — persists window positions, workspace assignments,
//! and focus state across reboots via `/data/session.toml`.

use std::fs;

use crate::workspace;
use crate::{AppRegistry, FocusedApp, Inbox};

/// Path where session state is persisted.
const SESSION_PATH: &str = "/data/session.toml";

/// One entry per app window in the saved session.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionEntry {
    pub app_name: String,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub workspace: usize,
    pub focused: bool,
}

/// Serialize a list of session entries to TOML `[[app]]` format.
pub fn serialize_session(entries: &[SessionEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str("[[app]]\n");
        out.push_str(&format!("name = \"{}\"\n", e.app_name));
        out.push_str(&format!("x = {}\n", e.x));
        out.push_str(&format!("y = {}\n", e.y));
        out.push_str(&format!("w = {}\n", e.w));
        out.push_str(&format!("h = {}\n", e.h));
        out.push_str(&format!("workspace = {}\n", e.workspace));
        out.push_str(&format!("focused = {}\n", e.focused));
        out.push('\n');
    }
    out
}

/// Parse session entries from TOML `[[app]]` format.
/// Hand-parsed to avoid adding a toml crate dependency for this simple format.
pub fn deserialize_session(content: &str) -> Vec<SessionEntry> {
    let mut entries = Vec::new();
    let mut current: Option<SessionEntry> = None;

    for line in content.lines() {
        let l = line.trim();
        if l == "[[app]]" {
            if let Some(entry) = current.take() {
                if !entry.app_name.is_empty() {
                    entries.push(entry);
                }
            }
            current = Some(SessionEntry {
                app_name: String::new(),
                x: 0,
                y: 0,
                w: 0,
                h: 0,
                workspace: 0,
                focused: false,
            });
        } else if let Some(ref mut entry) = current {
            if let Some(v) = l.strip_prefix("name = ") {
                entry.app_name = v.trim_matches('"').to_string();
            } else if let Some(v) = l.strip_prefix("x = ") {
                entry.x = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("y = ") {
                entry.y = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("w = ") {
                entry.w = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("h = ") {
                entry.h = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("workspace = ") {
                entry.workspace = v.parse().unwrap_or(0);
            } else if let Some(v) = l.strip_prefix("focused = ") {
                entry.focused = v == "true";
            }
        }
    }
    // Don't forget the last entry
    if let Some(entry) = current {
        if !entry.app_name.is_empty() {
            entries.push(entry);
        }
    }
    entries
}

/// Build session entries from live app registry state.
fn collect_entries(
    app_registry: &AppRegistry,
    focused: &FocusedApp,
) -> Vec<SessionEntry> {
    let reg = app_registry.lock().unwrap();
    let focused_name = focused.lock().unwrap().clone().unwrap_or_default();
    let ws_mgr = workspace::manager().lock().unwrap();

    let mut entries = Vec::new();
    for (name, state_arc) in reg.iter() {
        let st = state_arc.lock().unwrap();
        if let Some((x, y, w, h)) = st.win_region {
            let ws = ws_mgr.app_workspace.get(name).copied().unwrap_or(0);
            entries.push(SessionEntry {
                app_name: name.clone(),
                x,
                y,
                w,
                h,
                workspace: ws,
                focused: name == &focused_name,
            });
        }
    }
    // Sort by name for deterministic output
    entries.sort_by(|a, b| a.app_name.cmp(&b.app_name));
    entries
}

/// Save the current session (window positions, workspaces, focus) to disk.
/// Returns the number of bytes written, or an error message.
pub fn save_session(
    app_registry: &AppRegistry,
    focused: &FocusedApp,
) -> Result<usize, String> {
    let entries = collect_entries(app_registry, focused);
    let toml_out = serialize_session(&entries);
    let len = toml_out.len();
    fs::write(SESSION_PATH, &toml_out).map_err(|e| format!("write {SESSION_PATH}: {e}"))?;
    Ok(len)
}

/// Read and parse the saved session from disk.
/// Returns an empty vec if the file does not exist or cannot be read.
pub fn restore_session() -> Vec<SessionEntry> {
    match fs::read_to_string(SESSION_PATH) {
        Ok(content) => deserialize_session(&content),
        Err(_) => Vec::new(),
    }
}

/// Apply restored session entries to the running app registry.
/// Sets win_region, workspace assignment, focus, and sends resize notifications.
/// Returns the number of windows successfully restored.
pub fn apply_session(
    entries: &[SessionEntry],
    app_registry: &AppRegistry,
    focused: &FocusedApp,
    inbox: &Inbox,
) -> usize {
    let mut restored = 0usize;
    let mut focus_target: Option<String> = None;

    for entry in entries {
        // Apply win_region
        let updated = {
            let reg = app_registry.lock().unwrap();
            if let Some(state_arc) = reg.get(&entry.app_name) {
                let mut st = state_arc.lock().unwrap();
                st.win_region = Some((entry.x, entry.y, entry.w, entry.h));
                true
            } else {
                false
            }
        };

        if updated {
            // Apply workspace assignment
            workspace::move_app_to(&entry.app_name, entry.workspace);

            // Send resize notification to the app
            let msg = format!("VYOMA_SYSTEM:resize:{},{}", entry.w, entry.h);
            crate::send_reply(&entry.app_name, &msg, inbox);

            if entry.focused {
                focus_target = Some(entry.app_name.clone());
            }
            restored += 1;
        }
    }

    // Restore focus
    if let Some(name) = focus_target {
        *focused.lock().unwrap() = Some(name);
    }

    restored
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_empty() {
        let out = serialize_session(&[]);
        assert!(out.is_empty());
    }

    #[test]
    fn serialize_single_entry() {
        let entries = vec![SessionEntry {
            app_name: "calc".to_string(),
            x: 10,
            y: 20,
            w: 300,
            h: 400,
            workspace: 1,
            focused: true,
        }];
        let out = serialize_session(&entries);
        assert!(out.contains("[[app]]"));
        assert!(out.contains("name = \"calc\""));
        assert!(out.contains("x = 10"));
        assert!(out.contains("y = 20"));
        assert!(out.contains("w = 300"));
        assert!(out.contains("h = 400"));
        assert!(out.contains("workspace = 1"));
        assert!(out.contains("focused = true"));
    }

    #[test]
    fn deserialize_empty() {
        let entries = deserialize_session("");
        assert!(entries.is_empty());
    }

    #[test]
    fn round_trip_single() {
        let original = vec![SessionEntry {
            app_name: "shell".to_string(),
            x: 0,
            y: 24,
            w: 640,
            h: 480,
            workspace: 0,
            focused: true,
        }];
        let toml = serialize_session(&original);
        let parsed = deserialize_session(&toml);
        assert_eq!(original, parsed);
    }

    #[test]
    fn round_trip_multiple() {
        let original = vec![
            SessionEntry {
                app_name: "calc".to_string(),
                x: 100,
                y: 200,
                w: 320,
                h: 240,
                workspace: 0,
                focused: false,
            },
            SessionEntry {
                app_name: "editor".to_string(),
                x: 50,
                y: 50,
                w: 800,
                h: 600,
                workspace: 2,
                focused: true,
            },
            SessionEntry {
                app_name: "term".to_string(),
                x: 0,
                y: 0,
                w: 960,
                h: 700,
                workspace: 1,
                focused: false,
            },
        ];
        let toml = serialize_session(&original);
        let parsed = deserialize_session(&toml);
        assert_eq!(original, parsed);
    }

    #[test]
    fn deserialize_ignores_unknown_keys() {
        let input = "\
[[app]]
name = \"foo\"
x = 5
y = 10
w = 100
h = 200
workspace = 0
focused = false
unknown_key = 42
";
        let entries = deserialize_session(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].app_name, "foo");
        assert_eq!(entries[0].x, 5);
    }

    #[test]
    fn deserialize_missing_fields_default_to_zero() {
        let input = "\
[[app]]
name = \"bar\"
w = 640
h = 480
";
        let entries = deserialize_session(input);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].x, 0);
        assert_eq!(entries[0].y, 0);
        assert_eq!(entries[0].workspace, 0);
        assert!(!entries[0].focused);
    }

    #[test]
    fn deserialize_skips_entries_without_name() {
        let input = "\
[[app]]
x = 10
y = 20
w = 100
h = 100
";
        let entries = deserialize_session(input);
        assert!(entries.is_empty());
    }
}
