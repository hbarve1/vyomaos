// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P87: Encrypted storage — OS infrastructure.
//!
//! Provides XOR-based placeholder encryption (proof-of-concept), SHA-256 key
//! derivation, and encrypted file read/write with integrity verification.
//!
//! **Security note**: XOR cipher is NOT cryptographically secure. A production
//! implementation would use AES-256-GCM (e.g. via the `aes-gcm` crate) with a
//! proper KDF such as Argon2 or PBKDF2. This module is a structural placeholder
//! that proves out the API surface, file format, and IPC integration.

use std::fs;

use sha2::{Digest, Sha256};

use crate::{log_info, send_reply, Inbox};
use supervisor::logging::Subsystem;

/// Magic header written at the start of every encrypted file.
const MAGIC: &[u8] = b"VYOMA_ENC_V1\n";

/// Length of the SHA-256 checksum stored after the magic header (32 bytes).
const CHECKSUM_LEN: usize = 32;

// ── XOR cipher (placeholder) ────────────────────────────────────────────────

/// Encrypt `data` by XOR-ing each byte with a repeating `key`.
///
/// **Not secure** — placeholder for AES-256-GCM. Symmetric, so the same
/// function serves as both encrypt and decrypt.
pub fn encrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    assert!(!key.is_empty(), "encryption key must not be empty");
    data.iter()
        .enumerate()
        .map(|(i, &b)| b ^ key[i % key.len()])
        .collect()
}

/// Decrypt `data` by XOR-ing each byte with a repeating `key`.
///
/// Identical to [`encrypt`] because XOR is its own inverse.
pub fn decrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    encrypt(data, key)
}

// ── Key derivation ──────────────────────────────────────────────────────────

/// Derive a 32-byte key from `password` using SHA-256.
///
/// In a production system this would use Argon2id or PBKDF2 with a random
/// salt and high iteration count.
pub fn derive_key(password: &str) -> Vec<u8> {
    Sha256::digest(password.as_bytes()).to_vec()
}

// ── Encrypted file operations ───────────────────────────────────────────────

/// Compute the on-disk path for an encrypted file.
///
/// If `path` already ends with `.enc`, it is returned as-is; otherwise `.enc`
/// is appended.
fn enc_path(path: &str) -> String {
    if path.ends_with(".enc") {
        path.to_string()
    } else {
        format!("{path}.enc")
    }
}

/// Write `data` encrypted with `password` to `<path>.enc`.
///
/// File format:
/// ```text
/// VYOMA_ENC_V1\n          (13 bytes magic)
/// <sha256 of plaintext>   (32 bytes raw)
/// <xor-encrypted data>    (variable)
/// ```
pub fn write_encrypted(path: &str, data: &[u8], password: &str) -> Result<(), String> {
    let key = derive_key(password);
    let checksum: Vec<u8> = Sha256::digest(data).to_vec();
    let cipher = encrypt(data, &key);

    let mut blob = Vec::with_capacity(MAGIC.len() + CHECKSUM_LEN + cipher.len());
    blob.extend_from_slice(MAGIC);
    blob.extend_from_slice(&checksum);
    blob.extend_from_slice(&cipher);

    let out = enc_path(path);
    // Ensure parent directory exists.
    if let Some(parent) = std::path::Path::new(&out).parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
    }
    fs::write(&out, &blob).map_err(|e| format!("write {out}: {e}"))
}

/// Read and decrypt `<path>.enc`, verifying the integrity checksum.
///
/// Returns the original plaintext on success, or an error if the file is
/// malformed, the password is wrong (checksum mismatch), or an I/O error
/// occurs.
pub fn read_encrypted(path: &str, password: &str) -> Result<Vec<u8>, String> {
    let src = enc_path(path);
    let blob = fs::read(&src).map_err(|e| format!("read {src}: {e}"))?;

    let min_len = MAGIC.len() + CHECKSUM_LEN;
    if blob.len() < min_len {
        return Err("file too short to be a valid encrypted file".to_string());
    }

    // Verify magic header.
    if &blob[..MAGIC.len()] != MAGIC {
        return Err("missing VYOMA_ENC_V1 magic header".to_string());
    }

    let stored_checksum = &blob[MAGIC.len()..MAGIC.len() + CHECKSUM_LEN];
    let cipher = &blob[min_len..];

    let key = derive_key(password);
    let plaintext = decrypt(cipher, &key);

    // Integrity check: SHA-256 of decrypted data must match stored checksum.
    let actual_checksum: Vec<u8> = Sha256::digest(&plaintext).to_vec();
    if actual_checksum != stored_checksum {
        return Err("integrity check failed — wrong password or corrupted file".to_string());
    }

    Ok(plaintext)
}

// ── IPC command handler ──────────────────────────────────────────────────────

/// Handle `@supervisor: encrypt|decrypt|encrypted-write` commands.
///
/// Returns `true` if the verb was recognised, `false` otherwise.
pub fn handle_encrypted_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        // @supervisor: encrypt <path> <password>
        "encrypt" => {
            let (path, password) = match parse_two_args(parts) {
                Some(v) => v,
                None => {
                    send_reply(
                        sender,
                        "REPLY:encrypt error: usage: encrypt <path> <password>",
                        inbox,
                    );
                    return true;
                }
            };
            match encrypt_file_in_place(path, password) {
                Ok(()) => {
                    log_info!(Subsystem::Ipc, Some(sender), "encrypt {path}: ok");
                    send_reply(sender, &format!("REPLY:encrypt {path} ok"), inbox);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:encrypt error: {e}"), inbox);
                }
            }
        }

        // @supervisor: decrypt <path> <password>
        "decrypt" => {
            let (path, password) = match parse_two_args(parts) {
                Some(v) => v,
                None => {
                    send_reply(
                        sender,
                        "REPLY:decrypt error: usage: decrypt <path> <password>",
                        inbox,
                    );
                    return true;
                }
            };
            match decrypt_file_in_place(path, password) {
                Ok(()) => {
                    log_info!(Subsystem::Ipc, Some(sender), "decrypt {path}: ok");
                    send_reply(sender, &format!("REPLY:decrypt {path} ok"), inbox);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:decrypt error: {e}"), inbox);
                }
            }
        }

        // @supervisor: encrypted-write <path> <password> <data>
        "encrypted-write" => {
            let (path, password, data) = match parse_three_args(parts) {
                Some(v) => v,
                None => {
                    send_reply(
                        sender,
                        "REPLY:encrypted-write error: usage: encrypted-write <path> <password> <data>",
                        inbox,
                    );
                    return true;
                }
            };
            match write_encrypted(path, data.as_bytes(), password) {
                Ok(()) => {
                    log_info!(Subsystem::Ipc, Some(sender), "encrypted-write {path}: ok");
                    send_reply(
                        sender,
                        &format!("REPLY:encrypted-write {path} ok"),
                        inbox,
                    );
                }
                Err(e) => {
                    send_reply(
                        sender,
                        &format!("REPLY:encrypted-write error: {e}"),
                        inbox,
                    );
                }
            }
        }

        _ => return false,
    }
    true
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Parse `parts` as `[verb, arg1, arg2]` (verb already consumed by caller).
fn parse_two_args<'a>(parts: &[&'a str]) -> Option<(&'a str, &'a str)> {
    // parts[0] = verb, rest is the argument string.
    let rest = parts.get(1)?;
    let mut iter = rest.trim().splitn(2, ' ');
    let a = iter.next()?.trim();
    let b = iter.next()?.trim();
    if a.is_empty() || b.is_empty() {
        return None;
    }
    Some((a, b))
}

/// Parse `parts` as `[verb, arg1, arg2, arg3...]` (verb already consumed).
fn parse_three_args<'a>(parts: &[&'a str]) -> Option<(&'a str, &'a str, &'a str)> {
    let rest = parts.get(1)?;
    let mut iter = rest.trim().splitn(3, ' ');
    let a = iter.next()?.trim();
    let b = iter.next()?.trim();
    let c = iter.next()?.trim();
    if a.is_empty() || b.is_empty() || c.is_empty() {
        return None;
    }
    Some((a, b, c))
}

/// Encrypt a plaintext file in-place: read it, write `<path>.enc`, remove
/// the original.
fn encrypt_file_in_place(path: &str, password: &str) -> Result<(), String> {
    let data = fs::read(path).map_err(|e| format!("read {path}: {e}"))?;
    write_encrypted(path, &data, password)?;
    fs::remove_file(path).map_err(|e| format!("remove {path}: {e}"))?;
    Ok(())
}

/// Decrypt an encrypted file in-place: read `<path>.enc`, write plaintext
/// back to `<path>`, remove the `.enc` file.
fn decrypt_file_in_place(path: &str, password: &str) -> Result<(), String> {
    let plaintext = read_encrypted(path, password)?;
    fs::write(path, &plaintext).map_err(|e| format!("write {path}: {e}"))?;
    let enc = enc_path(path);
    if enc != path {
        fs::remove_file(&enc).map_err(|e| format!("remove {enc}: {e}"))?;
    }
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_roundtrip() {
        let key = b"secret";
        let data = b"Hello, VyomaOS!";
        let cipher = encrypt(data, key);
        assert_ne!(&cipher, data);
        let plain = decrypt(&cipher, key);
        assert_eq!(&plain, data);
    }

    #[test]
    fn xor_empty_data() {
        let key = b"key";
        let cipher = encrypt(b"", key);
        assert!(cipher.is_empty());
        let plain = decrypt(&cipher, key);
        assert!(plain.is_empty());
    }

    #[test]
    fn key_derivation_deterministic() {
        let k1 = derive_key("password123");
        let k2 = derive_key("password123");
        assert_eq!(k1, k2);
        assert_eq!(k1.len(), 32);
    }

    #[test]
    fn key_derivation_different_passwords() {
        let k1 = derive_key("alpha");
        let k2 = derive_key("beta");
        assert_ne!(k1, k2);
    }

    #[test]
    fn file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret.txt");
        let path_str = path.to_str().unwrap();

        let data = b"top secret data for VyomaOS";
        write_encrypted(path_str, data, "mypass").unwrap();

        // Encrypted file should exist with .enc suffix.
        let enc = format!("{path_str}.enc");
        assert!(std::path::Path::new(&enc).exists());

        // Read back and verify.
        let plain = read_encrypted(path_str, "mypass").unwrap();
        assert_eq!(plain, data);
    }

    #[test]
    fn wrong_password_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secret2.txt");
        let path_str = path.to_str().unwrap();

        write_encrypted(path_str, b"data", "correct").unwrap();
        let result = read_encrypted(path_str, "wrong");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("integrity check failed"));
    }

    #[test]
    fn integrity_check_corrupted_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrupt.txt");
        let path_str = path.to_str().unwrap();

        write_encrypted(path_str, b"original", "pass").unwrap();

        // Corrupt one byte in the ciphertext region.
        let enc = enc_path(path_str);
        let mut blob = fs::read(&enc).unwrap();
        let last = blob.len() - 1;
        blob[last] ^= 0xFF;
        fs::write(&enc, &blob).unwrap();

        let result = read_encrypted(path_str, "pass");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("integrity check failed"));
    }

    #[test]
    fn magic_header_present() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("magic.txt");
        let path_str = path.to_str().unwrap();

        write_encrypted(path_str, b"test", "pw").unwrap();

        let enc = enc_path(path_str);
        let blob = fs::read(&enc).unwrap();
        assert!(blob.starts_with(MAGIC));
    }

    #[test]
    fn file_too_short_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.txt.enc");
        let path_str = path.to_str().unwrap();

        fs::write(path_str, b"VYOMA_ENC_V1\n").unwrap();
        let result = read_encrypted(
            path_str.strip_suffix(".enc").unwrap(),
            "pw",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too short"));
    }

    #[test]
    fn bad_magic_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("badmagic.txt.enc");
        let path_str = path.to_str().unwrap();

        let mut blob = vec![0u8; 100];
        blob[..5].copy_from_slice(b"WRONG");
        fs::write(path_str, &blob).unwrap();

        let result = read_encrypted(
            path_str.strip_suffix(".enc").unwrap(),
            "pw",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("magic header"));
    }

    #[test]
    fn encrypt_in_place_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("inplace.txt");
        let path_str = path.to_str().unwrap();

        fs::write(path_str, b"plaintext content").unwrap();
        encrypt_file_in_place(path_str, "pw").unwrap();

        // Original should be gone, .enc should exist.
        assert!(!path.exists());
        assert!(dir.path().join("inplace.txt.enc").exists());

        decrypt_file_in_place(path_str, "pw").unwrap();

        // Plaintext restored, .enc removed.
        assert!(path.exists());
        assert!(!dir.path().join("inplace.txt.enc").exists());
        assert_eq!(fs::read(path_str).unwrap(), b"plaintext content");
    }

    #[test]
    fn enc_path_idempotent() {
        assert_eq!(enc_path("/data/foo.txt"), "/data/foo.txt.enc");
        assert_eq!(enc_path("/data/foo.txt.enc"), "/data/foo.txt.enc");
    }

    #[test]
    fn parse_two_args_valid() {
        let parts = ["encrypt", "/data/f.txt secret"];
        let (a, b) = parse_two_args(&parts).unwrap();
        assert_eq!(a, "/data/f.txt");
        assert_eq!(b, "secret");
    }

    #[test]
    fn parse_two_args_missing() {
        let parts = ["encrypt", "/data/f.txt"];
        assert!(parse_two_args(&parts).is_none());
    }

    #[test]
    fn parse_three_args_valid() {
        let parts = ["encrypted-write", "/data/f.txt pw hello world"];
        let (a, b, c) = parse_three_args(&parts).unwrap();
        assert_eq!(a, "/data/f.txt");
        assert_eq!(b, "pw");
        assert_eq!(c, "hello world");
    }
}
