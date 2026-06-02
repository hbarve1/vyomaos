// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P112: App Store launch infrastructure.
//!
//! Provides a remote-catalog-backed app store with fetch, search, install, and
//! uninstall operations.  The catalog is downloaded via `http_get`, parsed as
//! TOML, and cached locally.  Install verifies SHA-256 before copying the WASM
//! binary into `/apps/<name>/`.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

use crate::{send_reply, Inbox};

const STORE_CONFIG_PATH: &str = "/data/store-config.toml";
const STORE_CACHE_DIR: &str = "/data/store-cache";
const CACHED_CATALOG_PATH: &str = "/data/store-cache/catalog.toml";

/// Persistent store configuration at `/data/store-config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreConfig {
    pub catalog_url: String,
    pub local_cache: String,
    pub auto_check: bool,
    pub last_check_secs: u64,
}

impl Default for StoreConfig {
    fn default() -> Self {
        Self {
            catalog_url: String::new(),
            local_cache: STORE_CACHE_DIR.to_string(),
            auto_check: false,
            last_check_secs: 0,
        }
    }
}

/// A single entry in the remote store catalog.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StoreEntry {
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub url: String,
    pub description: String,
    pub category: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub icon_url: String,
}

/// Top-level TOML wrapper: `[[app]]` array.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct StoreCatalog {
    #[serde(default)]
    app: Vec<StoreEntry>,
}

/// Load store config from disk; returns defaults if missing or unparseable.
pub fn load_config() -> StoreConfig {
    match fs::read_to_string(STORE_CONFIG_PATH) {
        Ok(raw) => toml::from_str(&raw).unwrap_or_default(),
        Err(_) => StoreConfig::default(),
    }
}

/// Save store config to disk.
pub fn save_config(cfg: &StoreConfig) -> Result<(), String> {
    let raw = toml::to_string_pretty(cfg)
        .map_err(|e| format!("serialise store config: {e}"))?;
    fs::write(STORE_CONFIG_PATH, raw)
        .map_err(|e| format!("write {STORE_CONFIG_PATH}: {e}"))
}

/// Parse a TOML catalog string into a list of `StoreEntry`.
pub fn parse_catalog(toml_str: &str) -> Result<Vec<StoreEntry>, String> {
    let catalog: StoreCatalog =
        toml::from_str(toml_str).map_err(|e| format!("catalog parse error: {e}"))?;
    Ok(catalog.app)
}

/// Fetch the remote catalog via HTTP, cache it locally, and return entries.
pub fn fetch_catalog(url: &str) -> Result<Vec<StoreEntry>, String> {
    if url.is_empty() {
        return Err("catalog_url is empty — set it with store-config".to_string());
    }
    let bytes = crate::net::http_get(url)
        .map_err(|e| format!("fetch catalog: {e}"))?;
    let body_raw = String::from_utf8_lossy(&bytes);
    // Strip HTTP headers if present (raw TCP response).
    let body = if let Some(pos) = body_raw.find("\r\n\r\n") {
        &body_raw[pos + 4..]
    } else if let Some(pos) = body_raw.find("\n\n") {
        &body_raw[pos + 2..]
    } else {
        &body_raw
    };
    let entries = parse_catalog(body)?;
    // Cache to disk.
    let _ = fs::create_dir_all(STORE_CACHE_DIR);
    let _ = fs::write(CACHED_CATALOG_PATH, body);
    Ok(entries)
}

/// Load the cached catalog from disk (no network).
pub fn load_cached_catalog() -> Vec<StoreEntry> {
    match fs::read_to_string(CACHED_CATALOG_PATH) {
        Ok(raw) => parse_catalog(&raw).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Search cached catalog entries by substring match on name, description, or
/// category (case-insensitive).
pub fn search_catalog<'a>(entries: &'a [StoreEntry], query: &str) -> Vec<&'a StoreEntry> {
    let q = query.to_lowercase();
    entries
        .iter()
        .filter(|e| {
            e.name.to_lowercase().contains(&q)
                || e.description.to_lowercase().contains(&q)
                || e.category.to_lowercase().contains(&q)
        })
        .collect()
}

/// Download a WASM binary from `entry.url`, verify its SHA-256, and install it
/// into `/apps/<name>/`.  Creates a minimal `vyoma.toml` manifest and appends
/// the app to `/etc/vyoma/boot.toml`.
pub fn install_from_store(entry: &StoreEntry) -> Result<(), String> {
    let app_dir = format!("/apps/{}", entry.name);
    fs::create_dir_all(&app_dir)
        .map_err(|e| format!("mkdir {app_dir}: {e}"))?;

    // Download WASM binary.
    let bytes = crate::net::http_get(&entry.url)
        .map_err(|e| format!("download {}: {e}", entry.url))?;

    // Strip HTTP headers if present.
    let wasm_bytes = strip_http_headers(&bytes);

    // Verify SHA-256.
    let actual_sha = format!("{:x}", Sha256::digest(wasm_bytes));
    if actual_sha != entry.sha256.to_lowercase() {
        return Err(format!(
            "SHA-256 mismatch: expected {}, got {actual_sha}",
            entry.sha256
        ));
    }

    // Write WASM binary.
    let wasm_path = format!("{app_dir}/{}.wasm", entry.name);
    fs::write(&wasm_path, wasm_bytes)
        .map_err(|e| format!("write {wasm_path}: {e}"))?;

    // Create vyoma.toml manifest.
    let manifest = format!(
        "[app]\nname    = \"{}\"\nversion = \"{}\"\nwasm    = \"{}.wasm\"\n\n\
         [capabilities]\nstdio = true\n",
        entry.name, entry.version, entry.name
    );
    let manifest_path = format!("{app_dir}/vyoma.toml");
    fs::write(&manifest_path, &manifest)
        .map_err(|e| format!("write {manifest_path}: {e}"))?;

    // Append to boot.toml so supervisor picks it up on next boot.
    append_to_boot_toml(&entry.name, &manifest_path)?;

    Ok(())
}

/// Remove an app's directory and its entry from boot.toml.
pub fn uninstall_app(name: &str) -> Result<(), String> {
    let app_dir = format!("/apps/{name}");
    if Path::new(&app_dir).exists() {
        fs::remove_dir_all(&app_dir)
            .map_err(|e| format!("remove {app_dir}: {e}"))?;
    }
    remove_from_boot_toml(name)?;
    Ok(())
}

const BOOT_TOML_PATH: &str = "/etc/vyoma/boot.toml";

fn append_to_boot_toml(name: &str, manifest_path: &str) -> Result<(), String> {
    let entry = format!(
        "\n[[app]]\nname     = \"{name}\"\nmanifest = \"{manifest_path}\"\n"
    );
    let mut content = fs::read_to_string(BOOT_TOML_PATH).unwrap_or_default();
    // Avoid duplicate entries.
    if content.contains(&format!("name     = \"{name}\"")) {
        return Ok(());
    }
    content.push_str(&entry);
    fs::write(BOOT_TOML_PATH, &content)
        .map_err(|e| format!("write {BOOT_TOML_PATH}: {e}"))
}

fn remove_from_boot_toml(name: &str) -> Result<(), String> {
    let content = match fs::read_to_string(BOOT_TOML_PATH) {
        Ok(c) => c,
        Err(_) => return Ok(()),
    };
    // Remove the [[app]] block that contains this name.
    let marker = format!("name     = \"{name}\"");
    let mut lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].contains(&marker) {
            // Walk backwards to find the [[app]] header.
            let mut start = i;
            while start > 0 {
                start -= 1;
                if lines[start].trim() == "[[app]]" {
                    break;
                }
            }
            // Walk forward past the block.
            let mut end = i + 1;
            while end < lines.len()
                && !lines[end].trim().starts_with("[[")
                && !lines[end].trim().is_empty()
            {
                end += 1;
            }
            lines.drain(start..end);
        } else {
            i += 1;
        }
    }
    let cleaned = lines.join("\n");
    fs::write(BOOT_TOML_PATH, &cleaned)
        .map_err(|e| format!("write {BOOT_TOML_PATH}: {e}"))
}

/// Strip HTTP response headers from raw bytes, returning only the body.
fn strip_http_headers(raw: &[u8]) -> &[u8] {
    // Look for \r\n\r\n separator.
    for i in 0..raw.len().saturating_sub(3) {
        if raw[i] == b'\r' && raw[i + 1] == b'\n' && raw[i + 2] == b'\r' && raw[i + 3] == b'\n' {
            return &raw[i + 4..];
        }
    }
    // Fall back to \n\n.
    for i in 0..raw.len().saturating_sub(1) {
        if raw[i] == b'\n' && raw[i + 1] == b'\n' {
            return &raw[i + 2..];
        }
    }
    raw
}

/// Handle store-related IPC commands.  Returns `true` if recognised.
pub fn handle_store_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "store-fetch" => {
            let cfg = load_config();
            match fetch_catalog(&cfg.catalog_url) {
                Ok(entries) => {
                    send_reply(
                        sender,
                        &format!("REPLY:store-fetch ok, {} app(s) in catalog", entries.len()),
                        inbox,
                    );
                }
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }
        "store-search" => {
            let query = parts.get(1).unwrap_or(&"").trim();
            if query.is_empty() {
                send_reply(sender, "REPLY:error: usage: store-search <query>", inbox);
                return true;
            }
            let entries = load_cached_catalog();
            let results = search_catalog(&entries, query);
            if results.is_empty() {
                send_reply(sender, "REPLY:store-search: no matches", inbox);
            } else {
                let rows: Vec<String> = results.iter().map(|e| {
                    format!("{} v{} [{}] {} — {}", e.name, e.version, e.category, e.size_bytes, e.description)
                }).collect();
                send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox);
            }
        }
        "store-install" => {
            let name = parts.get(1).unwrap_or(&"").trim();
            if name.is_empty() {
                send_reply(sender, "REPLY:error: usage: store-install <name>", inbox);
                return true;
            }
            let entries = load_cached_catalog();
            let entry = entries.iter().find(|e| e.name == name);
            match entry {
                Some(e) => match install_from_store(e) {
                    Ok(()) => send_reply(
                        sender,
                        &format!("REPLY:store-install {name} v{} ok", e.version),
                        inbox,
                    ),
                    Err(err) => {
                        send_reply(sender, &format!("REPLY:error: {err}"), inbox);
                    }
                },
                None => send_reply(
                    sender,
                    &format!("REPLY:error: '{name}' not found in catalog (run store-fetch first)"),
                    inbox,
                ),
            }
        }
        "store-uninstall" => {
            let name = parts.get(1).unwrap_or(&"").trim();
            if name.is_empty() {
                send_reply(sender, "REPLY:error: usage: store-uninstall <name>", inbox);
                return true;
            }
            match uninstall_app(name) {
                Ok(()) => send_reply(
                    sender,
                    &format!("REPLY:store-uninstall {name} ok"),
                    inbox,
                ),
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }
        "store-config" => {
            let cfg = load_config();
            let msg = format!(
                "catalog_url={} local_cache={} auto_check={} last_check={}",
                cfg.catalog_url, cfg.local_cache, cfg.auto_check, cfg.last_check_secs
            );
            send_reply(sender, &format!("REPLY:{msg}"), inbox);
        }
        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog_toml() -> &'static str {
        r#"
[[app]]
name = "notes"
version = "1.0.0"
sha256 = "aabbccdd"
url = "http://example.com/notes.wasm"
description = "Simple note-taking app"
category = "productivity"
size_bytes = 4096

[[app]]
name = "game-tetris"
version = "0.3.0"
sha256 = "11223344"
url = "http://example.com/tetris.wasm"
description = "Classic tetris game"
category = "games"
size_bytes = 8192
icon_url = "http://example.com/tetris.png"
"#
    }

    #[test]
    fn test_parse_catalog_valid() {
        let entries = parse_catalog(sample_catalog_toml()).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "notes");
        assert_eq!(entries[0].version, "1.0.0");
        assert_eq!(entries[0].sha256, "aabbccdd");
        assert_eq!(entries[0].category, "productivity");
        assert_eq!(entries[0].size_bytes, 4096);
        assert_eq!(entries[1].name, "game-tetris");
        assert_eq!(entries[1].icon_url, "http://example.com/tetris.png");
    }

    #[test]
    fn test_parse_catalog_empty() {
        let entries = parse_catalog("").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_catalog_invalid() {
        let result = parse_catalog("this is not valid toml [[[");
        assert!(result.is_err());
    }

    #[test]
    fn test_search_variants() {
        let entries = parse_catalog(sample_catalog_toml()).unwrap();
        // by name
        assert_eq!(search_catalog(&entries, "notes").len(), 1);
        assert_eq!(search_catalog(&entries, "notes")[0].name, "notes");
        // by description
        assert_eq!(search_catalog(&entries, "tetris")[0].name, "game-tetris");
        // by category
        assert_eq!(search_catalog(&entries, "games")[0].name, "game-tetris");
        // case-insensitive
        assert_eq!(search_catalog(&entries, "NOTES").len(), 1);
        // no match
        assert!(search_catalog(&entries, "nonexistent").is_empty());
    }

    #[test]
    fn test_config_default() {
        let cfg = StoreConfig::default();
        assert!(cfg.catalog_url.is_empty());
        assert_eq!(cfg.local_cache, STORE_CACHE_DIR);
        assert!(!cfg.auto_check);
        assert_eq!(cfg.last_check_secs, 0);
    }

    #[test]
    fn test_config_roundtrip() {
        let cfg = StoreConfig {
            catalog_url: "http://example.com/catalog.toml".to_string(),
            local_cache: "/data/store-cache".to_string(),
            auto_check: true,
            last_check_secs: 1234567890,
        };
        let raw = toml::to_string_pretty(&cfg).unwrap();
        let parsed: StoreConfig = toml::from_str(&raw).unwrap();
        assert_eq!(parsed.catalog_url, cfg.catalog_url);
        assert_eq!(parsed.auto_check, cfg.auto_check);
        assert_eq!(parsed.last_check_secs, cfg.last_check_secs);
    }

    #[test]
    fn test_store_entry_icon_url_default() {
        let toml_str = r#"
[[app]]
name = "minimal"
version = "0.1.0"
sha256 = "abc"
url = "http://example.com/minimal.wasm"
description = "Minimal app"
category = "util"
size_bytes = 1024
"#;
        let entries = parse_catalog(toml_str).unwrap();
        assert_eq!(entries[0].icon_url, ""); // default empty
    }

    #[test]
    fn test_strip_http_headers_crlf() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\n\r\nhello";
        let body = strip_http_headers(raw);
        assert_eq!(body, b"hello");
    }

    #[test]
    fn test_strip_http_headers_lf() {
        let raw = b"HTTP/1.1 200 OK\n\nhello";
        let body = strip_http_headers(raw);
        assert_eq!(body, b"hello");
    }

    #[test]
    fn test_strip_http_headers_none() {
        let raw = b"\x00\x61\x73\x6d"; // raw WASM magic
        let body = strip_http_headers(raw);
        assert_eq!(body, raw);
    }

    #[test]
    fn test_catalog_roundtrip_serialisation() {
        let entries = parse_catalog(sample_catalog_toml()).unwrap();
        let catalog = StoreCatalog { app: entries.clone() };
        let raw = toml::to_string_pretty(&catalog).unwrap();
        let parsed = parse_catalog(&raw).unwrap();
        assert_eq!(parsed, entries);
    }
}
