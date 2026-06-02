// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P85: User accounts system — user model, database, session, and IPC commands.

use std::sync::{Mutex, OnceLock};
use crate::lock_or_recover;

use sha2::{Digest, Sha256};

use crate::{log_info, log_warn, send_reply, Inbox};
use supervisor::logging::Subsystem;

// ── User model ───────────────────────────────────────────────────────────────

/// User role — determines administrative privileges.
#[derive(Clone, Debug, PartialEq)]
pub enum UserRole {
    Admin,
    Standard,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Admin => "admin",
            UserRole::Standard => "standard",
        }
    }

    pub fn from_str(s: &str) -> Option<UserRole> {
        match s.to_lowercase().as_str() {
            "admin" => Some(UserRole::Admin),
            "standard" => Some(UserRole::Standard),
            _ => None,
        }
    }
}

/// A user account entry.
#[derive(Clone, Debug)]
pub struct User {
    pub username: String,
    pub display_name: String,
    pub pin_hash: String,
    pub role: UserRole,
    pub home_dir: String,
}

// ── PIN hashing ──────────────────────────────────────────────────────────────

/// Compute SHA-256 hex digest of a PIN string.
pub fn hash_pin(pin: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pin.as_bytes());
    let result = hasher.finalize();
    result.iter().map(|b| format!("{b:02x}")).collect()
}

/// Verify a plaintext PIN against a stored hash.
pub fn verify_pin(pin: &str, expected_hash: &str) -> bool {
    hash_pin(pin) == expected_hash
}

// ── User database (TOML at /data/users.toml) ────────────────────────────────

const USERS_DB_PATH: &str = "/data/users.toml";

/// Load all users from the on-disk database. Returns default root user if file
/// is missing or unparseable.
pub fn load_users() -> Vec<User> {
    let raw = match std::fs::read_to_string(USERS_DB_PATH) {
        Ok(s) => s,
        Err(_) => return default_users(),
    };
    parse_users_toml(&raw).unwrap_or_else(|_| default_users())
}

/// Save the user list to the on-disk database.
pub fn save_users(users: &[User]) -> Result<(), String> {
    let toml = serialize_users_toml(users);
    std::fs::write(USERS_DB_PATH, toml).map_err(|e| format!("write {USERS_DB_PATH}: {e}"))
}

/// Default user set: single root admin with PIN "0000".
fn default_users() -> Vec<User> {
    vec![User {
        username: "root".to_string(),
        display_name: "Root".to_string(),
        pin_hash: hash_pin("0000"),
        role: UserRole::Admin,
        home_dir: "/data/users/root".to_string(),
    }]
}

/// Parse `[[user]]` entries from TOML text.
pub fn parse_users_toml(raw: &str) -> Result<Vec<User>, String> {
    let doc: toml::Value =
        toml::from_str(raw).map_err(|e| format!("users.toml parse error: {e}"))?;
    let arr = doc
        .get("user")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "missing [[user]] array".to_string())?;
    let mut users = Vec::with_capacity(arr.len());
    for entry in arr {
        let username = entry
            .get("username")
            .and_then(|v| v.as_str())
            .ok_or("user missing username")?
            .to_string();
        let display_name = entry
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or(&username)
            .to_string();
        let pin_hash = entry
            .get("pin_hash")
            .and_then(|v| v.as_str())
            .ok_or("user missing pin_hash")?
            .to_string();
        let role_str = entry
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("standard");
        let role =
            UserRole::from_str(role_str).ok_or_else(|| format!("bad role: {role_str}"))?;
        let home_dir = entry
            .get("home_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(&format!("/data/users/{username}"))
            .to_string();
        users.push(User {
            username,
            display_name,
            pin_hash,
            role,
            home_dir,
        });
    }
    Ok(users)
}

/// Serialize users back to TOML `[[user]]` format.
pub fn serialize_users_toml(users: &[User]) -> String {
    let mut out = String::new();
    for u in users {
        out.push_str("[[user]]\n");
        out.push_str(&format!("username = {:?}\n", u.username));
        out.push_str(&format!("display_name = {:?}\n", u.display_name));
        out.push_str(&format!("pin_hash = {:?}\n", u.pin_hash));
        out.push_str(&format!("role = {:?}\n", u.role.as_str()));
        out.push_str(&format!("home_dir = {:?}\n", u.home_dir));
        out.push('\n');
    }
    out
}

// ── Active session ───────────────────────────────────────────────────────────

static CURRENT_USER: OnceLock<Mutex<Option<String>>> = OnceLock::new();

fn session() -> &'static Mutex<Option<String>> {
    CURRENT_USER.get_or_init(|| Mutex::new(None))
}

/// Log in as `username` after verifying `pin`. Returns error string on failure.
pub fn login(username: &str, pin: &str) -> Result<(), String> {
    let users = load_users();
    let user = users
        .iter()
        .find(|u| u.username == username)
        .ok_or_else(|| format!("unknown user '{username}'"))?;
    if !verify_pin(pin, &user.pin_hash) {
        return Err("incorrect PIN".to_string());
    }
    *lock_or_recover(&session()) = Some(username.to_string());
    Ok(())
}

/// Log out the current user.
pub fn logout() {
    *lock_or_recover(&session()) = None;
}

/// Return the currently logged-in username, if any.
pub fn current_user() -> Option<String> {
    lock_or_recover(&session()).clone()
}

/// Check whether the current user has the Admin role.
pub fn is_admin() -> bool {
    let username = match current_user() {
        Some(u) => u,
        None => return false,
    };
    let users = load_users();
    users
        .iter()
        .any(|u| u.username == username && u.role == UserRole::Admin)
}

// ── Per-user home directory ──────────────────────────────────────────────────

/// Create the home directory for a user (idempotent).
fn ensure_home_dir(home: &str) {
    let _ = std::fs::create_dir_all(home);
}

// ── IPC command handler ──────────────────────────────────────────────────────

/// Handle `@supervisor: user-*` IPC commands. Returns `true` if the verb was
/// recognised (even on error), `false` if unrecognised.
pub fn handle_user_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "user-list" => {
            let users = load_users();
            let rows: Vec<String> = users
                .iter()
                .map(|u| format!("{} ({}) [{}]", u.username, u.display_name, u.role.as_str()))
                .collect();
            send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox);
        }

        "user-add" => {
            if !is_admin() {
                send_reply(sender, "REPLY:error: admin required", inbox);
                return true;
            }
            // usage: user-add <username> <pin> <role>
            let rest = parts.get(1).unwrap_or(&"").trim();
            let tokens: Vec<&str> = rest.splitn(3, ' ').collect();
            if tokens.len() < 3 || tokens[0].is_empty() {
                send_reply(
                    sender,
                    "REPLY:error: usage: user-add <username> <pin> <role>",
                    inbox,
                );
                return true;
            }
            let (name, pin, role_str) = (tokens[0], tokens[1], tokens[2]);
            let role = match UserRole::from_str(role_str) {
                Some(r) => r,
                None => {
                    send_reply(
                        sender,
                        "REPLY:error: role must be admin or standard",
                        inbox,
                    );
                    return true;
                }
            };
            let mut users = load_users();
            if users.iter().any(|u| u.username == name) {
                send_reply(
                    sender,
                    &format!("REPLY:error: user '{name}' already exists"),
                    inbox,
                );
                return true;
            }
            let home = format!("/data/users/{name}");
            users.push(User {
                username: name.to_string(),
                display_name: name.to_string(),
                pin_hash: hash_pin(pin),
                role,
                home_dir: home.clone(),
            });
            if let Err(e) = save_users(&users) {
                send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                return true;
            }
            ensure_home_dir(&home);
            log_info!(Subsystem::Lifecycle, None, "user added: {name}");
            send_reply(sender, &format!("REPLY:user {name} added"), inbox);
        }

        "user-remove" => {
            if !is_admin() {
                send_reply(sender, "REPLY:error: admin required", inbox);
                return true;
            }
            let name = parts.get(1).unwrap_or(&"").trim();
            if name.is_empty() {
                send_reply(
                    sender,
                    "REPLY:error: usage: user-remove <username>",
                    inbox,
                );
                return true;
            }
            if name == "root" {
                send_reply(sender, "REPLY:error: cannot remove root", inbox);
                return true;
            }
            let mut users = load_users();
            let before = users.len();
            users.retain(|u| u.username != name);
            if users.len() == before {
                send_reply(
                    sender,
                    &format!("REPLY:error: user '{name}' not found"),
                    inbox,
                );
                return true;
            }
            if let Err(e) = save_users(&users) {
                send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                return true;
            }
            // If removed user is logged in, log them out.
            if current_user().as_deref() == Some(name) {
                logout();
            }
            log_info!(Subsystem::Lifecycle, None, "user removed: {name}");
            send_reply(sender, &format!("REPLY:user {name} removed"), inbox);
        }

        "user-login" => {
            // usage: user-login <username> <pin> [totp_code]
            let rest = parts.get(1).unwrap_or(&"").trim();
            let tokens: Vec<&str> = rest.splitn(3, ' ').collect();
            if tokens.len() < 2 || tokens[0].is_empty() {
                send_reply(
                    sender,
                    "REPLY:error: usage: user-login <username> <pin> [totp_code]",
                    inbox,
                );
                return true;
            }
            let name = tokens[0].trim();
            let pin = tokens[1].trim();
            let totp_code = tokens.get(2).and_then(|s| s.trim().parse::<u32>().ok());
            match crate::totp::login_with_2fa(name, pin, totp_code) {
                Ok(()) => {
                    log_info!(Subsystem::Lifecycle, None, "user logged in: {name}");
                    send_reply(
                        sender,
                        &format!("REPLY:logged in as {name}"),
                        inbox,
                    );
                }
                Err(e) => {
                    log_warn!(Subsystem::Lifecycle, None, "login failed for {name}: {e}");
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
        }

        "user-logout" => {
            match current_user() {
                Some(name) => {
                    logout();
                    log_info!(Subsystem::Lifecycle, None, "user logged out: {name}");
                    send_reply(sender, "REPLY:logged out", inbox);
                }
                None => {
                    send_reply(sender, "REPLY:error: not logged in", inbox);
                }
            }
        }

        "user-whoami" => {
            let reply = match current_user() {
                Some(name) => {
                    let admin = is_admin();
                    let role = if admin { "admin" } else { "standard" };
                    format!("REPLY:{name} [{role}]")
                }
                None => "REPLY:not logged in".to_string(),
            };
            send_reply(sender, &reply, inbox);
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
    fn test_hash_pin_deterministic() {
        let h1 = hash_pin("0000");
        let h2 = hash_pin("0000");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64); // SHA-256 hex is 64 chars
    }

    #[test]
    fn test_hash_pin_different_inputs() {
        assert_ne!(hash_pin("0000"), hash_pin("1234"));
    }

    #[test]
    fn test_verify_pin_correct() {
        let h = hash_pin("5678");
        assert!(verify_pin("5678", &h));
    }

    #[test]
    fn test_verify_pin_incorrect() {
        let h = hash_pin("5678");
        assert!(!verify_pin("9999", &h));
    }

    #[test]
    fn test_role_roundtrip() {
        assert_eq!(UserRole::from_str("admin"), Some(UserRole::Admin));
        assert_eq!(UserRole::from_str("standard"), Some(UserRole::Standard));
        assert_eq!(UserRole::from_str("ADMIN"), Some(UserRole::Admin));
        assert_eq!(UserRole::from_str("bogus"), None);
        assert_eq!(UserRole::Admin.as_str(), "admin");
        assert_eq!(UserRole::Standard.as_str(), "standard");
    }

    #[test]
    fn test_default_users() {
        let users = default_users();
        assert_eq!(users.len(), 1);
        assert_eq!(users[0].username, "root");
        assert_eq!(users[0].role, UserRole::Admin);
        assert!(verify_pin("0000", &users[0].pin_hash));
    }

    #[test]
    fn test_serialize_parse_roundtrip() {
        let users = vec![
            User {
                username: "alice".to_string(),
                display_name: "Alice A".to_string(),
                pin_hash: hash_pin("1111"),
                role: UserRole::Admin,
                home_dir: "/data/users/alice".to_string(),
            },
            User {
                username: "bob".to_string(),
                display_name: "Bob B".to_string(),
                pin_hash: hash_pin("2222"),
                role: UserRole::Standard,
                home_dir: "/data/users/bob".to_string(),
            },
        ];
        let toml = serialize_users_toml(&users);
        let parsed = parse_users_toml(&toml).expect("parse roundtrip");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].username, "alice");
        assert_eq!(parsed[0].display_name, "Alice A");
        assert_eq!(parsed[0].role, UserRole::Admin);
        assert!(verify_pin("1111", &parsed[0].pin_hash));
        assert_eq!(parsed[1].username, "bob");
        assert_eq!(parsed[1].role, UserRole::Standard);
    }

    #[test]
    fn test_parse_bad_toml() {
        assert!(parse_users_toml("not valid toml {{{").is_err());
    }

    #[test]
    fn test_parse_missing_user_array() {
        assert!(parse_users_toml("[other]\nkey = 1\n").is_err());
    }

    #[test]
    fn test_session_login_logout() {
        // Reset session state for test isolation.
        *session().lock().unwrap() = None;

        // No user logged in initially.
        assert!(current_user().is_none());
        assert!(!is_admin());

        // Direct session manipulation (bypasses disk-based load_users).
        *session().lock().unwrap() = Some("testuser".to_string());
        assert_eq!(current_user(), Some("testuser".to_string()));

        logout();
        assert!(current_user().is_none());
    }
}
