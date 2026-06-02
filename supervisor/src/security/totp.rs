// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P90: Two-factor authentication — TOTP generation, verification, and IPC commands.
//!
//! Simplified TOTP using SHA-256 HMAC with 30-second windows.
//! Uses boot-elapsed seconds since VyomaOS has no real-time clock.

use sha2::{Digest, Sha256};

use crate::user::{current_user, load_users, verify_pin};
use crate::{log_info, log_warn, send_reply, Inbox, BOOT_INSTANT};
use supervisor::logging::Subsystem;

const TOTP_DB_PATH: &str = "/data/totp_secrets.toml";
const BLOCK_SIZE: usize = 64; // SHA-256 block size

#[derive(Clone, Debug)]
struct TotpEntry {
    username: String,
    secret_hex: String,
}

/// Current time step (boot elapsed seconds / 30).
fn current_time_step() -> u64 {
    BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0) / 30
}

// ── HMAC-SHA256 (manual, no extra crate) ────────────────────────────────────

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let key_block = if key.len() > BLOCK_SIZE {
        let mut h = Sha256::new();
        h.update(key);
        let hashed = h.finalize();
        let mut block = [0u8; BLOCK_SIZE];
        block[..32].copy_from_slice(&hashed);
        block
    } else {
        let mut block = [0u8; BLOCK_SIZE];
        block[..key.len()].copy_from_slice(key);
        block
    };
    let mut i_key_pad = [0x36u8; BLOCK_SIZE];
    let mut o_key_pad = [0x5cu8; BLOCK_SIZE];
    for (i, b) in key_block.iter().enumerate() {
        i_key_pad[i] ^= b;
        o_key_pad[i] ^= b;
    }
    let mut inner = Sha256::new();
    inner.update(&i_key_pad);
    inner.update(message);
    let inner_hash = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&o_key_pad);
    outer.update(&inner_hash);
    let result = outer.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&result);
    out
}

// ── TOTP generation ─────────────────────────────────────────────────────────

/// Generate a 6-digit TOTP code from a secret and a time step.
/// Uses HMAC-SHA256 with dynamic truncation (RFC 4226 style).
pub fn generate_totp(secret: &[u8], time_step: u64) -> u32 {
    let mac = hmac_sha256(secret, &time_step.to_be_bytes());
    let offset = (mac[31] & 0x0F) as usize;
    let code = u32::from_be_bytes([
        mac[offset] & 0x7F, mac[offset + 1], mac[offset + 2], mac[offset + 3],
    ]);
    code % 1_000_000
}

// ── Secret management ───────────────────────────────────────────────────────

/// Generate a secret from username and boot time. Returns hex string (64 chars).
pub fn generate_secret(username: &str) -> String {
    let boot_ns = BOOT_INSTANT.get().map(|i| i.elapsed().as_nanos()).unwrap_or(0);
    let mut hasher = Sha256::new();
    hasher.update(username.as_bytes());
    hasher.update(b"vyomaos-totp-secret-salt");
    hasher.update(boot_ns.to_le_bytes());
    hasher.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn decode_hex_secret(hex: &str) -> Option<Vec<u8>> {
    if hex.len() % 2 != 0 { return None; }
    let mut bytes = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks(2) {
        let hi = hex_nibble(chunk[0])?;
        let lo = hex_nibble(chunk[1])?;
        bytes.push((hi << 4) | lo);
    }
    Some(bytes)
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

// ── TOTP database ───────────────────────────────────────────────────────────

fn load_totp_entries() -> Vec<TotpEntry> {
    match std::fs::read_to_string(TOTP_DB_PATH) {
        Ok(s) => parse_totp_toml(&s).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_totp_entries(entries: &[TotpEntry]) -> Result<(), String> {
    let toml = serialize_totp_toml(entries);
    std::fs::write(TOTP_DB_PATH, toml).map_err(|e| format!("write {TOTP_DB_PATH}: {e}"))
}

fn parse_totp_toml(raw: &str) -> Result<Vec<TotpEntry>, String> {
    let doc: toml::Value =
        toml::from_str(raw).map_err(|e| format!("totp_secrets.toml parse: {e}"))?;
    let arr = match doc.get("totp").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    let mut entries = Vec::with_capacity(arr.len());
    for item in arr {
        let username = item.get("username").and_then(|v| v.as_str())
            .ok_or("totp entry missing username")?.to_string();
        let secret_hex = item.get("secret").and_then(|v| v.as_str())
            .ok_or("totp entry missing secret")?.to_string();
        entries.push(TotpEntry { username, secret_hex });
    }
    Ok(entries)
}

fn serialize_totp_toml(entries: &[TotpEntry]) -> String {
    let mut out = String::new();
    for e in entries {
        out.push_str("[[totp]]\n");
        out.push_str(&format!("username = {:?}\n", e.username));
        out.push_str(&format!("secret = {:?}\n\n", e.secret_hex));
    }
    out
}

fn get_user_secret(username: &str) -> Option<String> {
    load_totp_entries().iter().find(|e| e.username == username).map(|e| e.secret_hex.clone())
}

/// Check whether a user has 2FA enabled.
pub fn is_2fa_enabled(username: &str) -> bool {
    get_user_secret(username).is_some()
}

// ── Enable / Disable ────────────────────────────────────────────────────────

/// Enable 2FA for a user. Returns the hex secret for the user to save.
pub fn enable_2fa(username: &str) -> Result<String, String> {
    if !load_users().iter().any(|u| u.username == username) {
        return Err(format!("unknown user '{username}'"));
    }
    let mut entries = load_totp_entries();
    if entries.iter().any(|e| e.username == username) {
        return Err("2FA already enabled".to_string());
    }
    let secret = generate_secret(username);
    entries.push(TotpEntry { username: username.to_string(), secret_hex: secret.clone() });
    save_totp_entries(&entries)?;
    Ok(secret)
}

/// Disable 2FA for a user.
pub fn disable_2fa(username: &str) -> Result<(), String> {
    let mut entries = load_totp_entries();
    let before = entries.len();
    entries.retain(|e| e.username != username);
    if entries.len() == before {
        return Err("2FA not enabled".to_string());
    }
    save_totp_entries(&entries)
}

// ── Verification ────────────────────────────────────────────────────────────

/// Verify a TOTP code for a user. Checks current, previous, and next windows.
pub fn verify_totp(username: &str, code: u32) -> bool {
    let secret_hex = match get_user_secret(username) {
        Some(s) => s,
        None => return false,
    };
    let secret = match decode_hex_secret(&secret_hex) {
        Some(s) => s,
        None => return false,
    };
    verify_totp_at_step(&secret, code, current_time_step())
}

/// Check code against current, previous, and next time windows.
fn verify_totp_at_step(secret: &[u8], code: u32, step: u64) -> bool {
    if generate_totp(secret, step) == code { return true; }
    if step > 0 && generate_totp(secret, step - 1) == code { return true; }
    generate_totp(secret, step + 1) == code
}

// ── Login flow integration ──────────────────────────────────────────────────

/// Extended login: after PIN check, require TOTP if 2FA is enabled.
pub fn login_with_2fa(username: &str, pin: &str, totp_code: Option<u32>) -> Result<(), String> {
    let users = load_users();
    let user = users.iter().find(|u| u.username == username)
        .ok_or_else(|| format!("unknown user '{username}'"))?;
    if !verify_pin(pin, &user.pin_hash) {
        return Err("incorrect PIN".to_string());
    }
    if is_2fa_enabled(username) {
        match totp_code {
            None => return Err("2FA code required".to_string()),
            Some(code) => {
                if !verify_totp(username, code) {
                    return Err("invalid 2FA code".to_string());
                }
            }
        }
    }
    crate::user::login(username, pin)?;
    Ok(())
}

// ── IPC command handler ─────────────────────────────────────────────────────

/// Handle `@supervisor: 2fa-*` IPC commands. Returns `true` if recognised.
pub fn handle_2fa_command(verb: &str, parts: &[&str], sender: &str, inbox: &Inbox) -> bool {
    match verb {
        "2fa-enable" => {
            let username = match current_user() {
                Some(u) => u,
                None => { send_reply(sender, "REPLY:error: not logged in", inbox); return true; }
            };
            match enable_2fa(&username) {
                Ok(secret) => {
                    log_info!(Subsystem::Lifecycle, None, "2FA enabled for {username}");
                    send_reply(sender, &format!("REPLY:2FA enabled. Secret: {secret}"), inbox);
                }
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }
        "2fa-disable" => {
            let username = match current_user() {
                Some(u) => u,
                None => { send_reply(sender, "REPLY:error: not logged in", inbox); return true; }
            };
            let pin = parts.get(1).unwrap_or(&"").trim();
            if pin.is_empty() {
                send_reply(sender, "REPLY:error: usage: 2fa-disable <pin>", inbox);
                return true;
            }
            let users = load_users();
            let user = match users.iter().find(|u| u.username == username) {
                Some(u) => u,
                None => { send_reply(sender, "REPLY:error: user not found", inbox); return true; }
            };
            if !verify_pin(pin, &user.pin_hash) {
                log_warn!(Subsystem::Lifecycle, None, "2FA disable failed for {username}: bad PIN");
                send_reply(sender, "REPLY:error: incorrect PIN", inbox);
                return true;
            }
            match disable_2fa(&username) {
                Ok(()) => {
                    log_info!(Subsystem::Lifecycle, None, "2FA disabled for {username}");
                    send_reply(sender, "REPLY:2FA disabled", inbox);
                }
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }
        "2fa-status" => {
            let username = match current_user() {
                Some(u) => u,
                None => { send_reply(sender, "REPLY:error: not logged in", inbox); return true; }
            };
            let st = if is_2fa_enabled(&username) { "active" } else { "inactive" };
            send_reply(sender, &format!("REPLY:2FA {st}"), inbox);
        }
        _ => return false,
    }
    true
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn time_step_from_secs(secs: u64) -> u64 { secs / 30 }

    #[test]
    fn test_generate_totp_deterministic_and_bounded() {
        let secret = b"test-secret-key-1234";
        let c1 = generate_totp(secret, 100);
        let c2 = generate_totp(secret, 100);
        assert_eq!(c1, c2);
        assert!(c1 < 1_000_000);
    }

    #[test]
    fn test_generate_totp_different_inputs() {
        // Different secrets at same step produce different codes
        assert_ne!(generate_totp(b"secret-alpha", 50), generate_totp(b"secret-beta", 50));
        // Different steps (extremely unlikely to collide but we just exercise the path)
        let _ = (generate_totp(b"k", 100), generate_totp(b"k", 101));
    }

    #[test]
    fn test_verify_totp_current_and_adjacent_windows() {
        let secret = b"verify-test-secret";
        let step = 200;
        // Current window
        assert!(verify_totp_at_step(secret, generate_totp(secret, step), step));
        // Previous window (clock skew tolerance)
        assert!(verify_totp_at_step(secret, generate_totp(secret, step - 1), step));
        // Next window (clock skew tolerance)
        assert!(verify_totp_at_step(secret, generate_totp(secret, step + 1), step));
    }

    #[test]
    fn test_verify_totp_wrong_code() {
        let secret = b"verify-test-secret";
        let step = 200;
        let correct = generate_totp(secret, step);
        let wrong = (correct + 1) % 1_000_000;
        let adj_prev = generate_totp(secret, step - 1);
        let adj_next = generate_totp(secret, step + 1);
        if wrong != adj_prev && wrong != adj_next {
            assert!(!verify_totp_at_step(secret, wrong, step));
        }
    }

    #[test]
    fn test_hmac_sha256_known_vector() {
        // RFC 4231 Test Case 2
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn test_decode_hex_secret() {
        assert_eq!(decode_hex_secret("48656c6c6f").unwrap(), b"Hello");
        assert!(decode_hex_secret("zz").is_none());
        assert!(decode_hex_secret("abc").is_none()); // odd length
    }

    #[test]
    fn test_totp_db_roundtrip() {
        let entries = vec![
            TotpEntry { username: "alice".into(), secret_hex: "deadbeef".into() },
            TotpEntry { username: "bob".into(), secret_hex: "cafebabe".into() },
        ];
        let toml = serialize_totp_toml(&entries);
        let parsed = parse_totp_toml(&toml).expect("parse roundtrip");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].username, "alice");
        assert_eq!(parsed[0].secret_hex, "deadbeef");
        assert_eq!(parsed[1].username, "bob");
        assert_eq!(parsed[1].secret_hex, "cafebabe");
        // Empty DB
        assert!(parse_totp_toml("").unwrap().is_empty());
    }

    #[test]
    fn test_time_step_from_secs() {
        assert_eq!(time_step_from_secs(0), 0);
        assert_eq!(time_step_from_secs(29), 0);
        assert_eq!(time_step_from_secs(30), 1);
        assert_eq!(time_step_from_secs(59), 1);
        assert_eq!(time_step_from_secs(60), 2);
    }
}
