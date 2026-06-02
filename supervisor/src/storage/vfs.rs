// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P51: Virtual File System (VFS) abstraction.
//!
//! Provides a mount table, path sandboxing, and per-app permission checks
//! that sit between IPC commands and the real filesystem.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use crate::lock_or_recover;

use crate::{log_info, send_reply, AppRegistry, Inbox};
use supervisor::logging::Subsystem;

// ── VFS model ────────────────────────────────────────────────────────────────

/// Storage backend for a mount point.
#[derive(Debug, Clone, PartialEq)]
pub enum VfsBackend {
    /// Backed by on-disk storage (e.g. /data via 9P).
    Local,
    /// In-memory tmpfs-style storage (not persisted across reboots).
    Memory,
}

/// A single mount in the VFS mount table.
#[derive(Debug, Clone)]
pub struct VfsMount {
    pub mount_point: String,
    pub backend: VfsBackend,
    pub read_only: bool,
}

// ── Mount table (global, initialised once) ───────────────────────────────────

static MOUNT_TABLE: OnceLock<Mutex<Vec<VfsMount>>> = OnceLock::new();

fn mount_table() -> &'static Mutex<Vec<VfsMount>> {
    MOUNT_TABLE.get_or_init(|| {
        Mutex::new(vec![VfsMount {
            mount_point: "/data".to_string(),
            backend: VfsBackend::Local,
            read_only: false,
        }])
    })
}

/// Add a mount to the global table. Returns `Err` if the mount point is already present.
pub fn add_mount(mount: VfsMount) -> Result<(), String> {
    let mut table = lock_or_recover(&mount_table());
    if table.iter().any(|m| m.mount_point == mount.mount_point) {
        return Err(format!("mount point {} already exists", mount.mount_point));
    }
    table.push(mount);
    Ok(())
}

/// List all active mounts (cloned snapshot).
pub fn list_mounts() -> Vec<VfsMount> {
    lock_or_recover(&mount_table()).clone()
}

// ── Path sandboxing ──────────────────────────────────────────────────────────

/// Resolve and sandbox `path` against the mount table.
///
/// Returns `(real_path, mount_index)` on success.
/// Rejects paths that escape the mount point via `..` traversal, or that do
/// not fall under any registered mount.
fn resolve_path(path: &str) -> Result<(PathBuf, usize), String> {
    let table = lock_or_recover(&mount_table());

    // Normalise the requested path (no fs access yet — purely lexical).
    let req = lexical_clean(path);

    // Find the longest-prefix mount.
    let mut best: Option<(usize, usize)> = None; // (index, prefix_len)
    for (i, m) in table.iter().enumerate() {
        let mp = &m.mount_point;
        if req == *mp || req.starts_with(&format!("{mp}/")) {
            let plen = mp.len();
            if best.map_or(true, |(_, bl)| plen > bl) {
                best = Some((i, plen));
            }
        }
    }

    let (idx, _) = best.ok_or_else(|| format!("path {path} is not under any mount"))?;
    let mount = &table[idx];

    // Verify the resolved path stays inside the mount point.
    // Use lexical canonicalization to avoid TOCTOU — we only check the
    // logical path, not what the filesystem actually resolves to.
    let canon = lexical_clean(&req);
    let mp = &mount.mount_point;
    if canon != *mp && !canon.starts_with(&format!("{mp}/")) {
        return Err(format!("path traversal rejected: {path} escapes {mp}"));
    }

    Ok((PathBuf::from(&canon), idx))
}

/// Lexical path cleaning: collapse `.`, `..`, duplicate slashes.
/// Does NOT touch the filesystem (no symlink resolution).
fn lexical_clean(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => { parts.pop(); }
            other => parts.push(other),
        }
    }
    format!("/{}", parts.join("/"))
}

// ── App permission check ─────────────────────────────────────────────────────

/// Check whether `app` has the `filesystem` capability by inspecting the
/// app registry for manifest data.
fn app_has_filesystem(app: &str, app_registry: &AppRegistry) -> bool {
    let reg = lock_or_recover(&app_registry);
    let Some(state_arc) = reg.get(app) else { return false };
    let state = lock_or_recover(&state_arc);

    // Re-parse the manifest to check capabilities.filesystem.
    // The entry.manifest path is stored in AppState.
    let manifest_path = &state.entry.manifest;
    match supervisor::manifest::parse_manifest(Path::new(manifest_path)) {
        Ok(m) => m.capabilities.filesystem,
        Err(_) => false,
    }
}

// ── VFS operations ───────────────────────────────────────────────────────────

/// Read a file. Checks that `app` has filesystem access and the path is
/// inside a mount point.
pub fn vfs_read(app: &str, path: &str, app_registry: &AppRegistry) -> Result<Vec<u8>, String> {
    if !app_has_filesystem(app, app_registry) {
        return Err(format!("app {app} does not have filesystem capability"));
    }
    let (real_path, _idx) = resolve_path(path)?;
    fs::read(&real_path).map_err(|e| format!("read {}: {e}", real_path.display()))
}

/// Write data to a file. Checks permissions, mount read-only flag, and
/// path sandboxing.
pub fn vfs_write(
    app: &str, path: &str, data: &[u8], app_registry: &AppRegistry,
) -> Result<(), String> {
    if !app_has_filesystem(app, app_registry) {
        return Err(format!("app {app} does not have filesystem capability"));
    }
    let (real_path, idx) = resolve_path(path)?;
    let table = lock_or_recover(&mount_table());
    if table[idx].read_only {
        return Err(format!("mount {} is read-only", table[idx].mount_point));
    }
    drop(table);

    // Ensure parent directory exists.
    if let Some(parent) = real_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    fs::write(&real_path, data).map_err(|e| format!("write {}: {e}", real_path.display()))
}

/// List entries in a directory.
pub fn vfs_list(app: &str, path: &str, app_registry: &AppRegistry) -> Result<Vec<String>, String> {
    if !app_has_filesystem(app, app_registry) {
        return Err(format!("app {app} does not have filesystem capability"));
    }
    let (real_path, _idx) = resolve_path(path)?;
    let entries = fs::read_dir(&real_path)
        .map_err(|e| format!("readdir {}: {e}", real_path.display()))?;
    let mut names: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
            let suffix = if entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false) {
                "/"
            } else {
                ""
            };
            names.push(format!("{name}{suffix}"));
        }
    }
    names.sort();
    Ok(names)
}

/// Delete a file or empty directory.
pub fn vfs_delete(app: &str, path: &str, app_registry: &AppRegistry) -> Result<(), String> {
    if !app_has_filesystem(app, app_registry) {
        return Err(format!("app {app} does not have filesystem capability"));
    }
    let (real_path, idx) = resolve_path(path)?;
    let table = lock_or_recover(&mount_table());
    if table[idx].read_only {
        return Err(format!("mount {} is read-only", table[idx].mount_point));
    }
    drop(table);

    if real_path.is_dir() {
        fs::remove_dir(&real_path)
            .map_err(|e| format!("rmdir {}: {e}", real_path.display()))
    } else {
        fs::remove_file(&real_path)
            .map_err(|e| format!("rm {}: {e}", real_path.display()))
    }
}

/// Return file metadata: size and type (file / dir / unknown).
pub fn vfs_stat(app: &str, path: &str, app_registry: &AppRegistry) -> Result<String, String> {
    if !app_has_filesystem(app, app_registry) {
        return Err(format!("app {app} does not have filesystem capability"));
    }
    let (real_path, _idx) = resolve_path(path)?;
    let meta = fs::metadata(&real_path)
        .map_err(|e| format!("stat {}: {e}", real_path.display()))?;
    let kind = if meta.is_dir() { "dir" } else if meta.is_file() { "file" } else { "other" };
    Ok(format!("{kind} {}", meta.len()))
}

// ── IPC command handler ──────────────────────────────────────────────────────

/// Handle `@supervisor: vfs-*` commands. Returns `true` if the verb was
/// recognised, `false` otherwise.
pub fn handle_vfs_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
    app_registry: &AppRegistry,
) -> bool {
    match verb {
        "vfs-list" => {
            let path = parts.get(1).unwrap_or(&"/data").trim();
            match vfs_list(sender, path, app_registry) {
                Ok(entries) => {
                    let joined = if entries.is_empty() {
                        "(empty)".to_string()
                    } else {
                        entries.join("|")
                    };
                    log_info!(Subsystem::Ipc, Some(sender), "vfs-list {path}: {} entries", entries.len());
                    send_reply(sender, &format!("REPLY:vfs-list {joined}"), inbox);
                }
                Err(e) => send_reply(sender, &format!("REPLY:vfs-list error: {e}"), inbox),
            }
        }

        "vfs-stat" => {
            let path = parts.get(1).unwrap_or(&"").trim();
            if path.is_empty() {
                send_reply(sender, "REPLY:vfs-stat error: usage: vfs-stat <path>", inbox);
                return true;
            }
            match vfs_stat(sender, path, app_registry) {
                Ok(info) => {
                    log_info!(Subsystem::Ipc, Some(sender), "vfs-stat {path}: {info}");
                    send_reply(sender, &format!("REPLY:vfs-stat {path} {info}"), inbox);
                }
                Err(e) => send_reply(sender, &format!("REPLY:vfs-stat error: {e}"), inbox),
            }
        }

        "vfs-mounts" => {
            let mounts = list_mounts();
            let lines: Vec<String> = mounts.iter().map(|m| {
                let backend = match m.backend {
                    VfsBackend::Local => "local",
                    VfsBackend::Memory => "memory",
                };
                let ro = if m.read_only { "ro" } else { "rw" };
                format!("{} {} {}", m.mount_point, backend, ro)
            }).collect();
            let joined = lines.join("|");
            log_info!(Subsystem::Ipc, Some(sender), "vfs-mounts: {} mount(s)", mounts.len());
            send_reply(sender, &format!("REPLY:vfs-mounts {joined}"), inbox);
        }

        _ => return false,
    }
    true
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_clean_basic() {
        assert_eq!(lexical_clean("/data/foo/bar"), "/data/foo/bar");
        assert_eq!(lexical_clean("/data/./foo"), "/data/foo");
        assert_eq!(lexical_clean("/data/foo/../bar"), "/data/bar");
        assert_eq!(lexical_clean("/data///foo"), "/data/foo");
        assert_eq!(lexical_clean("/"), "/");
    }

    #[test]
    fn lexical_clean_traversal_to_root() {
        // Attempting to go above root collapses to /
        assert_eq!(lexical_clean("/data/../../etc/passwd"), "/etc/passwd");
        assert_eq!(lexical_clean("/../../../etc/shadow"), "/etc/shadow");
    }

    #[test]
    fn resolve_path_valid() {
        // Ensure mount table is initialised with /data default.
        let _ = mount_table();
        let (p, idx) = resolve_path("/data/foo.txt").unwrap();
        assert_eq!(p, PathBuf::from("/data/foo.txt"));
        assert_eq!(idx, 0);
    }

    #[test]
    fn resolve_path_mount_root() {
        let _ = mount_table();
        let (p, _) = resolve_path("/data").unwrap();
        assert_eq!(p, PathBuf::from("/data"));
    }

    #[test]
    fn resolve_path_traversal_rejected() {
        let _ = mount_table();
        // Try to escape /data via ..
        let result = resolve_path("/data/../etc/passwd");
        assert!(result.is_err(), "traversal should be rejected");
        assert!(result.unwrap_err().contains("not under any mount"));
    }

    #[test]
    fn resolve_path_outside_mount() {
        let _ = mount_table();
        let result = resolve_path("/etc/passwd");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not under any mount"));
    }

    #[test]
    fn resolve_path_double_dot_inside_mount() {
        let _ = mount_table();
        // /data/subdir/../file.txt should resolve to /data/file.txt (still inside mount)
        let (p, _) = resolve_path("/data/subdir/../file.txt").unwrap();
        assert_eq!(p, PathBuf::from("/data/file.txt"));
    }

    #[test]
    fn mount_table_default() {
        let mounts = list_mounts();
        assert!(!mounts.is_empty());
        assert_eq!(mounts[0].mount_point, "/data");
        assert_eq!(mounts[0].backend, VfsBackend::Local);
        assert!(!mounts[0].read_only);
    }

    #[test]
    fn add_mount_duplicate_rejected() {
        // The /data mount is already in the table from init.
        let result = add_mount(VfsMount {
            mount_point: "/data".to_string(),
            backend: VfsBackend::Memory,
            read_only: false,
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn vfs_backend_debug() {
        // Ensure Debug derive works.
        let local = VfsBackend::Local;
        let mem = VfsBackend::Memory;
        assert_eq!(format!("{local:?}"), "Local");
        assert_eq!(format!("{mem:?}"), "Memory");
    }
}
