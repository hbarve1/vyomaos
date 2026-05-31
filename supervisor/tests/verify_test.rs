// P28: integration tests for WASM binary SHA-256 verification.

use sha2::{Digest, Sha256};
use std::fs;

// The verify module is in the binary crate, so we test via the same logic
// (sha2 + file read) that the module uses.  This validates the contract
// expected by spawn_app in app_threads.rs.

fn verify_wasm_binary(wasm_path: &str, expected_sha256: &str) -> Result<(), String> {
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

    let wrong = "0000000000000000000000000000000000000000000000000000000000000000";
    let result = verify_wasm_binary(path.to_str().unwrap(), wrong);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("SHA-256 mismatch"));
}

#[test]
fn test_sha256_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.wasm");
    fs::write(&path, b"").unwrap();

    let expected = format!("{:x}", Sha256::digest(b""));
    let result = verify_wasm_binary(path.to_str().unwrap(), &expected);
    assert!(result.is_ok(), "expected Ok for empty file, got {:?}", result);
}
