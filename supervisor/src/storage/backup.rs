// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P107: Backup and restore — tar.gz system configuration snapshots.
//!
//! Creates, lists, restores, and deletes backup archives under `/data/backups/`.
//! Each backup is a `.tar.gz` containing configuration files and user data
//! from `/data/`.

use std::fs;
use std::io::Write;
use std::path::Path;

use flate2::write::GzEncoder;
use flate2::Compression;

use supervisor::archive::extract_tar_gz;

// ── Constants ────────────────────────────────────────────────────────────────

const BACKUPS_DIR: &str = "/data/backups";
const TAR_BLOCK: usize = 512;

/// Files (relative to `/data/`) included in every backup.
const BACKUP_FILES: &[&str] = &[
    "settings.toml",
    "session.toml",
    "users.toml",
    "firewall.toml",
    "user-caps.toml",
];

/// Directories (relative to `/data/`) included in every backup.
const BACKUP_DIRS: &[&str] = &["users"];

// ── BackupInfo ───────────────────────────────────────────────────────────────

/// Metadata for a single backup archive.
#[derive(Debug, Clone, PartialEq)]
pub struct BackupInfo {
    pub name: String,
    pub path: String,
    pub created_secs: u64,
    pub size_bytes: u64,
    pub includes: Vec<String>,
}

/// Serialize a `BackupInfo` to a human-readable string.
pub fn format_backup_info(info: &BackupInfo) -> String {
    format!(
        "{} ({} bytes, {} items, {}s)",
        info.name,
        info.size_bytes,
        info.includes.len(),
        info.created_secs,
    )
}

// ── Tar creation helpers ─────────────────────────────────────────────────────

/// Build a minimal POSIX tar header block.
fn make_tar_header(name: &str, size: usize, type_flag: u8) -> [u8; TAR_BLOCK] {
    let mut block = [0u8; TAR_BLOCK];
    let name_bytes = name.as_bytes();
    let len = name_bytes.len().min(100);
    block[..len].copy_from_slice(&name_bytes[..len]);

    // Size field at offset 124, 12 bytes, octal, nul-terminated.
    let size_str = format!("{:011o}", size);
    block[124..124 + size_str.len()].copy_from_slice(size_str.as_bytes());

    // Type flag at offset 156.
    block[156] = type_flag;

    // Compute and write checksum (offset 148, 8 bytes).
    // During checksum calculation, the checksum field is treated as spaces.
    block[148..156].copy_from_slice(b"        ");
    let cksum: u32 = block.iter().map(|&b| b as u32).sum();
    let cksum_str = format!("{:06o}\0 ", cksum);
    let cksum_bytes = cksum_str.as_bytes();
    let ck_len = cksum_bytes.len().min(8);
    block[148..148 + ck_len].copy_from_slice(&cksum_bytes[..ck_len]);

    block
}

/// Append a file entry (header + data + padding) to a tar buffer.
fn tar_append_file(tar_buf: &mut Vec<u8>, archive_path: &str, content: &[u8]) {
    let header = make_tar_header(archive_path, content.len(), b'0');
    tar_buf.extend_from_slice(&header);
    tar_buf.extend_from_slice(content);
    let remainder = content.len() % TAR_BLOCK;
    if remainder != 0 {
        let padding = TAR_BLOCK - remainder;
        tar_buf.extend(std::iter::repeat(0u8).take(padding));
    }
}

/// Append a directory entry to a tar buffer.
fn tar_append_dir(tar_buf: &mut Vec<u8>, dir_path: &str) {
    let path = if dir_path.ends_with('/') {
        dir_path.to_string()
    } else {
        format!("{dir_path}/")
    };
    let header = make_tar_header(&path, 0, b'5');
    tar_buf.extend_from_slice(&header);
}

/// Recursively collect all files from a directory into the tar buffer.
/// Returns a list of archive paths that were added.
fn tar_append_dir_recursive(
    tar_buf: &mut Vec<u8>,
    fs_base: &Path,
    archive_prefix: &str,
    included: &mut Vec<String>,
) {
    let entries = match fs::read_dir(fs_base) {
        Ok(e) => e,
        Err(_) => return,
    };

    tar_append_dir(tar_buf, archive_prefix);

    let mut sorted: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    sorted.sort_by_key(|e| e.file_name());

    for entry in sorted {
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        let archive_path = format!("{archive_prefix}{name_str}");

        if ft.is_file() {
            if let Ok(content) = fs::read(entry.path()) {
                tar_append_file(tar_buf, &archive_path, &content);
                included.push(archive_path);
            }
        } else if ft.is_dir() {
            let sub_prefix = format!("{archive_path}/");
            tar_append_dir_recursive(tar_buf, &entry.path(), &sub_prefix, included);
        }
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Generate a default backup name based on boot-relative timestamp.
pub fn default_backup_name() -> String {
    let secs = crate::BOOT_INSTANT
        .get()
        .map(|i| i.elapsed().as_secs())
        .unwrap_or(0);
    format!("backup-{secs}")
}

/// Create a backup archive at `/data/backups/<name>.tar.gz`.
///
/// Includes all configuration files and user data directories that exist.
pub fn create_backup(name: &str) -> Result<BackupInfo, String> {
    if name.is_empty() {
        return Err("backup name cannot be empty".to_string());
    }
    if name.contains('/') || name.contains("..") {
        return Err("backup name must not contain path separators".to_string());
    }

    fs::create_dir_all(BACKUPS_DIR)
        .map_err(|e| format!("cannot create {BACKUPS_DIR}: {e}"))?;

    let archive_path = format!("{BACKUPS_DIR}/{name}.tar.gz");

    let mut tar_buf: Vec<u8> = Vec::new();
    let mut included: Vec<String> = Vec::new();

    // Add individual config files.
    for rel in BACKUP_FILES {
        let full = format!("/data/{rel}");
        if let Ok(content) = fs::read(&full) {
            tar_append_file(&mut tar_buf, rel, &content);
            included.push(rel.to_string());
        }
    }

    // Add directories recursively.
    for dir in BACKUP_DIRS {
        let full = Path::new("/data").join(dir);
        if full.is_dir() {
            let prefix = format!("{dir}/");
            tar_append_dir_recursive(&mut tar_buf, &full, &prefix, &mut included);
        }
    }

    if included.is_empty() {
        return Err("nothing to back up — no config files or user data found".to_string());
    }

    // End-of-archive marker: two 512-byte zero blocks.
    tar_buf.extend_from_slice(&[0u8; 1024]);

    // Gzip compress.
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&tar_buf)
        .map_err(|e| format!("gzip encode error: {e}"))?;
    let compressed = encoder.finish().map_err(|e| format!("gzip finish error: {e}"))?;

    fs::write(&archive_path, &compressed)
        .map_err(|e| format!("write {archive_path}: {e}"))?;

    let created_secs = crate::BOOT_INSTANT
        .get()
        .map(|i| i.elapsed().as_secs())
        .unwrap_or(0);

    Ok(BackupInfo {
        name: name.to_string(),
        path: archive_path,
        created_secs,
        size_bytes: compressed.len() as u64,
        includes: included,
    })
}

/// Restore a backup archive from `/data/backups/<name>.tar.gz` to `/data/`.
///
/// Returns a list of restored file paths.
pub fn restore_backup(name: &str) -> Result<Vec<String>, String> {
    if name.is_empty() {
        return Err("backup name cannot be empty".to_string());
    }
    if name.contains('/') || name.contains("..") {
        return Err("backup name must not contain path separators".to_string());
    }

    let archive_path = format!("{BACKUPS_DIR}/{name}.tar.gz");
    if !Path::new(&archive_path).exists() {
        return Err(format!("backup '{name}' not found at {archive_path}"));
    }

    let restored = extract_tar_gz(&archive_path, "/data")?;
    Ok(restored)
}

/// List all available backup archives in `/data/backups/`.
pub fn list_backups() -> Vec<BackupInfo> {
    let dir = Path::new(BACKUPS_DIR);
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut backups: Vec<BackupInfo> = Vec::new();

    for entry in entries.filter_map(|e| e.ok()) {
        let name_os = entry.file_name();
        let fname = name_os.to_string_lossy();
        if !fname.ends_with(".tar.gz") {
            continue;
        }
        let name = fname.trim_end_matches(".tar.gz").to_string();
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        let path = entry.path().to_string_lossy().to_string();

        backups.push(BackupInfo {
            name,
            path,
            created_secs: 0,
            size_bytes: size,
            includes: Vec::new(),
        });
    }

    backups.sort_by(|a, b| a.name.cmp(&b.name));
    backups
}

/// Delete a backup archive.
pub fn delete_backup(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("backup name cannot be empty".to_string());
    }
    if name.contains('/') || name.contains("..") {
        return Err("backup name must not contain path separators".to_string());
    }

    let archive_path = format!("{BACKUPS_DIR}/{name}.tar.gz");
    if !Path::new(&archive_path).exists() {
        return Err(format!("backup '{name}' not found"));
    }

    fs::remove_file(&archive_path)
        .map_err(|e| format!("delete {archive_path}: {e}"))?;
    Ok(())
}

// ── IPC handler ──────────────────────────────────────────────────────────────

/// Handle `backup-*` IPC commands. Returns `true` if handled.
pub fn handle_backup_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &crate::Inbox,
) -> bool {
    match verb {
        "backup-create" => {
            let name = parts
                .get(1)
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(default_backup_name);
            match create_backup(&name) {
                Ok(info) => {
                    let msg = format!(
                        "REPLY:backup created: {} ({} bytes, {} items)",
                        info.name,
                        info.size_bytes,
                        info.includes.len(),
                    );
                    crate::send_reply(sender, &msg, inbox);
                }
                Err(e) => {
                    crate::send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
        }

        "backup-list" => {
            let backups = list_backups();
            if backups.is_empty() {
                crate::send_reply(sender, "REPLY:no backups found", inbox);
            } else {
                let rows: Vec<String> = backups
                    .iter()
                    .map(|b| format!("{} ({} bytes)", b.name, b.size_bytes))
                    .collect();
                crate::send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox);
            }
        }

        "backup-restore" => {
            let name = match parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                Some(n) => n,
                None => {
                    crate::send_reply(
                        sender,
                        "REPLY:error: usage: backup-restore <name>",
                        inbox,
                    );
                    return true;
                }
            };
            match restore_backup(name) {
                Ok(files) => {
                    let msg = format!("REPLY:restored {} files from backup '{name}'", files.len());
                    crate::send_reply(sender, &msg, inbox);
                }
                Err(e) => {
                    crate::send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
        }

        "backup-delete" => {
            let name = match parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty()) {
                Some(n) => n,
                None => {
                    crate::send_reply(
                        sender,
                        "REPLY:error: usage: backup-delete <name>",
                        inbox,
                    );
                    return true;
                }
            };
            match delete_backup(name) {
                Ok(()) => {
                    crate::send_reply(
                        sender,
                        &format!("REPLY:deleted backup '{name}'"),
                        inbox,
                    );
                }
                Err(e) => {
                    crate::send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
        }

        _ => return false,
    }
    true
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_backup_info_output() {
        let info = BackupInfo {
            name: "test-backup".to_string(),
            path: "/data/backups/test-backup.tar.gz".to_string(),
            created_secs: 42,
            size_bytes: 1024,
            includes: vec!["settings.toml".to_string(), "session.toml".to_string()],
        };
        let out = format_backup_info(&info);
        assert!(out.contains("test-backup"));
        assert!(out.contains("1024 bytes"));
        assert!(out.contains("2 items"));
        assert!(out.contains("42s"));
    }

    #[test]
    fn backup_info_equality() {
        let a = BackupInfo {
            name: "snap".to_string(),
            path: "/data/backups/snap.tar.gz".to_string(),
            created_secs: 10,
            size_bytes: 512,
            includes: vec!["users.toml".to_string()],
        };
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn default_name_is_nonempty() {
        // BOOT_INSTANT may not be initialized in tests; function still returns a name.
        let name = default_backup_name();
        assert!(name.starts_with("backup-"));
        assert!(name.len() > "backup-".len());
    }

    #[test]
    fn reject_empty_name() {
        assert!(create_backup("").is_err());
        assert!(restore_backup("").is_err());
        assert!(delete_backup("").is_err());
    }

    #[test]
    fn reject_path_traversal() {
        assert!(create_backup("../evil").is_err());
        assert!(restore_backup("foo/bar").is_err());
        assert!(delete_backup("../etc/passwd").is_err());
    }

    #[test]
    fn make_tar_header_fields() {
        let hdr = make_tar_header("hello.txt", 42, b'0');
        // Name starts at offset 0.
        assert_eq!(&hdr[..9], b"hello.txt");
        // Type flag at offset 156.
        assert_eq!(hdr[156], b'0');
        // Size field at offset 124 is octal "00000000052".
        let size_field = std::str::from_utf8(&hdr[124..135]).unwrap();
        assert_eq!(size_field, "00000000052");
    }

    #[test]
    fn tar_roundtrip_in_memory() {
        let mut tar_buf: Vec<u8> = Vec::new();
        let content = b"key = \"value\"\n";
        tar_append_file(&mut tar_buf, "test.toml", content);
        tar_buf.extend_from_slice(&[0u8; 1024]); // end marker

        // Verify we can parse the header back.
        let hdr = supervisor::archive::parse_tar_header(&tar_buf[..TAR_BLOCK]);
        assert!(hdr.is_some());
        let hdr = hdr.unwrap();
        assert_eq!(hdr.path, "test.toml");
        assert_eq!(hdr.size, content.len());
    }

    #[test]
    fn list_backups_empty_dir() {
        // When the backups dir doesn't exist, should return empty vec.
        let backups = list_backups();
        // On CI /data/backups may not exist; just verify it doesn't panic.
        let _ = backups;
    }
}
