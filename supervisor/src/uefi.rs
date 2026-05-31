// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P98: UEFI boot support framework — detects UEFI vs BIOS boot mode, reads
//! EFI variables, parses boot entries, checks Secure Boot status, and exposes
//! IPC commands (`uefi-info`, `uefi-entries`).

use std::path::Path;

use crate::Inbox;
use crate::send_reply;

// ── EFI variable representation ─────────────────────────────────────────────

/// An EFI variable read from `/sys/firmware/efi/efivars/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EfiVar {
    pub name: String,
    pub size: u64,
}

/// A UEFI boot entry parsed from EFI variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootEntry {
    pub id: String,
    pub label: String,
    pub active: bool,
}

// ── UEFI detection ──────────────────────────────────────────────────────────

const EFI_DIR: &str = "/sys/firmware/efi";
const EFIVARS_DIR: &str = "/sys/firmware/efi/efivars";

/// Check whether the system booted via UEFI by looking for `/sys/firmware/efi`.
pub fn is_uefi_boot() -> bool {
    is_uefi_boot_at(EFI_DIR)
}

/// Testable version: check if the given path exists and is a directory.
pub fn is_uefi_boot_at(efi_path: &str) -> bool {
    Path::new(efi_path).is_dir()
}

/// Return `"UEFI"` or `"BIOS"` depending on whether `/sys/firmware/efi` exists.
pub fn boot_mode() -> &'static str {
    boot_mode_at(EFI_DIR)
}

/// Testable version of `boot_mode` using a custom path.
pub fn boot_mode_at(efi_path: &str) -> &'static str {
    if is_uefi_boot_at(efi_path) {
        "UEFI"
    } else {
        "BIOS"
    }
}

// ── EFI variable reading ────────────────────────────────────────────────────

/// List EFI variables from `/sys/firmware/efi/efivars/`.
/// Returns an empty vec if the directory does not exist.
pub fn read_efi_vars() -> Vec<EfiVar> {
    read_efi_vars_from(EFIVARS_DIR)
}

/// Testable version: list EFI variables from a given directory.
pub fn read_efi_vars_from(dir: &str) -> Vec<EfiVar> {
    let path = Path::new(dir);
    if !path.is_dir() {
        return Vec::new();
    }

    let mut vars = Vec::new();
    let entries = match std::fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    for entry in entries.flatten() {
        let name = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        vars.push(EfiVar { name, size });
    }

    vars.sort_by(|a, b| a.name.cmp(&b.name));
    vars
}

// ── Boot entry parsing ──────────────────────────────────────────────────────

/// List boot entries from EFI variables. Looks for variables matching
/// `Boot[0-9A-Fa-f]{4}-*` pattern in the efivars directory.
pub fn list_boot_entries() -> Vec<BootEntry> {
    list_boot_entries_from(EFIVARS_DIR)
}

/// Testable version: parse boot entries from a given efivars directory.
///
/// EFI Boot#### variables are stored as files like:
///   `Boot0001-8be4df61-93ca-11d2-aa0d-00e098032b8c`
///
/// The file content starts with a 4-byte attributes field (u32 LE):
///   bit 0 = LOAD_OPTION_ACTIVE
/// followed by a 2-byte FilePathListLength, then a null-terminated UTF-16LE
/// description string (the label).
pub fn list_boot_entries_from(dir: &str) -> Vec<BootEntry> {
    let path = Path::new(dir);
    if !path.is_dir() {
        return Vec::new();
    }

    let entries = match std::fs::read_dir(path) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut boot_entries = Vec::new();

    for entry in entries.flatten() {
        let fname = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };

        // Match Boot####-<guid> pattern.
        if !fname.starts_with("Boot") || fname.len() < 9 {
            continue;
        }
        let id_part = &fname[4..];
        // The boot ID is the first 4 hex chars.
        if id_part.len() < 4 {
            continue;
        }
        let hex_id = &id_part[..4];
        if !hex_id.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        // Must be followed by '-' (GUID separator).
        if id_part.as_bytes().get(4) != Some(&b'-') {
            continue;
        }

        let boot_id = format!("Boot{hex_id}");

        // Try to read and parse the variable file.
        let file_path = path.join(&fname);
        let data = match std::fs::read(&file_path) {
            Ok(d) => d,
            Err(_) => continue,
        };

        let (active, label) = parse_boot_var(&data);
        boot_entries.push(BootEntry {
            id: boot_id,
            label,
            active,
        });
    }

    boot_entries.sort_by(|a, b| a.id.cmp(&b.id));
    boot_entries
}

/// Parse a Boot#### EFI variable's raw bytes.
///
/// Layout (after the 4-byte sysfs attribute prefix):
///   [0..4]  u32 LE  Attributes (bit 0 = LOAD_OPTION_ACTIVE)
///   [4..6]  u16 LE  FilePathListLength
///   [6..]   UTF-16LE null-terminated Description
///
/// The sysfs prefix is 4 bytes (efivarfs adds attributes before the data).
fn parse_boot_var(data: &[u8]) -> (bool, String) {
    // Need at least: 4 (sysfs attr) + 4 (attributes) + 2 (fplen) = 10 bytes.
    if data.len() < 10 {
        return (false, String::new());
    }

    // Skip 4 bytes of sysfs attribute prefix.
    let efi_data = &data[4..];

    // First 4 bytes: EFI Load Option Attributes (u32 LE).
    let attrs = u32::from_le_bytes([efi_data[0], efi_data[1], efi_data[2], efi_data[3]]);
    let active = (attrs & 1) != 0;

    // Bytes 4..6: FilePathListLength (u16 LE) — skip, we just need the label.
    // Bytes 6..: UTF-16LE null-terminated Description.
    let desc_start = 6;
    let label = parse_utf16le_nul(&efi_data[desc_start..]);

    (active, label)
}

/// Extract a null-terminated UTF-16LE string from a byte slice.
fn parse_utf16le_nul(data: &[u8]) -> String {
    let mut chars = Vec::new();
    let mut i = 0;
    while i + 1 < data.len() {
        let code = u16::from_le_bytes([data[i], data[i + 1]]);
        if code == 0 {
            break;
        }
        chars.push(code);
        i += 2;
    }
    String::from_utf16_lossy(&chars)
}

// ── Secure Boot status ──────────────────────────────────────────────────────

/// Check if Secure Boot is enabled by reading the `SecureBoot-*` EFI variable.
/// Returns `Some(true)` if enabled, `Some(false)` if disabled, `None` if
/// the variable cannot be read (BIOS boot, no access, etc.).
pub fn secure_boot_enabled() -> Option<bool> {
    secure_boot_enabled_at(EFIVARS_DIR)
}

/// Testable version: check Secure Boot from a given efivars directory.
pub fn secure_boot_enabled_at(dir: &str) -> Option<bool> {
    let path = Path::new(dir);
    if !path.is_dir() {
        return None;
    }

    // Look for a file matching SecureBoot-<guid>.
    let entries = std::fs::read_dir(path).ok()?;
    for entry in entries.flatten() {
        let fname = match entry.file_name().into_string() {
            Ok(n) => n,
            Err(_) => continue,
        };
        if !fname.starts_with("SecureBoot-") {
            continue;
        }

        let data = match std::fs::read(path.join(&fname)) {
            Ok(d) => d,
            Err(_) => return None,
        };

        // SecureBoot variable: 4-byte sysfs prefix + 1 byte value.
        // Value: 0 = disabled, 1 = enabled.
        if data.len() >= 5 {
            return Some(data[4] != 0);
        }
        return None;
    }

    None
}

// ── IPC command handler ─────────────────────────────────────────────────────

/// Handle `uefi-info` and `uefi-entries` IPC commands.
/// Returns `true` if the command was handled.
pub fn handle_uefi_command(
    verb: &str,
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "uefi-info" => {
            let mode = boot_mode();
            let sb = match secure_boot_enabled() {
                Some(true) => "enabled",
                Some(false) => "disabled",
                None => "unknown",
            };
            let var_count = read_efi_vars().len();
            let reply = format!(
                "REPLY:boot={mode} secure_boot={sb} efi_vars={var_count}"
            );
            send_reply(sender, &reply, inbox);
            true
        }

        "uefi-entries" => {
            let entries = list_boot_entries();
            if entries.is_empty() {
                send_reply(sender, "REPLY:no boot entries found", inbox);
            } else {
                let lines: Vec<String> = entries
                    .iter()
                    .map(|e| {
                        let status = if e.active { "active" } else { "inactive" };
                        format!("{} [{}] {}", e.id, status, e.label)
                    })
                    .collect();
                send_reply(sender, &format!("REPLY:{}", lines.join("|")), inbox);
            }
            true
        }

        _ => false,
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_uefi_boot_false_nonexistent() {
        // A path that does not exist should return false.
        assert!(!is_uefi_boot_at("/tmp/vyoma_test_nonexistent_efi_dir"));
    }

    #[test]
    fn test_is_uefi_boot_true_with_dir() {
        let dir = std::env::temp_dir().join("vyoma_test_efi_detect");
        let _ = std::fs::create_dir_all(&dir);
        assert!(is_uefi_boot_at(dir.to_str().unwrap()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_boot_mode_bios() {
        assert_eq!(boot_mode_at("/tmp/vyoma_test_no_such_dir"), "BIOS");
    }

    #[test]
    fn test_boot_mode_uefi() {
        let dir = std::env::temp_dir().join("vyoma_test_boot_mode_uefi");
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(boot_mode_at(dir.to_str().unwrap()), "UEFI");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_efi_vars_empty_on_missing_dir() {
        let vars = read_efi_vars_from("/tmp/vyoma_test_no_efivars_dir");
        assert!(vars.is_empty());
    }

    #[test]
    fn test_read_efi_vars_lists_files() {
        let dir = std::env::temp_dir().join("vyoma_test_efivars_list");
        let _ = std::fs::create_dir_all(&dir);
        // Create some fake EFI variable files.
        std::fs::write(dir.join("Timeout-guid"), b"data").unwrap();
        std::fs::write(dir.join("BootOrder-guid"), b"more data").unwrap();

        let vars = read_efi_vars_from(dir.to_str().unwrap());
        assert_eq!(vars.len(), 2);
        assert_eq!(vars[0].name, "BootOrder-guid");
        assert_eq!(vars[1].name, "Timeout-guid");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_list_boot_entries_empty_on_missing_dir() {
        let entries = list_boot_entries_from("/tmp/vyoma_test_no_boot_entries");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_list_boot_entries_parses_files() {
        let dir = std::env::temp_dir().join("vyoma_test_boot_entries");
        let _ = std::fs::create_dir_all(&dir);

        // Build a minimal Boot0001 EFI variable file.
        // 4 bytes sysfs attr + 4 bytes attributes (active=1) + 2 bytes fp_len + UTF-16LE "VyomaOS\0"
        let mut data: Vec<u8> = vec![0; 4]; // sysfs prefix
        data.extend_from_slice(&1u32.to_le_bytes()); // attributes: active
        data.extend_from_slice(&0u16.to_le_bytes()); // FilePathListLength
        // UTF-16LE "VyomaOS" + null terminator
        for c in "VyomaOS".encode_utf16() {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.extend_from_slice(&0u16.to_le_bytes()); // null terminator

        std::fs::write(
            dir.join("Boot0001-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
            &data,
        )
        .unwrap();

        // Build an inactive Boot0002 entry.
        let mut data2: Vec<u8> = vec![0; 4]; // sysfs prefix
        data2.extend_from_slice(&0u32.to_le_bytes()); // attributes: inactive
        data2.extend_from_slice(&0u16.to_le_bytes());
        for c in "Linux".encode_utf16() {
            data2.extend_from_slice(&c.to_le_bytes());
        }
        data2.extend_from_slice(&0u16.to_le_bytes());

        std::fs::write(
            dir.join("Boot0002-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
            &data2,
        )
        .unwrap();

        let entries = list_boot_entries_from(dir.to_str().unwrap());
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, "Boot0001");
        assert_eq!(entries[0].label, "VyomaOS");
        assert!(entries[0].active);
        assert_eq!(entries[1].id, "Boot0002");
        assert_eq!(entries[1].label, "Linux");
        assert!(!entries[1].active);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_secure_boot_enabled_none_on_missing() {
        assert_eq!(secure_boot_enabled_at("/tmp/vyoma_test_no_sb_dir"), None);
    }

    #[test]
    fn test_secure_boot_enabled_true() {
        let dir = std::env::temp_dir().join("vyoma_test_sb_enabled");
        let _ = std::fs::create_dir_all(&dir);

        // SecureBoot variable: 4 bytes sysfs attr + 1 byte value (1 = enabled).
        let data = vec![0, 0, 0, 0, 1];
        std::fs::write(
            dir.join("SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
            &data,
        )
        .unwrap();

        assert_eq!(secure_boot_enabled_at(dir.to_str().unwrap()), Some(true));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_secure_boot_disabled() {
        let dir = std::env::temp_dir().join("vyoma_test_sb_disabled");
        let _ = std::fs::create_dir_all(&dir);

        let data = vec![0, 0, 0, 0, 0];
        std::fs::write(
            dir.join("SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c"),
            &data,
        )
        .unwrap();

        assert_eq!(secure_boot_enabled_at(dir.to_str().unwrap()), Some(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_utf16le_nul_empty() {
        assert_eq!(parse_utf16le_nul(&[]), "");
    }

    #[test]
    fn test_parse_utf16le_nul_basic() {
        // "Hi" in UTF-16LE + null
        let data = [b'H', 0, b'i', 0, 0, 0];
        assert_eq!(parse_utf16le_nul(&data), "Hi");
    }

    #[test]
    fn test_parse_boot_var_too_short() {
        let (active, label) = parse_boot_var(&[0; 5]);
        assert!(!active);
        assert!(label.is_empty());
    }

    #[test]
    fn test_parse_boot_var_active() {
        let mut data: Vec<u8> = vec![0; 4]; // sysfs prefix
        data.extend_from_slice(&1u32.to_le_bytes()); // active
        data.extend_from_slice(&0u16.to_le_bytes()); // fp_len
        for c in "Test".encode_utf16() {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.extend_from_slice(&0u16.to_le_bytes());

        let (active, label) = parse_boot_var(&data);
        assert!(active);
        assert_eq!(label, "Test");
    }
}
