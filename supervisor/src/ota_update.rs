// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P30: OTA hot-swap update helpers.
//!
//! Provides file-copy based update with SHA-256 verification for local WASM
//! binary replacement.  The supervisor's `update-local` IPC command uses these
//! helpers to atomically replace a running app's WASM binary and restart it.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;

/// Copy a file from `src_path` to `dest_path` via a temporary file, verifying
/// that the SHA-256 of the source matches `expected_sha256`.
///
/// Steps:
///   1. Read the source file.
///   2. Compute SHA-256 and compare with `expected_sha256`.
///   3. Write to a temp file next to `dest_path`.
///   4. Atomic rename temp -> dest.
pub fn copy_and_verify(src_path: &str, expected_sha256: &str, dest_path: &str) -> Result<(), String> {
    let bytes = fs::read(src_path)
        .map_err(|e| format!("read {src_path}: {e}"))?;

    let actual = format!("{:x}", Sha256::digest(&bytes));
    if actual != expected_sha256.to_lowercase() {
        return Err(format!(
            "SHA-256 mismatch: expected {expected_sha256}, got {actual}"
        ));
    }

    let tmp = format!("{dest_path}.tmp");
    fs::write(&tmp, &bytes)
        .map_err(|e| format!("write {tmp}: {e}"))?;

    fs::rename(&tmp, dest_path)
        .map_err(|e| format!("rename {tmp} -> {dest_path}: {e}"))?;

    Ok(())
}

/// Copy a file from `src_path` to `dest_path` without SHA-256 verification.
///
/// Uses the same temp-file + atomic-rename strategy as `copy_and_verify`.
pub fn copy_no_verify(src_path: &str, dest_path: &str) -> Result<(), String> {
    let bytes = fs::read(src_path)
        .map_err(|e| format!("read {src_path}: {e}"))?;

    let tmp = format!("{dest_path}.tmp");
    fs::write(&tmp, &bytes)
        .map_err(|e| format!("write {tmp}: {e}"))?;

    fs::rename(&tmp, dest_path)
        .map_err(|e| format!("rename {tmp} -> {dest_path}: {e}"))?;

    Ok(())
}

/// Compute the SHA-256 hex digest of a file.
#[allow(dead_code)]
pub fn sha256_of_file(path: &str) -> Result<String, String> {
    let bytes = fs::read(path)
        .map_err(|e| format!("read {path}: {e}"))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

/// Resolve the WASM binary path for an app given its manifest path.
pub fn wasm_path_for_manifest(manifest_path: &str) -> Result<String, String> {
    let raw = fs::read_to_string(manifest_path)
        .map_err(|e| format!("read manifest {manifest_path}: {e}"))?;
    let m: supervisor::manifest::AppManifest = toml::from_str(&raw)
        .map_err(|e| format!("parse manifest: {e}"))?;
    let parent = Path::new(manifest_path)
        .parent()
        .unwrap_or(Path::new("/apps"));
    Ok(parent.join(&m.app.wasm).to_string_lossy().to_string())
}

/// IPC handler for `@supervisor: update-local <app> <wasm_path>`.
///
/// Copies the local WASM binary to the app's configured path, verifies SHA-256
/// if declared in the manifest, kills the old instance, and respawns.
pub fn handle_update_local(
    parts: &[&str],
    sender: &str,
    inbox: &crate::Inbox,
    focused: &crate::FocusedApp,
    app_registry: &crate::AppRegistry,
) {
    use crate::{log_info, send_reply};
    use supervisor::logging::Subsystem;

    let rest = parts.get(1).unwrap_or(&"").trim();
    let (app_name, wasm_src) = match rest.split_once(' ') {
        Some((a, p)) if !a.trim().is_empty() && !p.trim().is_empty() => {
            (a.trim().to_string(), p.trim().to_string())
        }
        _ => {
            send_reply(sender, "REPLY:error: usage: update-local <app> <wasm_path>", inbox);
            return;
        }
    };
    let entry = {
        let reg = app_registry.lock().unwrap();
        reg.get(&app_name).map(|st| st.lock().unwrap().entry.clone())
    };
    let entry = match entry {
        Some(e) => e,
        None => {
            send_reply(sender, &format!("REPLY:error: unknown app: {app_name}"), inbox);
            return;
        }
    };
    let dest = match wasm_path_for_manifest(&entry.manifest) {
        Ok(d) => d,
        Err(e) => { send_reply(sender, &format!("REPLY:error: {e}"), inbox); return; }
    };
    let manifest_sha = fs::read_to_string(&entry.manifest).ok()
        .and_then(|raw| toml::from_str::<supervisor::manifest::AppManifest>(&raw).ok())
        .and_then(|m| m.app.wasm_sha256);
    let result = if let Some(expected) = &manifest_sha {
        copy_and_verify(&wasm_src, expected, &dest)
    } else {
        copy_no_verify(&wasm_src, &dest)
    };
    if let Err(e) = result {
        send_reply(sender, &format!("REPLY:error: {e}"), inbox);
        return;
    }
    log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "update-local: installed → {dest}");
    // Kill old instance
    {
        let reg = app_registry.lock().unwrap();
        if let Some(st) = reg.get(&app_name) {
            if let Some(pid) = st.lock().unwrap().child_pid {
                #[cfg(target_os = "linux")]
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                let _ = pid;
            }
        }
    }
    // Respawn
    match crate::app_threads::spawn_app(&entry, inbox, app_registry) {
        Some(app) => {
            crate::app_threads::launch_app_threads(app, inbox, focused, app_registry);
            log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "update-local: restarted OK");
            send_reply(sender, &format!("REPLY:updated {app_name} OK"), inbox);
        }
        None => send_reply(sender, "REPLY:error: restart failed after update", inbox),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_copy_and_verify_success() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.wasm");
        let dest = dir.path().join("dest.wasm");

        let content = b"hello wasm binary content";
        fs::write(&src, content).unwrap();

        let expected = format!("{:x}", Sha256::digest(content));

        let result = copy_and_verify(
            src.to_str().unwrap(),
            &expected,
            dest.to_str().unwrap(),
        );
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
        assert_eq!(fs::read(&dest).unwrap(), content);
        // Temp file should be cleaned up by atomic rename
        assert!(!dir.path().join("dest.wasm.tmp").exists());
    }

    #[test]
    fn test_copy_and_verify_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.wasm");
        let dest = dir.path().join("dest.wasm");

        let content = b"hello wasm binary content";
        fs::write(&src, content).unwrap();

        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";

        let result = copy_and_verify(
            src.to_str().unwrap(),
            wrong_hash,
            dest.to_str().unwrap(),
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("SHA-256 mismatch"), "error was: {err}");
        // Dest file should not exist since verification failed
        assert!(!dest.exists());
    }

    #[test]
    fn test_copy_no_verify() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("source.wasm");
        let dest = dir.path().join("dest.wasm");

        let content = b"some wasm bytes";
        fs::write(&src, content).unwrap();

        let result = copy_no_verify(src.to_str().unwrap(), dest.to_str().unwrap());
        assert!(result.is_ok());
        assert_eq!(fs::read(&dest).unwrap(), content);
    }

    #[test]
    fn test_sha256_of_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        let content = b"test data";
        fs::write(&path, content).unwrap();

        let expected = format!("{:x}", Sha256::digest(content));
        let actual = sha256_of_file(path.to_str().unwrap()).unwrap();
        assert_eq!(actual, expected);
    }
}
