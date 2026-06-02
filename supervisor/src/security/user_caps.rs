// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P86: Per-user capability tokens — enforce per-user limits on apps, capabilities,
//! network connections, and disk usage.

use std::sync::{Mutex, OnceLock};
use crate::lock_or_recover;

use crate::user::{current_user, is_admin, load_users, UserRole};
use crate::{log_info, log_warn, send_reply, Inbox};
use supervisor::logging::Subsystem;

// ── Per-user capability profile ─────────────────────────────────────────────

/// Defines the capability limits and permissions for a single user.
#[derive(Clone, Debug)]
pub struct UserCapProfile {
    pub username: String,
    pub max_apps: usize,           // 0 = unlimited
    pub allowed_caps: Vec<String>, // empty = all allowed
    pub denied_caps: Vec<String>,  // explicitly denied capabilities
    pub max_network_conns: usize,  // 0 = unlimited
    pub disk_quota_kb: u64,        // 0 = unlimited
}

// ── Default profiles ────────────────────────────────────────────────────────

pub fn default_admin_profile(username: &str) -> UserCapProfile {
    UserCapProfile {
        username: username.to_string(),
        max_apps: 0,
        allowed_caps: Vec::new(),
        denied_caps: Vec::new(),
        max_network_conns: 0,
        disk_quota_kb: 0,
    }
}

pub fn default_standard_profile(username: &str) -> UserCapProfile {
    UserCapProfile {
        username: username.to_string(),
        max_apps: 10,
        allowed_caps: Vec::new(),
        denied_caps: vec!["shell".to_string()],
        max_network_conns: 20,
        disk_quota_kb: 10_240, // 10 MB
    }
}

fn default_profile_for(username: &str) -> UserCapProfile {
    let users = load_users();
    let role = users
        .iter()
        .find(|u| u.username == username)
        .map(|u| u.role.clone())
        .unwrap_or(UserRole::Standard);
    match role {
        UserRole::Admin => default_admin_profile(username),
        UserRole::Standard => default_standard_profile(username),
    }
}

// ── Persistence: /data/user-caps.toml ───────────────────────────────────────

const CAPS_DB_PATH: &str = "/data/user-caps.toml";

static PROFILES: OnceLock<Mutex<Vec<UserCapProfile>>> = OnceLock::new();

fn profiles() -> &'static Mutex<Vec<UserCapProfile>> {
    PROFILES.get_or_init(|| Mutex::new(load_profiles()))
}

fn load_profiles() -> Vec<UserCapProfile> {
    let raw = match std::fs::read_to_string(CAPS_DB_PATH) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    parse_profiles_toml(&raw).unwrap_or_default()
}

pub fn parse_profiles_toml(raw: &str) -> Result<Vec<UserCapProfile>, String> {
    let doc: toml::Value =
        toml::from_str(raw).map_err(|e| format!("user-caps.toml parse error: {e}"))?;
    let arr = match doc.get("profile").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::with_capacity(arr.len());
    for entry in arr {
        let username = entry
            .get("username")
            .and_then(|v| v.as_str())
            .ok_or("profile missing username")?
            .to_string();
        let max_apps = entry.get("max_apps").and_then(|v| v.as_integer()).unwrap_or(0) as usize;
        let allowed_caps = parse_string_array(entry, "allowed_caps");
        let denied_caps = parse_string_array(entry, "denied_caps");
        let max_net = entry.get("max_network_conns").and_then(|v| v.as_integer()).unwrap_or(0);
        let quota = entry.get("disk_quota_kb").and_then(|v| v.as_integer()).unwrap_or(0);
        out.push(UserCapProfile {
            username,
            max_apps,
            allowed_caps,
            denied_caps,
            max_network_conns: max_net as usize,
            disk_quota_kb: quota as u64,
        });
    }
    Ok(out)
}

fn parse_string_array(entry: &toml::Value, key: &str) -> Vec<String> {
    entry
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn serialize_profiles_toml(profs: &[UserCapProfile]) -> String {
    let mut out = String::new();
    for p in profs {
        out.push_str("[[profile]]\n");
        out.push_str(&format!("username = {:?}\n", p.username));
        out.push_str(&format!("max_apps = {}\n", p.max_apps));
        let fmt_list = |v: &[String]| -> String {
            v.iter().map(|c| format!("{c:?}")).collect::<Vec<_>>().join(", ")
        };
        out.push_str(&format!("allowed_caps = [{}]\n", fmt_list(&p.allowed_caps)));
        out.push_str(&format!("denied_caps = [{}]\n", fmt_list(&p.denied_caps)));
        out.push_str(&format!("max_network_conns = {}\n", p.max_network_conns));
        out.push_str(&format!("disk_quota_kb = {}\n", p.disk_quota_kb));
        out.push('\n');
    }
    out
}

fn save_profiles(profs: &[UserCapProfile]) -> Result<(), String> {
    let toml = serialize_profiles_toml(profs);
    std::fs::write(CAPS_DB_PATH, toml).map_err(|e| format!("write {CAPS_DB_PATH}: {e}"))
}

// ── Profile lookup ──────────────────────────────────────────────────────────

pub fn get_profile(username: &str) -> UserCapProfile {
    let profs = lock_or_recover(&profiles());
    profs.iter().find(|p| p.username == username).cloned()
        .unwrap_or_else(|| default_profile_for(username))
}

fn upsert_profile(profile: UserCapProfile) -> Result<(), String> {
    let mut profs = lock_or_recover(&profiles());
    if let Some(existing) = profs.iter_mut().find(|p| p.username == profile.username) {
        *existing = profile;
    } else {
        profs.push(profile);
    }
    save_profiles(&profs)
}

// ── Enforcement checks ─────────────────────────────────────────────────────

/// Check whether the user can launch another app (app count limit).
pub fn check_user_can_launch(username: &str, running_count: usize) -> Result<(), String> {
    let profile = get_profile(username);
    if profile.max_apps == 0 { return Ok(()); }
    if running_count >= profile.max_apps {
        return Err(format!(
            "user '{username}' at app limit ({running_count}/{})", profile.max_apps
        ));
    }
    Ok(())
}

/// Check whether the user's profile allows a specific capability.
pub fn check_user_cap(username: &str, cap: &str) -> Result<(), String> {
    let profile = get_profile(username);
    if profile.denied_caps.iter().any(|c| c == cap) {
        return Err(format!("capability '{cap}' denied for user '{username}'"));
    }
    if !profile.allowed_caps.is_empty() && !profile.allowed_caps.iter().any(|c| c == cap) {
        return Err(format!("capability '{cap}' not in allowed set for user '{username}'"));
    }
    Ok(())
}

/// Check disk usage for a user against their quota.
pub fn check_disk_quota(username: &str) -> Result<(), String> {
    let profile = get_profile(username);
    if profile.disk_quota_kb == 0 { return Ok(()); }
    let home = format!("/data/users/{username}");
    let used_kb = dir_size_bytes(std::path::Path::new(&home)) / 1024;
    if used_kb > profile.disk_quota_kb {
        return Err(format!(
            "user '{username}' over disk quota ({used_kb} KB / {} KB)", profile.disk_quota_kb
        ));
    }
    Ok(())
}

fn dir_size_bytes(path: &std::path::Path) -> u64 {
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        let meta = match entry.metadata() { Ok(m) => m, Err(_) => continue };
        if meta.is_dir() {
            total += dir_size_bytes(&entry.path());
        } else {
            total += meta.len();
        }
    }
    total
}

// ── IPC command handler ─────────────────────────────────────────────────────

/// Handle `@supervisor: user-caps` and `@supervisor: user-set-quota` IPC commands.
pub fn handle_user_caps_command(
    verb: &str, parts: &[&str], sender: &str, inbox: &Inbox,
) -> bool {
    match verb {
        "user-caps" => {
            let target = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty())
                .map(String::from).or_else(current_user);
            let target = match target {
                Some(u) => u,
                None => {
                    send_reply(sender, "REPLY:error: not logged in and no username given", inbox);
                    return true;
                }
            };
            let p = get_profile(&target);
            let max_apps = if p.max_apps == 0 { "unlimited".into() } else { p.max_apps.to_string() };
            let max_net = if p.max_network_conns == 0 { "unlimited".into() } else { p.max_network_conns.to_string() };
            let quota = if p.disk_quota_kb == 0 { "unlimited".into() } else { format!("{} KB", p.disk_quota_kb) };
            let allowed = if p.allowed_caps.is_empty() { "all".into() } else { p.allowed_caps.join(",") };
            let denied = if p.denied_caps.is_empty() { "none".into() } else { p.denied_caps.join(",") };
            send_reply(sender, &format!(
                "REPLY:user={target} max_apps={max_apps} allowed={allowed} denied={denied} max_net={max_net} quota={quota}"
            ), inbox);
        }
        "user-set-quota" => {
            if !is_admin() {
                send_reply(sender, "REPLY:error: admin required", inbox);
                return true;
            }
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (target, kb_str) = match rest.split_once(' ') {
                Some((a, b)) if !a.trim().is_empty() && !b.trim().is_empty() => {
                    (a.trim(), b.trim())
                }
                _ => {
                    send_reply(sender, "REPLY:error: usage: user-set-quota <username> <kb>", inbox);
                    return true;
                }
            };
            let kb: u64 = match kb_str.parse() {
                Ok(v) => v,
                Err(_) => {
                    send_reply(sender, "REPLY:error: kb must be a number", inbox);
                    return true;
                }
            };
            let mut profile = get_profile(target);
            profile.disk_quota_kb = kb;
            match upsert_profile(profile) {
                Ok(()) => {
                    log_info!(Subsystem::Lifecycle, None, "user-set-quota: {target} -> {kb} KB");
                    send_reply(sender, &format!("REPLY:quota for {target} set to {kb} KB"), inbox);
                }
                Err(e) => {
                    log_warn!(Subsystem::Lifecycle, None, "user-set-quota error: {e}");
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
        }
        _ => return false,
    }
    true
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_profile_unlimited() {
        let p = default_admin_profile("root");
        assert_eq!(p.max_apps, 0);
        assert!(p.allowed_caps.is_empty());
        assert!(p.denied_caps.is_empty());
        assert_eq!(p.max_network_conns, 0);
        assert_eq!(p.disk_quota_kb, 0);
    }

    #[test]
    fn standard_profile_limits() {
        let p = default_standard_profile("alice");
        assert_eq!(p.max_apps, 10);
        assert_eq!(p.denied_caps, vec!["shell".to_string()]);
        assert_eq!(p.max_network_conns, 20);
        assert_eq!(p.disk_quota_kb, 10_240);
    }

    // Pure launch-limit check helper (avoids disk/global state).
    fn check_launch(max: usize, running: usize) -> Result<(), String> {
        if max == 0 { return Ok(()); }
        if running >= max { return Err("at limit".into()); }
        Ok(())
    }

    #[test]
    fn launch_unlimited() { assert!(check_launch(0, 100).is_ok()); }

    #[test]
    fn launch_at_limit() {
        assert!(check_launch(10, 10).is_err());
        assert!(check_launch(10, 9).is_ok());
        assert!(check_launch(10, 0).is_ok());
    }

    // Pure cap-check helper (avoids disk/global state).
    fn cap_ok(allowed: &[String], denied: &[String], cap: &str) -> Result<(), String> {
        if denied.iter().any(|c| c == cap) { return Err("denied".into()); }
        if !allowed.is_empty() && !allowed.iter().any(|c| c == cap) {
            return Err("not allowed".into());
        }
        Ok(())
    }

    #[test]
    fn cap_denied() {
        let p = default_standard_profile("bob");
        assert!(cap_ok(&p.allowed_caps, &p.denied_caps, "shell").is_err());
        assert!(cap_ok(&p.allowed_caps, &p.denied_caps, "filesystem").is_ok());
    }

    #[test]
    fn cap_allowed_list() {
        let a = vec!["stdio".into(), "filesystem".into()];
        let d: Vec<String> = Vec::new();
        assert!(cap_ok(&a, &d, "stdio").is_ok());
        assert!(cap_ok(&a, &d, "network").is_err());
    }

    #[test]
    fn cap_denied_takes_precedence() {
        let a = vec!["shell".into()];
        let d = vec!["shell".into()];
        assert!(cap_ok(&a, &d, "shell").is_err());
    }

    #[test]
    fn serialize_parse_roundtrip() {
        let profs = vec![default_admin_profile("root"), default_standard_profile("alice")];
        let toml = serialize_profiles_toml(&profs);
        let parsed = parse_profiles_toml(&toml).expect("roundtrip parse");
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].username, "root");
        assert_eq!(parsed[0].max_apps, 0);
        assert_eq!(parsed[1].username, "alice");
        assert_eq!(parsed[1].max_apps, 10);
        assert_eq!(parsed[1].denied_caps, vec!["shell".to_string()]);
        assert_eq!(parsed[1].disk_quota_kb, 10_240);
    }

    #[test]
    fn parse_empty_toml() { assert!(parse_profiles_toml("").unwrap().is_empty()); }

    #[test]
    fn parse_bad_toml() { assert!(parse_profiles_toml("not valid {{{").is_err()); }

    // Pure quota-check helper.
    fn quota_ok(quota: u64, used: u64) -> Result<(), String> {
        if quota == 0 { return Ok(()); }
        if used > quota { return Err("over".into()); }
        Ok(())
    }

    #[test]
    fn quota_unlimited() { assert!(quota_ok(0, 999_999).is_ok()); }
    #[test]
    fn quota_over() { assert!(quota_ok(1024, 2048).is_err()); }
    #[test]
    fn quota_under() { assert!(quota_ok(1024, 512).is_ok()); }
}
