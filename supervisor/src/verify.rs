// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P28: WASM binary integrity verification.
//!
//! Computes the SHA-256 digest of a file on disk and compares it against an
//! expected hex string from the app manifest (`wasm_sha256` field).

use sha2::{Digest, Sha256};
use std::fs;

/// Verify that the WASM binary at `wasm_path` matches `expected_sha256`.
///
/// Returns `Ok(())` when the hex-encoded SHA-256 digest of the file matches
/// (case-insensitive comparison).  Returns `Err` with a human-readable
/// message on I/O failure or hash mismatch.
pub fn verify_wasm_binary(wasm_path: &str, expected_sha256: &str) -> Result<(), String> {
    let bytes = fs::read(wasm_path)
        .map_err(|e| format!("cannot read {wasm_path}: {e}"))?;

    let actual = format!("{:x}", Sha256::digest(&bytes));
    let expected = expected_sha256.to_lowercase();

    if actual == expected {
        Ok(())
    } else {
        Err(format!(
            "SHA-256 mismatch for {wasm_path}\n  expected {expected}\n  actual   {actual}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_match() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wasm");
        let content = b"hello wasm binary";
        fs::write(&path, content).unwrap();

        let expected = format!("{:x}", Sha256::digest(content));
        let result = verify_wasm_binary(path.to_str().unwrap(), &expected);
        assert!(result.is_ok(), "expected Ok, got {:?}", result);
    }

    #[test]
    fn test_sha256_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wasm");
        fs::write(&path, b"real content").unwrap();

        let wrong_hash = "0000000000000000000000000000000000000000000000000000000000000000";
        let result = verify_wasm_binary(path.to_str().unwrap(), wrong_hash);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("SHA-256 mismatch"), "unexpected error: {msg}");
    }

    #[test]
    fn test_sha256_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.wasm");
        fs::write(&path, b"").unwrap();

        // SHA-256 of empty input
        let expected = format!("{:x}", Sha256::digest(b""));
        let result = verify_wasm_binary(path.to_str().unwrap(), &expected);
        assert!(result.is_ok(), "expected Ok for empty file, got {:?}", result);
    }

    #[test]
    fn test_sha256_file_not_found() {
        let result = verify_wasm_binary("/nonexistent/path.wasm", "abc123");
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("cannot read"), "unexpected error: {msg}");
    }

    #[test]
    fn test_sha256_case_insensitive() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wasm");
        fs::write(&path, b"test data").unwrap();

        let expected = format!("{:x}", Sha256::digest(b"test data"));
        // Pass uppercase version
        let result = verify_wasm_binary(path.to_str().unwrap(), &expected.to_uppercase());
        assert!(result.is_ok(), "case-insensitive match should pass");
    }
}
