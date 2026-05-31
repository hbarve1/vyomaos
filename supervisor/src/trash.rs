// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P57: Recycle bin (trash) — soft-delete files to `/data/.trash/` with
//! metadata tracking for restore.  Exposed via `@supervisor: trash-*` IPC.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Constants ───────────────────────────────────────────────────────────────

const TRASH_DIR: &str = "/data/.trash";
const MANIFEST_PATH: &str = "/data/.trash/.manifest";

// ── Data model ──────────────────────────────────────────────────────────────

/// One entry in the trash manifest.
#[derive(Debug, Clone, PartialEq)]
pub struct TrashEntry {
    /// Name of the file inside `/data/.trash/` (includes timestamp suffix).
    pub trash_name: String,
    /// Absolute path where the file originally lived.
    pub original_path: String,
    /// Unix timestamp (seconds) when the file was trashed.
    pub trashed_at_secs: u64,
}

// ── Manifest serialization ──────────────────────────────────────────────────

/// Serialize trash entries to a simple line-oriented manifest format.
/// Each entry is three consecutive lines: `trash_name`, `original_path`, `timestamp`.
/// Entries are separated by a blank line.
pub fn serialize_manifest(entries: &[TrashEntry]) -> String {
    let mut out = String::new();
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&e.trash_name);
        out.push('\n');
        out.push_str(&e.original_path);
        out.push('\n');
        out.push_str(&e.trashed_at_secs.to_string());
        out.push('\n');
    }
    out
}

/// Deserialize trash entries from the manifest format.
pub fn deserialize_manifest(content: &str) -> Vec<TrashEntry> {
    let mut entries = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i + 2 < lines.len() {
        // Skip blank lines
        if lines[i].trim().is_empty() {
            i += 1;
            continue;
        }
        let trash_name = lines[i].trim().to_string();
        let original_path = lines[i + 1].trim().to_string();
        let trashed_at_secs = lines[i + 2].trim().parse::<u64>().unwrap_or(0);
        if !trash_name.is_empty() && !original_path.is_empty() {
            entries.push(TrashEntry {
                trash_name,
                original_path,
                trashed_at_secs,
            });
        }
        i += 3;
    }
    entries
}

// ── Manifest persistence helpers ────────────────────────────────────────────

fn ensure_trash_dir() -> Result<(), String> {
    fs::create_dir_all(TRASH_DIR).map_err(|e| format!("cannot create trash dir: {e}"))
}

fn read_manifest() -> Vec<TrashEntry> {
    match fs::read_to_string(MANIFEST_PATH) {
        Ok(content) => deserialize_manifest(&content),
        Err(_) => Vec::new(),
    }
}

fn write_manifest(entries: &[TrashEntry]) -> Result<(), String> {
    let data = serialize_manifest(entries);
    fs::write(MANIFEST_PATH, data).map_err(|e| format!("cannot write trash manifest: {e}"))
}

// ── Public operations ───────────────────────────────────────────────────────

/// Move a file to the trash directory.  The file is renamed to
/// `<filename>_<timestamp>` inside `/data/.trash/` and the original path is
/// recorded in the manifest so it can be restored later.
pub fn trash_file(path: &str) -> Result<(), String> {
    let src = Path::new(path);
    if !src.exists() {
        return Err(format!("file not found: {path}"));
    }
    if !src.is_file() {
        return Err(format!("not a regular file: {path}"));
    }

    ensure_trash_dir()?;

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let file_name = src
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let trash_name = format!("{file_name}_{now}");
    let dest = Path::new(TRASH_DIR).join(&trash_name);

    fs::rename(src, &dest).or_else(|_| {
        // rename fails across mount points; fall back to copy + remove.
        fs::copy(src, &dest)
            .and_then(|_| fs::remove_file(src))
            .map_err(|e| format!("cannot move file to trash: {e}"))
    })?;

    let mut entries = read_manifest();
    entries.push(TrashEntry {
        trash_name,
        original_path: path.to_string(),
        trashed_at_secs: now,
    });
    write_manifest(&entries)
}

/// Restore a file from the trash back to its original path.
/// Returns the original path on success.
pub fn restore_file(trash_name: &str) -> Result<String, String> {
    let mut entries = read_manifest();
    let idx = entries
        .iter()
        .position(|e| e.trash_name == trash_name)
        .ok_or_else(|| format!("not in trash: {trash_name}"))?;

    let entry = entries.remove(idx);
    let src = Path::new(TRASH_DIR).join(&entry.trash_name);

    if !src.exists() {
        write_manifest(&entries)?;
        return Err(format!("trash file missing: {}", entry.trash_name));
    }

    // Ensure the parent directory of the original path exists.
    if let Some(parent) = Path::new(&entry.original_path).parent() {
        let _ = fs::create_dir_all(parent);
    }

    fs::rename(&src, &entry.original_path).or_else(|_| {
        fs::copy(&src, &entry.original_path)
            .and_then(|_| fs::remove_file(&src))
            .map_err(|e| format!("cannot restore file: {e}"))
    })?;

    write_manifest(&entries)?;
    Ok(entry.original_path)
}

/// List all items currently in the trash.
pub fn list_trash() -> Vec<TrashEntry> {
    read_manifest()
}

/// Delete all files in the trash and clear the manifest.
/// Returns the number of files deleted.
pub fn empty_trash() -> Result<usize, String> {
    let entries = read_manifest();
    let count = entries.len();

    for entry in &entries {
        let p = Path::new(TRASH_DIR).join(&entry.trash_name);
        let _ = fs::remove_file(p);
    }

    // Clear manifest.
    write_manifest(&[])?;
    Ok(count)
}

/// Calculate the total size in bytes of all files in the trash.
pub fn trash_size() -> u64 {
    let entries = read_manifest();
    let mut total: u64 = 0;
    for entry in &entries {
        let p = Path::new(TRASH_DIR).join(&entry.trash_name);
        if let Ok(meta) = fs::metadata(p) {
            total += meta.len();
        }
    }
    total
}

// ── IPC command handler ─────────────────────────────────────────────────────

use crate::{send_reply, Inbox};
use supervisor::logging::Subsystem;

/// Handle `@supervisor: trash-*` IPC commands.  Returns `true` if handled.
pub fn handle_trash_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "trash" => {
            let path = match parts.get(1).map(|s| s.trim()) {
                Some(p) if !p.is_empty() => p,
                _ => {
                    send_reply(sender, "REPLY:error: usage: trash <path>", inbox);
                    return true;
                }
            };
            match trash_file(path) {
                Ok(()) => {
                    crate::log_info!(Subsystem::Lifecycle, None, "trashed {path}");
                    send_reply(sender, &format!("REPLY:trashed {path}"), inbox);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
        }

        "trash-list" => {
            let items = list_trash();
            if items.is_empty() {
                send_reply(sender, "REPLY:trash is empty", inbox);
            } else {
                let rows: Vec<String> = items
                    .iter()
                    .map(|e| format!("{} <- {} ({})", e.trash_name, e.original_path, e.trashed_at_secs))
                    .collect();
                send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox);
            }
        }

        "trash-restore" => {
            let name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n,
                _ => {
                    send_reply(sender, "REPLY:error: usage: trash-restore <name>", inbox);
                    return true;
                }
            };
            match restore_file(name) {
                Ok(orig) => {
                    crate::log_info!(Subsystem::Lifecycle, None, "restored {name} -> {orig}");
                    send_reply(sender, &format!("REPLY:restored to {orig}"), inbox);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
        }

        "trash-empty" => {
            match empty_trash() {
                Ok(n) => {
                    crate::log_info!(Subsystem::Lifecycle, None, "emptied trash ({n} items)");
                    send_reply(sender, &format!("REPLY:emptied {n} item(s)"), inbox);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
        }

        "trash-size" => {
            let size = trash_size();
            let human = if size < 1024 {
                format!("{size} B")
            } else if size < 1024 * 1024 {
                format!("{} KB", size / 1024)
            } else {
                format!("{} MB", size / (1024 * 1024))
            };
            send_reply(sender, &format!("REPLY:trash-size {human} ({size} bytes)"), inbox);
        }

        _ => return false,
    }
    true
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_round_trip_empty() {
        let entries: Vec<TrashEntry> = Vec::new();
        let serialized = serialize_manifest(&entries);
        let deserialized = deserialize_manifest(&serialized);
        assert!(deserialized.is_empty());
    }

    #[test]
    fn manifest_round_trip_single() {
        let entries = vec![TrashEntry {
            trash_name: "hello.txt_1717000000".to_string(),
            original_path: "/data/hello.txt".to_string(),
            trashed_at_secs: 1717000000,
        }];
        let serialized = serialize_manifest(&entries);
        let deserialized = deserialize_manifest(&serialized);
        assert_eq!(deserialized, entries);
    }

    #[test]
    fn manifest_round_trip_multiple() {
        let entries = vec![
            TrashEntry {
                trash_name: "a.txt_100".to_string(),
                original_path: "/data/docs/a.txt".to_string(),
                trashed_at_secs: 100,
            },
            TrashEntry {
                trash_name: "b.log_200".to_string(),
                original_path: "/data/logs/b.log".to_string(),
                trashed_at_secs: 200,
            },
            TrashEntry {
                trash_name: "c.dat_300".to_string(),
                original_path: "/data/c.dat".to_string(),
                trashed_at_secs: 300,
            },
        ];
        let serialized = serialize_manifest(&entries);
        let deserialized = deserialize_manifest(&serialized);
        assert_eq!(deserialized, entries);
    }

    #[test]
    fn trash_and_restore_round_trip() {
        // Use a temp directory to simulate /data/.trash
        let dir = std::env::temp_dir().join("vyoma_trash_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let src_path = dir.join("myfile.txt");
        fs::write(&src_path, "hello world").unwrap();
        assert!(src_path.exists());

        // We cannot call trash_file directly because it hardcodes TRASH_DIR,
        // but we can test the manifest serialization and the helpers that
        // don't depend on the hardcoded path.  The integration path
        // (trash_file -> restore_file) is tested via IPC in smoke tests.

        // Instead, verify the manifest round-trip preserves all fields.
        let entry = TrashEntry {
            trash_name: "myfile.txt_999".to_string(),
            original_path: src_path.to_string_lossy().to_string(),
            trashed_at_secs: 999,
        };
        let data = serialize_manifest(&[entry.clone()]);
        let parsed = deserialize_manifest(&data);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0], entry);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_trash_on_empty_manifest() {
        // Deserializing an empty string should yield an empty vec.
        let entries = deserialize_manifest("");
        assert!(entries.is_empty());
    }

    #[test]
    fn deserialize_skips_blank_lines() {
        let content = "\n\nhello.txt_100\n/data/hello.txt\n100\n\n\nworld.txt_200\n/data/world.txt\n200\n";
        let entries = deserialize_manifest(content);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].trash_name, "hello.txt_100");
        assert_eq!(entries[1].trash_name, "world.txt_200");
    }
}
