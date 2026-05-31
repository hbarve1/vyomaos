// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P60: Runtime capability request engine.
//!
//! Apps can dynamically request additional capabilities at runtime via IPC.
//! The supervisor evaluates each request against a simple policy engine and
//! replies with granted/denied.  Granted capabilities are tracked per-app in
//! an in-memory registry.

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use crate::{log_info, log_warn, send_reply, AppRegistry, Inbox};
use supervisor::logging::Subsystem;

// ── Capability names ────────────────────────────────────────────────────────

/// The set of capability names that apps can request at runtime.
const KNOWN_CAPS: &[&str] = &[
    "stdio",
    "filesystem",
    "network",
    "display",
    "shell",
    "mouse",
    "audio",
];

fn is_known_cap(name: &str) -> bool {
    KNOWN_CAPS.contains(&name)
}

// ── Policy engine ───────────────────────────────────────────────────────────

/// Per-capability grant policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapPolicy {
    /// Automatically grant the capability without user interaction.
    AutoGrant,
    /// Would prompt the user (future; currently treated as Deny).
    Prompt,
    /// Always deny the request.
    Deny,
}

/// Return the default policy for a given capability name.
pub fn default_policy(cap: &str) -> CapPolicy {
    match cap {
        "filesystem" | "network" | "stdio" | "display" | "mouse" | "audio" => CapPolicy::AutoGrant,
        "shell" => CapPolicy::Deny,
        _ => CapPolicy::Deny,
    }
}

/// Evaluate whether a capability request should be granted.
///
/// Returns `Ok(())` on grant, `Err(reason)` on denial.
pub fn evaluate_policy(cap: &str) -> Result<(), String> {
    if !is_known_cap(cap) {
        return Err(format!("unknown capability '{cap}'"));
    }
    match default_policy(cap) {
        CapPolicy::AutoGrant => Ok(()),
        CapPolicy::Prompt => Err("capability requires user approval (not yet implemented)".into()),
        CapPolicy::Deny => Err(format!("capability '{cap}' is denied by policy")),
    }
}

// ── Runtime grant registry ──────────────────────────────────────────────────

/// Global per-app runtime capability grants.
/// Key: app name, Value: set of granted capability names.
static RUNTIME_CAPS: OnceLock<Mutex<HashMap<String, HashSet<String>>>> = OnceLock::new();

fn runtime_caps() -> &'static Mutex<HashMap<String, HashSet<String>>> {
    RUNTIME_CAPS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Record a granted capability for an app.
pub fn grant_cap(app: &str, cap: &str) {
    runtime_caps()
        .lock()
        .unwrap()
        .entry(app.to_string())
        .or_default()
        .insert(cap.to_string());
}

/// Remove a runtime capability from an app. Returns true if it was present.
pub fn revoke_cap(app: &str, cap: &str) -> bool {
    let mut map = runtime_caps().lock().unwrap();
    if let Some(set) = map.get_mut(app) {
        return set.remove(cap);
    }
    false
}

/// List all runtime-granted capabilities for an app.
pub fn list_caps(app: &str) -> Vec<String> {
    let map = runtime_caps().lock().unwrap();
    match map.get(app) {
        Some(set) => {
            let mut v: Vec<String> = set.iter().cloned().collect();
            v.sort();
            v
        }
        None => Vec::new(),
    }
}

/// Check whether an app has a specific runtime-granted capability.
pub fn has_cap(app: &str, cap: &str) -> bool {
    let map = runtime_caps().lock().unwrap();
    map.get(app).map(|s| s.contains(cap)).unwrap_or(false)
}

// ── IPC command handlers ────────────────────────────────────────────────────

/// Handle `@supervisor: request-cap <capability>`.
pub fn handle_request_cap(
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
    app_registry: &AppRegistry,
) {
    let cap = match parts.get(1).map(|s| s.trim()) {
        Some(c) if !c.is_empty() => c.to_string(),
        _ => {
            send_reply(sender, "REPLY:cap-denied (none) usage: request-cap <capability>", inbox);
            return;
        }
    };

    // Check if the app is registered.
    {
        let reg = app_registry.lock().unwrap();
        if !reg.contains_key(sender) {
            send_reply(
                sender,
                &format!("REPLY:cap-denied {cap} app not found"),
                inbox,
            );
            return;
        }
    }

    // Already granted?
    if has_cap(sender, &cap) {
        send_reply(
            sender,
            &format!("REPLY:cap-granted {cap}"),
            inbox,
        );
        log_info!(Subsystem::Capability, Some(sender), "request-cap {cap}: already granted");
        return;
    }

    match evaluate_policy(&cap) {
        Ok(()) => {
            grant_cap(sender, &cap);
            log_info!(Subsystem::Capability, Some(sender), "request-cap {cap}: granted");
            send_reply(sender, &format!("REPLY:cap-granted {cap}"), inbox);
            // Broadcast capability change to all running apps.
            broadcast_cap_change(sender, &cap, "granted", inbox, app_registry);
        }
        Err(reason) => {
            log_warn!(Subsystem::Capability, Some(sender), "request-cap {cap}: denied — {reason}");
            send_reply(
                sender,
                &format!("REPLY:cap-denied {cap} {reason}"),
                inbox,
            );
        }
    }
}

/// Handle `@supervisor: revoke-cap <app> <capability>`.
pub fn handle_revoke_cap(
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
    app_registry: &AppRegistry,
) {
    let rest = parts.get(1).unwrap_or(&"").trim();
    let (target_app, cap) = match rest.split_once(' ') {
        Some((a, c)) if !a.trim().is_empty() && !c.trim().is_empty() => {
            (a.trim().to_string(), c.trim().to_string())
        }
        _ => {
            send_reply(
                sender,
                "REPLY:error: usage: revoke-cap <app> <capability>",
                inbox,
            );
            return;
        }
    };

    if !is_known_cap(&cap) {
        send_reply(
            sender,
            &format!("REPLY:error: unknown capability '{cap}'"),
            inbox,
        );
        return;
    }

    if revoke_cap(&target_app, &cap) {
        log_info!(
            Subsystem::Capability, Some(sender),
            "revoke-cap {target_app}:{cap} by {sender}"
        );
        send_reply(
            sender,
            &format!("REPLY:cap-revoked {target_app} {cap}"),
            inbox,
        );
        broadcast_cap_change(&target_app, &cap, "revoked", inbox, app_registry);
    } else {
        send_reply(
            sender,
            &format!("REPLY:error: {target_app} does not have runtime cap '{cap}'"),
            inbox,
        );
    }
}

/// Handle `@supervisor: list-caps <app>`.
pub fn handle_list_caps(
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) {
    let app_name = match parts.get(1).map(|s| s.trim()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            send_reply(sender, "REPLY:error: usage: list-caps <app>", inbox);
            return;
        }
    };

    let caps = list_caps(&app_name);
    if caps.is_empty() {
        send_reply(
            sender,
            &format!("REPLY:caps {app_name} (none)"),
            inbox,
        );
    } else {
        send_reply(
            sender,
            &format!("REPLY:caps {app_name} {}", caps.join(",")),
            inbox,
        );
    }
}

/// Broadcast a capability change notification to all running apps.
fn broadcast_cap_change(
    app: &str,
    cap: &str,
    action: &str,
    inbox: &Inbox,
    app_registry: &AppRegistry,
) {
    let msg = format!("VYOMA_SYSTEM:cap-change:{app}:{cap}:{action}");
    let reg = app_registry.lock().unwrap();
    let inb = inbox.lock().unwrap();
    for (name, state_arc) in reg.iter() {
        let st = state_arc.lock().unwrap();
        if matches!(st.status, crate::AppStatus::Running) {
            if let Some(tx) = inb.get(name) {
                let _ = tx.send(msg.clone());
            }
        }
    }
}

// ── Unit tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_caps_recognised() {
        assert!(is_known_cap("filesystem"));
        assert!(is_known_cap("network"));
        assert!(is_known_cap("shell"));
        assert!(!is_known_cap("teleport"));
    }

    #[test]
    fn default_policy_auto_grant() {
        assert_eq!(default_policy("filesystem"), CapPolicy::AutoGrant);
        assert_eq!(default_policy("network"), CapPolicy::AutoGrant);
        assert_eq!(default_policy("stdio"), CapPolicy::AutoGrant);
        assert_eq!(default_policy("display"), CapPolicy::AutoGrant);
        assert_eq!(default_policy("mouse"), CapPolicy::AutoGrant);
        assert_eq!(default_policy("audio"), CapPolicy::AutoGrant);
    }

    #[test]
    fn default_policy_deny_shell() {
        assert_eq!(default_policy("shell"), CapPolicy::Deny);
    }

    #[test]
    fn default_policy_deny_unknown() {
        assert_eq!(default_policy("magic"), CapPolicy::Deny);
    }

    #[test]
    fn evaluate_grants_filesystem() {
        assert!(evaluate_policy("filesystem").is_ok());
    }

    #[test]
    fn evaluate_denies_shell() {
        let err = evaluate_policy("shell").unwrap_err();
        assert!(err.contains("denied"));
    }

    #[test]
    fn evaluate_denies_unknown() {
        let err = evaluate_policy("foobar").unwrap_err();
        assert!(err.contains("unknown"));
    }

    #[test]
    fn grant_and_list() {
        // Use a unique app name to avoid cross-test interference.
        let app = "test-grant-list-app";
        grant_cap(app, "filesystem");
        grant_cap(app, "network");
        let caps = list_caps(app);
        assert!(caps.contains(&"filesystem".to_string()));
        assert!(caps.contains(&"network".to_string()));
        assert!(has_cap(app, "filesystem"));
        assert!(!has_cap(app, "shell"));
    }

    #[test]
    fn revoke_removes_cap() {
        let app = "test-revoke-app";
        grant_cap(app, "network");
        assert!(has_cap(app, "network"));
        assert!(revoke_cap(app, "network"));
        assert!(!has_cap(app, "network"));
    }

    #[test]
    fn revoke_nonexistent_returns_false() {
        let app = "test-revoke-none-app";
        assert!(!revoke_cap(app, "display"));
    }

    #[test]
    fn list_caps_empty_for_unknown_app() {
        let caps = list_caps("nonexistent-test-app-xyz");
        assert!(caps.is_empty());
    }

    #[test]
    fn grant_idempotent() {
        let app = "test-idempotent-app";
        grant_cap(app, "audio");
        grant_cap(app, "audio");
        let caps = list_caps(app);
        assert_eq!(caps.iter().filter(|c| *c == "audio").count(), 1);
    }
}
