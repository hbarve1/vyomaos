// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P52: File associations — open files with the default app for their type.
//!
//! Provides a registry of file-extension-to-app mappings with hardcoded defaults
//! and user-customisable overrides persisted at `/data/file-associations.toml`.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::lock_or_recover;
use std::sync::OnceLock;

/// Path where user-defined association overrides are stored.
const CUSTOM_ASSOC_PATH: &str = "/data/file-associations.toml";

// ── Model ────────────────────────────────────────────────────────────────────

/// A single file-type association.
#[derive(Debug, Clone, PartialEq)]
pub struct FileAssociation {
    pub extension: String,
    pub app_name: String,
    pub action: String,
}

// ── Default associations ─────────────────────────────────────────────────────

/// Hardcoded default associations shipped with VyomaOS.
fn default_associations() -> Vec<FileAssociation> {
    let text_exts = ["txt", "md", "toml", "rs", "log"];
    let image_exts = ["png", "jpg", "bmp"];

    let mut assocs: Vec<FileAssociation> = Vec::new();
    for ext in &text_exts {
        assocs.push(FileAssociation {
            extension: (*ext).to_string(),
            app_name: "text-editor".to_string(),
            action: "open".to_string(),
        });
    }
    for ext in &image_exts {
        assocs.push(FileAssociation {
            extension: (*ext).to_string(),
            app_name: "image-viewer".to_string(),
            action: "open".to_string(),
        });
    }
    assocs.push(FileAssociation {
        extension: "wasm".to_string(),
        app_name: "app-store".to_string(),
        action: "open".to_string(),
    });
    assocs
}

// ── Global registry (defaults + custom overrides) ────────────────────────────

/// Thread-safe association registry initialised once on first access.
static REGISTRY: OnceLock<Mutex<HashMap<String, FileAssociation>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, FileAssociation>> {
    REGISTRY.get_or_init(|| {
        let mut map = HashMap::new();
        for a in default_associations() {
            map.insert(a.extension.clone(), a);
        }
        // Overlay user-defined overrides (best-effort; ignore errors).
        if let Ok(overrides) = load_custom_overrides() {
            for a in overrides {
                map.insert(a.extension.clone(), a);
            }
        }
        Mutex::new(map)
    })
}

// ── Custom overrides (TOML persistence) ──────────────────────────────────────

/// Load custom overrides from `/data/file-associations.toml`.
///
/// Expected format:
/// ```toml
/// [associations]
/// txt = "my-editor"
/// png = "my-viewer"
/// ```
fn load_custom_overrides() -> Result<Vec<FileAssociation>, String> {
    let raw = std::fs::read_to_string(CUSTOM_ASSOC_PATH)
        .map_err(|e| format!("cannot read {CUSTOM_ASSOC_PATH}: {e}"))?;
    parse_custom_toml(&raw)
}

/// Parse the TOML string from the custom overrides file.
fn parse_custom_toml(raw: &str) -> Result<Vec<FileAssociation>, String> {
    let doc: toml::Value = toml::from_str(raw)
        .map_err(|e| format!("invalid TOML: {e}"))?;
    let table = doc
        .get("associations")
        .and_then(|v| v.as_table())
        .ok_or_else(|| "missing [associations] table".to_string())?;

    let mut out = Vec::new();
    for (ext, val) in table {
        if let Some(app) = val.as_str() {
            out.push(FileAssociation {
                extension: ext.clone(),
                app_name: app.to_string(),
                action: "open".to_string(),
            });
        }
    }
    Ok(out)
}

/// Save the current custom overrides to disk.
fn save_custom_overrides(map: &HashMap<String, FileAssociation>) -> Result<(), String> {
    // Collect only entries that differ from defaults.
    let defaults: HashMap<String, String> = default_associations()
        .into_iter()
        .map(|a| (a.extension, a.app_name))
        .collect();

    let mut custom_table = toml::value::Table::new();
    for (ext, assoc) in map {
        let is_custom = defaults.get(ext).map_or(true, |d| d != &assoc.app_name);
        if is_custom {
            custom_table.insert(
                ext.clone(),
                toml::Value::String(assoc.app_name.clone()),
            );
        }
    }

    if custom_table.is_empty() {
        // Remove the file if no custom overrides remain.
        let _ = std::fs::remove_file(CUSTOM_ASSOC_PATH);
        return Ok(());
    }

    let mut root = toml::value::Table::new();
    root.insert("associations".to_string(), toml::Value::Table(custom_table));
    let serialised = toml::to_string(&toml::Value::Table(root))
        .map_err(|e| format!("serialise error: {e}"))?;
    std::fs::write(CUSTOM_ASSOC_PATH, serialised)
        .map_err(|e| format!("write error: {e}"))?;
    Ok(())
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Extract the file extension from a path (lowercase, no leading dot).
pub fn extract_extension(path: &str) -> Option<String> {
    let filename = path.rsplit('/').next().unwrap_or(path);
    let ext = filename.rsplit('.').next()?;
    // If ext == filename there was no dot.
    if ext == filename {
        return None;
    }
    Some(ext.to_ascii_lowercase())
}

/// Look up the app that should handle the given file path.
/// Returns the app name if a matching association exists.
pub fn app_for_file(path: &str) -> Option<String> {
    let ext = extract_extension(path)?;
    let map = lock_or_recover(&registry());
    map.get(&ext).map(|a| a.app_name.clone())
}

/// Return all current associations sorted by extension.
pub fn list_associations() -> Vec<FileAssociation> {
    let map = lock_or_recover(&registry());
    let mut v: Vec<FileAssociation> = map.values().cloned().collect();
    v.sort_by(|a, b| a.extension.cmp(&b.extension));
    v
}

/// Set a custom association for the given extension, persisting to disk.
pub fn set_association(ext: &str, app_name: &str) -> Result<(), String> {
    let ext_lower = ext.to_ascii_lowercase();
    let assoc = FileAssociation {
        extension: ext_lower.clone(),
        app_name: app_name.to_string(),
        action: "open".to_string(),
    };
    let mut map = lock_or_recover(&registry());
    map.insert(ext_lower, assoc);
    save_custom_overrides(&map)
}

/// Format the `VYOMA_SYSTEM:open:<path>` message sent to the target app.
pub fn format_open_message(path: &str) -> String {
    format!("VYOMA_SYSTEM:open:{path}")
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_extension_basic() {
        assert_eq!(extract_extension("readme.txt"), Some("txt".to_string()));
        assert_eq!(extract_extension("/data/photo.PNG"), Some("png".to_string()));
        assert_eq!(extract_extension("archive.tar.gz"), Some("gz".to_string()));
    }

    #[test]
    fn extract_extension_no_dot() {
        assert_eq!(extract_extension("Makefile"), None);
        assert_eq!(extract_extension("/data/noext"), None);
    }

    #[test]
    fn extract_extension_hidden_file() {
        // ".gitignore" — extension is "gitignore"
        assert_eq!(extract_extension(".gitignore"), Some("gitignore".to_string()));
    }

    #[test]
    fn extract_extension_trailing_dot() {
        // "file." — extension is empty string
        assert_eq!(extract_extension("file."), Some(String::new()));
    }

    #[test]
    fn default_lookup_text() {
        let defaults: HashMap<String, FileAssociation> = default_associations()
            .into_iter()
            .map(|a| (a.extension.clone(), a))
            .collect();
        assert_eq!(defaults.get("txt").unwrap().app_name, "text-editor");
        assert_eq!(defaults.get("md").unwrap().app_name, "text-editor");
        assert_eq!(defaults.get("toml").unwrap().app_name, "text-editor");
        assert_eq!(defaults.get("rs").unwrap().app_name, "text-editor");
        assert_eq!(defaults.get("log").unwrap().app_name, "text-editor");
    }

    #[test]
    fn default_lookup_image() {
        let defaults: HashMap<String, FileAssociation> = default_associations()
            .into_iter()
            .map(|a| (a.extension.clone(), a))
            .collect();
        assert_eq!(defaults.get("png").unwrap().app_name, "image-viewer");
        assert_eq!(defaults.get("jpg").unwrap().app_name, "image-viewer");
        assert_eq!(defaults.get("bmp").unwrap().app_name, "image-viewer");
    }

    #[test]
    fn default_lookup_wasm() {
        let defaults: HashMap<String, FileAssociation> = default_associations()
            .into_iter()
            .map(|a| (a.extension.clone(), a))
            .collect();
        assert_eq!(defaults.get("wasm").unwrap().app_name, "app-store");
    }

    #[test]
    fn custom_override_parsing() {
        let toml = r#"
[associations]
txt = "nano"
csv = "spreadsheet"
"#;
        let overrides = parse_custom_toml(toml).unwrap();
        assert_eq!(overrides.len(), 2);
        let map: HashMap<String, String> = overrides
            .into_iter()
            .map(|a| (a.extension, a.app_name))
            .collect();
        assert_eq!(map.get("txt").unwrap(), "nano");
        assert_eq!(map.get("csv").unwrap(), "spreadsheet");
    }

    #[test]
    fn custom_override_bad_toml() {
        let result = parse_custom_toml("not valid {{{");
        assert!(result.is_err());
    }

    #[test]
    fn custom_override_missing_table() {
        let result = parse_custom_toml("[other]\nfoo = \"bar\"");
        assert!(result.is_err());
    }

    #[test]
    fn format_open_msg() {
        assert_eq!(
            format_open_message("/data/notes.txt"),
            "VYOMA_SYSTEM:open:/data/notes.txt"
        );
    }
}
