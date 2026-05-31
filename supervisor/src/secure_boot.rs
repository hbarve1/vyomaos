// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P91: Secure boot chain verification.
//!
//! At startup the supervisor hashes critical system binaries and compares them
//! against expected values from `/etc/vyoma/boot-manifest.toml`.  Results are
//! logged per-component and a global `BOOT_VERIFIED` flag is set.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use crate::{log_info, log_warn, log_error};
use supervisor::logging::Subsystem;

// ── Constants ────────────────────────────────────────────────────────────────

const BOOT_MANIFEST_PATH: &str = "/etc/vyoma/boot-manifest.toml";
const WASMTIME_PATH: &str = "/usr/bin/wasmtime";
const SELF_EXE_PATH: &str = "/proc/self/exe";
const PROC_CMDLINE: &str = "/proc/cmdline";

/// Global boot-verified flag: `true` when all components match the manifest
/// (or when no manifest is present — advisory mode).
pub static BOOT_VERIFIED: OnceLock<bool> = OnceLock::new();

// ── Boot manifest model ─────────────────────────────────────────────────────

/// On-disk TOML representation of the boot manifest.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct BootManifestToml {
    pub boot: BootManifestEntries,
}

#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct BootManifestEntries {
    #[serde(default)]
    pub supervisor_sha256: String,
    #[serde(default)]
    pub wasmtime_sha256: String,
}

/// Runtime result of the boot-chain verification.
#[derive(Debug, Clone)]
pub struct BootManifest {
    pub kernel_booted: bool,
    pub supervisor_sha256: String,
    pub wasmtime_sha256: String,
    pub supervisor_ok: VerifyResult,
    pub wasmtime_ok: VerifyResult,
    pub verified: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VerifyResult {
    /// Hash matched the manifest.
    Pass,
    /// Hash did not match the manifest.
    Fail,
    /// Component file could not be read.
    Missing,
    /// No manifest on disk — advisory only.
    Skipped,
}

// ── SHA-256 helper ───────────────────────────────────────────────────────────

/// Compute the lowercase hex SHA-256 of a file.  Returns `None` on I/O error.
pub fn sha256_file(path: &str) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    Some(format!("{:x}", Sha256::digest(&bytes)))
}

/// Compare two hex SHA-256 strings (case-insensitive).
pub fn sha256_matches(actual: &str, expected: &str) -> bool {
    actual.eq_ignore_ascii_case(expected)
}

// ── Boot-chain verification ─────────────────────────────────────────────────

/// Verify the boot chain by hashing critical binaries and comparing them to
/// the expected values in the boot manifest (if present).
///
/// When no manifest exists the hashes are still computed and logged but every
/// component is marked `Skipped` and `verified` is set to `true` (advisory
/// mode — the system is trusted by default on first boot).
pub fn verify_boot_chain() -> BootManifest {
    // 1. Kernel presence check via /proc/cmdline
    let kernel_booted = Path::new(PROC_CMDLINE).exists();

    // 2. Compute hashes of critical binaries
    let supervisor_hash = sha256_file(SELF_EXE_PATH).unwrap_or_default();
    let wasmtime_hash = sha256_file(WASMTIME_PATH).unwrap_or_default();

    // 3. Load manifest (if any)
    let manifest = load_boot_manifest();

    // 4. Compare
    let (supervisor_ok, wasmtime_ok) = match manifest {
        Some(ref m) => {
            let s = verify_component("supervisor", &supervisor_hash, &m.boot.supervisor_sha256);
            let w = verify_component("wasmtime", &wasmtime_hash, &m.boot.wasmtime_sha256);
            (s, w)
        }
        None => (VerifyResult::Skipped, VerifyResult::Skipped),
    };

    let verified = match (&supervisor_ok, &wasmtime_ok) {
        (VerifyResult::Pass, VerifyResult::Pass) => true,
        (VerifyResult::Skipped, VerifyResult::Skipped) => true, // advisory mode
        _ => false,
    };

    BootManifest {
        kernel_booted,
        supervisor_sha256: supervisor_hash,
        wasmtime_sha256: wasmtime_hash,
        supervisor_ok,
        wasmtime_ok,
        verified,
    }
}

fn verify_component(name: &str, actual: &str, expected: &str) -> VerifyResult {
    if expected.is_empty() {
        return VerifyResult::Skipped;
    }
    if actual.is_empty() {
        return VerifyResult::Missing;
    }
    if sha256_matches(actual, expected) {
        VerifyResult::Pass
    } else {
        log_error!(Subsystem::Lifecycle, None,
            "secure-boot: {name} hash mismatch  expected={expected}  actual={actual}");
        VerifyResult::Fail
    }
}

// ── Manifest I/O ─────────────────────────────────────────────────────────────

fn load_boot_manifest() -> Option<BootManifestToml> {
    let raw = fs::read_to_string(BOOT_MANIFEST_PATH).ok()?;
    parse_boot_manifest(&raw)
}

/// Parse a boot-manifest TOML string.  Public for unit testing.
pub fn parse_boot_manifest(raw: &str) -> Option<BootManifestToml> {
    toml::from_str(raw).ok()
}

/// Generate a TOML boot manifest from the current system state.
pub fn generate_boot_manifest() -> String {
    let supervisor_hash = sha256_file(SELF_EXE_PATH).unwrap_or_default();
    let wasmtime_hash = sha256_file(WASMTIME_PATH).unwrap_or_default();

    format!(
        "[boot]\nsupervisor_sha256 = \"{supervisor_hash}\"\nwasmtime_sha256 = \"{wasmtime_hash}\"\n"
    )
}

// ── Startup integration ──────────────────────────────────────────────────────

/// Run the full boot-chain verification, log results, and set the global flag.
/// Called once from `main()` right after filesystems are mounted.
pub fn run_boot_verification() {
    let result = verify_boot_chain();

    // Log per-component results
    log_info!(Subsystem::Lifecycle, None,
        "secure-boot: kernel_booted={}", result.kernel_booted);
    log_component("supervisor", &result.supervisor_ok, &result.supervisor_sha256);
    log_component("wasmtime", &result.wasmtime_ok, &result.wasmtime_sha256);

    if result.verified {
        log_info!(Subsystem::Lifecycle, None, "secure-boot: PASS — boot chain verified");
    } else {
        log_error!(Subsystem::Lifecycle, None, "secure-boot: FAIL — boot chain verification failed");
    }

    let _ = BOOT_VERIFIED.set(result.verified);
}

fn log_component(name: &str, result: &VerifyResult, hash: &str) {
    let tag = match result {
        VerifyResult::Pass    => "PASS",
        VerifyResult::Fail    => "FAIL",
        VerifyResult::Missing => "MISSING",
        VerifyResult::Skipped => "SKIPPED",
    };
    let short_hash = if hash.len() >= 12 { &hash[..12] } else { hash };
    log_info!(Subsystem::Lifecycle, None,
        "secure-boot: {name} {tag} sha256={short_hash}…");
}

// ── IPC handlers ─────────────────────────────────────────────────────────────

/// Handle `@supervisor: boot-verify` — reply with the boot verification status.
pub fn handle_boot_verify(sender: &str, inbox: &crate::Inbox) {
    let verified = BOOT_VERIFIED.get().copied().unwrap_or(false);
    let status = if verified { "PASS" } else { "FAIL" };
    crate::send_reply(sender, &format!("REPLY:boot-verify {status}"), inbox);
    log_info!(Subsystem::Lifecycle, None, "boot-verify query from {sender}: {status}");
}

/// Handle `@supervisor: boot-manifest-generate` — compute and write the boot
/// manifest to `/etc/vyoma/boot-manifest.toml`.
pub fn handle_boot_manifest_generate(sender: &str, inbox: &crate::Inbox) {
    let manifest_toml = generate_boot_manifest();
    match fs::write(BOOT_MANIFEST_PATH, &manifest_toml) {
        Ok(()) => {
            log_info!(Subsystem::Lifecycle, None,
                "boot-manifest-generate: wrote {BOOT_MANIFEST_PATH} ({} bytes)",
                manifest_toml.len());
            crate::send_reply(
                sender,
                &format!("REPLY:boot-manifest written to {BOOT_MANIFEST_PATH}"),
                inbox,
            );
        }
        Err(e) => {
            log_error!(Subsystem::Lifecycle, None,
                "boot-manifest-generate: failed to write {BOOT_MANIFEST_PATH}: {e}");
            crate::send_reply(
                sender,
                &format!("REPLY:error: cannot write boot manifest: {e}"),
                inbox,
            );
        }
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_file_known_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        fs::write(&path, b"VyomaOS boot").unwrap();

        let hash = sha256_file(path.to_str().unwrap()).unwrap();
        let expected = format!("{:x}", Sha256::digest(b"VyomaOS boot"));
        assert_eq!(hash, expected);
    }

    #[test]
    fn test_sha256_file_missing() {
        assert!(sha256_file("/nonexistent/path/file.bin").is_none());
    }

    #[test]
    fn test_sha256_matches_case_insensitive() {
        let lower = "abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890";
        let upper = "ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890ABCDEF1234567890";
        assert!(sha256_matches(lower, upper));
        assert!(sha256_matches(upper, lower));
    }

    #[test]
    fn test_sha256_matches_mismatch() {
        assert!(!sha256_matches("aaa", "bbb"));
    }

    #[test]
    fn test_parse_boot_manifest_valid() {
        let toml = r#"
[boot]
supervisor_sha256 = "abc123"
wasmtime_sha256   = "def456"
"#;
        let m = parse_boot_manifest(toml).unwrap();
        assert_eq!(m.boot.supervisor_sha256, "abc123");
        assert_eq!(m.boot.wasmtime_sha256, "def456");
    }

    #[test]
    fn test_parse_boot_manifest_empty_hashes() {
        let toml = "[boot]\n";
        let m = parse_boot_manifest(toml).unwrap();
        assert!(m.boot.supervisor_sha256.is_empty());
        assert!(m.boot.wasmtime_sha256.is_empty());
    }

    #[test]
    fn test_parse_boot_manifest_invalid() {
        assert!(parse_boot_manifest("not valid toml {{{{").is_none());
    }

    #[test]
    fn test_verify_component_pass() {
        let hash = "abcdef";
        assert_eq!(verify_component("test", hash, "ABCDEF"), VerifyResult::Pass);
    }

    #[test]
    fn test_verify_component_fail() {
        assert_eq!(verify_component("test", "aaa", "bbb"), VerifyResult::Fail);
    }

    #[test]
    fn test_verify_component_missing() {
        assert_eq!(verify_component("test", "", "expected"), VerifyResult::Missing);
    }

    #[test]
    fn test_verify_component_skipped() {
        assert_eq!(verify_component("test", "anything", ""), VerifyResult::Skipped);
    }

    #[test]
    fn test_generate_boot_manifest_format() {
        // generate_boot_manifest reads /proc/self/exe and /usr/bin/wasmtime;
        // on a dev machine those may or may not exist.  We only check the
        // output is valid TOML with the expected keys.
        let toml_str = generate_boot_manifest();
        let parsed = parse_boot_manifest(&toml_str);
        assert!(parsed.is_some(), "generated manifest should be valid TOML");
    }
}
