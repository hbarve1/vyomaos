// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Periodic app auto-update checking.
//!
//! Reads a local `/data/update-catalog.toml` file to discover available updates,
//! compares against installed app versions, and exposes IPC commands for checking
//! and applying updates. An optional background thread polls every hour.

use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use crate::lock_or_recover;

use crate::{log_info, log_warn, send_reply, AppRegistry, FocusedApp, Inbox};
use supervisor::logging::Subsystem;

// ── Data model ───────────────────────────────────────────────────────────────

/// Information about a single available app update.
#[derive(Debug, Clone, PartialEq)]
pub struct UpdateInfo {
    pub app_name: String,
    pub current_version: String,
    pub latest_version: String,
    pub update_url: String,
}

/// A single entry in the update catalog TOML file.
#[derive(Debug, serde::Deserialize)]
struct CatalogEntry {
    name: String,
    version: String,
    #[serde(default)]
    url: String,
}

/// Top-level structure of `/data/update-catalog.toml`.
#[derive(Debug, serde::Deserialize)]
struct UpdateCatalog {
    #[serde(default)]
    app: Vec<CatalogEntry>,
}

const CATALOG_PATH: &str = "/data/update-catalog.toml";
const CHECK_INTERVAL_SECS: u64 = 3600;

// ── Version comparison ───────────────────────────────────────────────────────

/// Compare two semver-style version strings (e.g. "1.2.3" vs "1.3.0").
/// Returns true if `latest` is strictly newer than `current`.
pub fn is_newer(current: &str, latest: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> {
        v.split('.')
            .map(|s| s.parse::<u64>().unwrap_or(0))
            .collect()
    };
    let cur = parse(current);
    let lat = parse(latest);
    let max_len = cur.len().max(lat.len());
    for i in 0..max_len {
        let c = cur.get(i).copied().unwrap_or(0);
        let l = lat.get(i).copied().unwrap_or(0);
        if l > c {
            return true;
        }
        if l < c {
            return false;
        }
    }
    false
}

// ── Catalog parsing ──────────────────────────────────────────────────────────

/// Parse the update catalog from a TOML string.
fn parse_catalog(toml_str: &str) -> Result<Vec<CatalogEntry>, String> {
    let catalog: UpdateCatalog =
        toml::from_str(toml_str).map_err(|e| format!("catalog parse error: {e}"))?;
    Ok(catalog.app)
}

/// Read the catalog from `catalog_path` (defaults to `/data/update-catalog.toml`).
fn read_catalog(catalog_path: &str) -> Result<Vec<CatalogEntry>, String> {
    let raw = fs::read_to_string(catalog_path)
        .map_err(|e| format!("read {catalog_path}: {e}"))?;
    parse_catalog(&raw)
}

// ── Installed version lookup ─────────────────────────────────────────────────

/// Collect installed app names and versions by reading manifests from the
/// app registry's boot entries.
fn installed_versions(app_registry: &AppRegistry) -> Vec<(String, String)> {
    let reg = lock_or_recover(&app_registry);
    let mut versions = Vec::new();
    for (name, state_arc) in reg.iter() {
        let st = lock_or_recover(&state_arc);
        let manifest_path = &st.entry.manifest;
        if let Ok(raw) = fs::read_to_string(manifest_path) {
            if let Ok(m) = toml::from_str::<supervisor::manifest::AppManifest>(&raw) {
                versions.push((name.clone(), m.app.version.clone()));
            }
        }
    }
    versions
}

// ── Check for updates ────────────────────────────────────────────────────────

/// Scan the catalog file and compare against installed app versions.
/// Returns a list of apps that have newer versions available.
pub fn check_updates(catalog_path: &str, app_registry: &AppRegistry) -> Vec<UpdateInfo> {
    let entries = match read_catalog(catalog_path) {
        Ok(e) => e,
        Err(e) => {
            log_warn!(Subsystem::Lifecycle, None, "auto-update: {e}");
            return Vec::new();
        }
    };

    let installed = installed_versions(app_registry);
    let mut updates = Vec::new();

    for entry in &entries {
        if let Some((_, current_ver)) = installed.iter().find(|(n, _)| *n == entry.name) {
            if is_newer(current_ver, &entry.version) {
                updates.push(UpdateInfo {
                    app_name: entry.name.clone(),
                    current_version: current_ver.clone(),
                    latest_version: entry.version.clone(),
                    update_url: entry.url.clone(),
                });
            }
        }
    }

    updates
}

// ── IPC handlers ─────────────────────────────────────────────────────────────

/// Handle `@supervisor: check-updates` — scan catalog and reply with results.
pub fn handle_check_updates(
    sender: &str,
    inbox: &Inbox,
    app_registry: &AppRegistry,
) {
    let updates = check_updates(CATALOG_PATH, app_registry);
    if updates.is_empty() {
        send_reply(sender, "REPLY:no updates available", inbox);
    } else {
        let lines: Vec<String> = updates
            .iter()
            .map(|u| format!("{} {} -> {}", u.app_name, u.current_version, u.latest_version))
            .collect();
        send_reply(sender, &format!("REPLY:updates available|{}", lines.join("|")), inbox);
    }
    log_info!(Subsystem::Lifecycle, None, "check-updates: {} update(s) found", updates.len());
}

/// Handle `@supervisor: auto-update <app>` — download and apply update.
///
/// Reads the catalog to find the URL for the given app, copies the WASM binary
/// via `ota_update::copy_no_verify`, kills the old instance, and respawns.
pub fn handle_auto_update(
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
    focused: &FocusedApp,
    app_registry: &AppRegistry,
) {
    let app_name = match parts.get(1).map(|s| s.trim()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            send_reply(sender, "REPLY:error: usage: auto-update <app>", inbox);
            return;
        }
    };

    // Read catalog to find the update entry
    let entries = match read_catalog(CATALOG_PATH) {
        Ok(e) => e,
        Err(e) => {
            send_reply(sender, &format!("REPLY:error: {e}"), inbox);
            return;
        }
    };
    let catalog_entry = match entries.iter().find(|e| e.name == app_name) {
        Some(e) => e,
        None => {
            send_reply(
                sender,
                &format!("REPLY:error: app '{app_name}' not in update catalog"),
                inbox,
            );
            return;
        }
    };

    // Resolve the installed WASM path from manifest
    let entry = {
        let reg = lock_or_recover(&app_registry);
        reg.get(&app_name).map(|st| lock_or_recover(&st).entry.clone())
    };
    let entry = match entry {
        Some(e) => e,
        None => {
            send_reply(sender, &format!("REPLY:error: unknown app: {app_name}"), inbox);
            return;
        }
    };
    let dest = match crate::ota_update::wasm_path_for_manifest(&entry.manifest) {
        Ok(d) => d,
        Err(e) => {
            send_reply(sender, &format!("REPLY:error: {e}"), inbox);
            return;
        }
    };

    // The URL field points to a local path (e.g. /data/updates/my-app.wasm)
    let src = &catalog_entry.url;
    if src.is_empty() {
        send_reply(
            sender,
            &format!("REPLY:error: no update URL for '{app_name}'"),
            inbox,
        );
        return;
    }

    // Copy the new WASM binary
    if !Path::new(src).exists() {
        send_reply(
            sender,
            &format!("REPLY:error: update source not found: {src}"),
            inbox,
        );
        return;
    }
    if let Err(e) = crate::ota_update::copy_no_verify(src, &dest) {
        send_reply(sender, &format!("REPLY:error: {e}"), inbox);
        return;
    }

    log_info!(
        Subsystem::Lifecycle,
        Some(app_name.as_str()),
        "auto-update: installed {} v{} -> {}",
        app_name,
        catalog_entry.version,
        dest
    );

    // Kill old instance
    {
        let reg = lock_or_recover(&app_registry);
        if let Some(st) = reg.get(&app_name) {
            if let Some(pid) = lock_or_recover(&st).child_pid {
                #[cfg(target_os = "linux")]
                unsafe {
                    libc::kill(pid as libc::pid_t, libc::SIGKILL);
                }
                let _ = pid;
            }
        }
    }

    // Respawn
    match crate::app_threads::spawn_app(&entry, inbox, app_registry) {
        Some(app) => {
            crate::app_threads::launch_app_threads(app, inbox, focused, app_registry);
            log_info!(
                Subsystem::Lifecycle,
                Some(app_name.as_str()),
                "auto-update: restarted OK"
            );
            send_reply(
                sender,
                &format!("REPLY:auto-updated {app_name} to v{}", catalog_entry.version),
                inbox,
            );
        }
        None => send_reply(sender, "REPLY:error: restart failed after auto-update", inbox),
    }
}

// ── Background check thread ──────────────────────────────────────────────────

/// Spawn a background thread that checks for updates every `CHECK_INTERVAL_SECS`
/// seconds (1 hour). If updates are found, sends a notification via the IPC
/// `@supervisor: notify` mechanism.
pub fn spawn_background_checker(inbox: &Inbox, app_registry: &AppRegistry) {
    let inbox = Arc::clone(inbox);
    let registry = Arc::clone(app_registry);

    thread::Builder::new()
        .name("auto-update-checker".into())
        .spawn(move || {
            loop {
                thread::sleep(Duration::from_secs(CHECK_INTERVAL_SECS));

                let updates = check_updates(CATALOG_PATH, &registry);
                if !updates.is_empty() {
                    let names: Vec<&str> = updates.iter().map(|u| u.app_name.as_str()).collect();
                    let msg = format!(
                        "Updates available for: {}",
                        names.join(", ")
                    );
                    log_info!(
                        Subsystem::Lifecycle,
                        None,
                        "auto-update background: {}",
                        msg
                    );
                    // Send notification via supervisor notify mechanism
                    crate::toast::enqueue_banner_with_icon(
                        "supervisor",
                        "Auto-Update",
                        &msg,
                        None,
                    );
                }
            }
        })
        .expect("spawn auto-update-checker thread");
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_basic() {
        assert!(is_newer("1.0.0", "1.0.1"));
        assert!(is_newer("1.0.0", "1.1.0"));
        assert!(is_newer("1.0.0", "2.0.0"));
        assert!(!is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("2.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.0")); // equal => not newer
    }

    #[test]
    fn test_is_newer_different_lengths() {
        assert!(is_newer("1.0", "1.0.1"));
        assert!(is_newer("1", "1.1"));
        assert!(!is_newer("1.0.1", "1.0"));
        assert!(!is_newer("1.1", "1"));
    }

    #[test]
    fn test_is_newer_multi_digit() {
        assert!(is_newer("1.9.0", "1.10.0"));
        assert!(is_newer("0.99.0", "1.0.0"));
        assert!(!is_newer("1.10.0", "1.9.0"));
    }

    #[test]
    fn test_parse_catalog_valid() {
        let toml = r#"
[[app]]
name = "hello-world"
version = "1.1.0"
url = "/data/updates/hello-world.wasm"

[[app]]
name = "calculator"
version = "2.0.0"
url = "/data/updates/calculator.wasm"
"#;
        let entries = parse_catalog(toml).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "hello-world");
        assert_eq!(entries[0].version, "1.1.0");
        assert_eq!(entries[0].url, "/data/updates/hello-world.wasm");
        assert_eq!(entries[1].name, "calculator");
        assert_eq!(entries[1].version, "2.0.0");
    }

    #[test]
    fn test_parse_catalog_empty() {
        let entries = parse_catalog("").unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_catalog_no_url() {
        let toml = r#"
[[app]]
name = "myapp"
version = "1.0.0"
"#;
        let entries = parse_catalog(toml).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, ""); // default empty
    }

    #[test]
    fn test_parse_catalog_invalid() {
        let result = parse_catalog("this is not valid toml [[[");
        assert!(result.is_err());
    }

    #[test]
    fn test_is_newer_empty_strings() {
        assert!(!is_newer("", ""));
        assert!(is_newer("", "1.0.0"));
        assert!(!is_newer("1.0.0", ""));
    }

    #[test]
    fn test_is_newer_non_numeric() {
        // Non-numeric parts parse as 0
        assert!(!is_newer("abc", "def")); // both parse as 0.0.0-ish
        assert!(is_newer("0.0.0", "0.0.1"));
    }

    #[test]
    fn test_update_info_struct() {
        let info = UpdateInfo {
            app_name: "test-app".to_string(),
            current_version: "1.0.0".to_string(),
            latest_version: "2.0.0".to_string(),
            update_url: "/data/updates/test-app.wasm".to_string(),
        };
        assert_eq!(info.app_name, "test-app");
        assert_eq!(info.current_version, "1.0.0");
        assert_eq!(info.latest_version, "2.0.0");
        assert_eq!(info.update_url, "/data/updates/test-app.wasm");
    }
}
