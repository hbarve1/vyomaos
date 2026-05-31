// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P59: Package registry backend.
//!
//! Manages a local TOML-based registry of available packages at `/data/registry.toml`.
//! Provides search, lookup, add, and remove operations, plus IPC command handlers.

use serde::{Deserialize, Serialize};
use std::fs;

/// Path to the persistent registry file.
const REGISTRY_PATH: &str = "/data/registry.toml";

/// A single entry in the package registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PackageEntry {
    pub name: String,
    pub version: String,
    pub description: String,
    pub wasm_url: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub category: String,
}

/// Wrapper for TOML serialisation of `[[package]]` array.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RegistryFile {
    #[serde(default)]
    package: Vec<PackageEntry>,
}

/// Load all package entries from `/data/registry.toml`.
///
/// Returns an empty vec if the file does not exist or cannot be parsed.
pub fn load_registry() -> Vec<PackageEntry> {
    let raw = match fs::read_to_string(REGISTRY_PATH) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    match toml::from_str::<RegistryFile>(&raw) {
        Ok(rf) => rf.package,
        Err(_) => Vec::new(),
    }
}

/// Save the given entries to `/data/registry.toml`.
pub fn save_registry(entries: &[PackageEntry]) -> Result<(), String> {
    let rf = RegistryFile { package: entries.to_vec() };
    let raw = toml::to_string_pretty(&rf)
        .map_err(|e| format!("serialise registry: {e}"))?;
    fs::write(REGISTRY_PATH, raw)
        .map_err(|e| format!("write {REGISTRY_PATH}: {e}"))
}

/// Search registry entries by substring match on name or description (case-insensitive).
pub fn search<'a>(entries: &'a [PackageEntry], query: &str) -> Vec<&'a PackageEntry> {
    let q = query.to_lowercase();
    entries
        .iter()
        .filter(|e| {
            e.name.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
        })
        .collect()
}

/// Exact name lookup.
pub fn get<'a>(entries: &'a [PackageEntry], name: &str) -> Option<&'a PackageEntry> {
    entries.iter().find(|e| e.name == name)
}

/// Add an entry to the registry. Replaces any existing entry with the same name.
pub fn add_entry(entries: &mut Vec<PackageEntry>, entry: PackageEntry) {
    entries.retain(|e| e.name != entry.name);
    entries.push(entry);
}

/// Remove an entry by name. Returns `true` if an entry was removed.
pub fn remove_entry(entries: &mut Vec<PackageEntry>, name: &str) -> bool {
    let before = entries.len();
    entries.retain(|e| e.name != name);
    entries.len() < before
}

// ---------------------------------------------------------------------------
// IPC command handlers
// ---------------------------------------------------------------------------

use crate::{send_reply, Inbox};

/// Handle registry IPC commands. Returns `true` if the command was recognised.
pub fn handle_registry_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "registry-search" => {
            let query = parts.get(1).unwrap_or(&"").trim();
            if query.is_empty() {
                send_reply(sender, "REPLY:error: usage: registry-search <query>", inbox);
                return true;
            }
            let entries = load_registry();
            let results = search(&entries, query);
            if results.is_empty() {
                send_reply(sender, "REPLY:registry-search: no matches", inbox);
            } else {
                let rows: Vec<String> = results
                    .iter()
                    .map(|e| format!("{} v{} [{}] — {}", e.name, e.version, e.category, e.description))
                    .collect();
                send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox);
            }
        }
        "registry-list" => {
            let entries = load_registry();
            if entries.is_empty() {
                send_reply(sender, "REPLY:registry empty", inbox);
            } else {
                let rows: Vec<String> = entries
                    .iter()
                    .map(|e| format!("{} v{} [{}] {} bytes", e.name, e.version, e.category, e.size_bytes))
                    .collect();
                send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox);
            }
        }
        "registry-add" => {
            // Usage: registry-add <name> <version> <url> <sha256>
            let rest = parts.get(1).unwrap_or(&"").trim();
            let args: Vec<&str> = rest.splitn(4, ' ').collect();
            if args.len() < 4 || args.iter().any(|a| a.is_empty()) {
                send_reply(
                    sender,
                    "REPLY:error: usage: registry-add <name> <version> <url> <sha256>",
                    inbox,
                );
                return true;
            }
            let entry = PackageEntry {
                name: args[0].to_string(),
                version: args[1].to_string(),
                description: String::new(),
                wasm_url: args[2].to_string(),
                sha256: args[3].to_string(),
                size_bytes: 0,
                category: "user".to_string(),
            };
            let mut entries = load_registry();
            add_entry(&mut entries, entry.clone());
            match save_registry(&entries) {
                Ok(()) => send_reply(
                    sender,
                    &format!("REPLY:registry-add {} v{} ok", entry.name, entry.version),
                    inbox,
                ),
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_entries() -> Vec<PackageEntry> {
        vec![
            PackageEntry {
                name: "editor".into(),
                version: "1.0.0".into(),
                description: "A simple text editor".into(),
                wasm_url: "https://example.com/editor.wasm".into(),
                sha256: "abc123".into(),
                size_bytes: 4096,
                category: "productivity".into(),
            },
            PackageEntry {
                name: "calculator".into(),
                version: "2.1.0".into(),
                description: "Basic arithmetic calculator".into(),
                wasm_url: "https://example.com/calc.wasm".into(),
                sha256: "def456".into(),
                size_bytes: 2048,
                category: "utility".into(),
            },
            PackageEntry {
                name: "game-snake".into(),
                version: "0.5.0".into(),
                description: "Classic snake game".into(),
                wasm_url: "https://example.com/snake.wasm".into(),
                sha256: "789ghi".into(),
                size_bytes: 8192,
                category: "games".into(),
            },
        ]
    }

    #[test]
    fn test_search_by_name() {
        let entries = sample_entries();
        let results = search(&entries, "calc");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "calculator");
    }

    #[test]
    fn test_search_by_description() {
        let entries = sample_entries();
        let results = search(&entries, "snake");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "game-snake");
    }

    #[test]
    fn test_search_case_insensitive() {
        let entries = sample_entries();
        let results = search(&entries, "EDITOR");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "editor");
    }

    #[test]
    fn test_search_no_match() {
        let entries = sample_entries();
        let results = search(&entries, "nonexistent");
        assert!(results.is_empty());
    }

    #[test]
    fn test_get_found() {
        let entries = sample_entries();
        let result = get(&entries, "editor");
        assert!(result.is_some());
        assert_eq!(result.unwrap().version, "1.0.0");
    }

    #[test]
    fn test_get_not_found() {
        let entries = sample_entries();
        assert!(get(&entries, "missing").is_none());
    }

    #[test]
    fn test_add_entry_new() {
        let mut entries = sample_entries();
        let new = PackageEntry {
            name: "browser".into(),
            version: "0.1.0".into(),
            description: "Web browser".into(),
            wasm_url: "https://example.com/browser.wasm".into(),
            sha256: "aaa111".into(),
            size_bytes: 16384,
            category: "internet".into(),
        };
        add_entry(&mut entries, new);
        assert_eq!(entries.len(), 4);
        assert!(get(&entries, "browser").is_some());
    }

    #[test]
    fn test_add_entry_replaces_existing() {
        let mut entries = sample_entries();
        let updated = PackageEntry {
            name: "editor".into(),
            version: "2.0.0".into(),
            description: "Updated editor".into(),
            wasm_url: "https://example.com/editor2.wasm".into(),
            sha256: "new_hash".into(),
            size_bytes: 5120,
            category: "productivity".into(),
        };
        add_entry(&mut entries, updated);
        assert_eq!(entries.len(), 3); // same count — replaced, not appended
        assert_eq!(get(&entries, "editor").unwrap().version, "2.0.0");
    }

    #[test]
    fn test_remove_entry_existing() {
        let mut entries = sample_entries();
        let removed = remove_entry(&mut entries, "calculator");
        assert!(removed);
        assert_eq!(entries.len(), 2);
        assert!(get(&entries, "calculator").is_none());
    }

    #[test]
    fn test_remove_entry_missing() {
        let mut entries = sample_entries();
        let removed = remove_entry(&mut entries, "nope");
        assert!(!removed);
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_roundtrip_serialisation() {
        let entries = sample_entries();
        let rf = RegistryFile { package: entries.clone() };
        let raw = toml::to_string_pretty(&rf).unwrap();
        let parsed: RegistryFile = toml::from_str(&raw).unwrap();
        assert_eq!(parsed.package, entries);
    }
}
