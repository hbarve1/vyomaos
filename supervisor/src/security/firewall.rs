// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P88: Per-app firewall rules.
//!
//! Provides a rule-based firewall that controls network access on a per-app
//! basis. Rules are stored in-memory (loaded from `/data/firewall.toml` at
//! startup) and can be managed at runtime via IPC commands.
//!
//! Design: first-match-wins semantics. Apps without `network = true` in their
//! manifest are implicitly denied all network access regardless of rules.

use std::sync::{Mutex, OnceLock};

use crate::{log_info, log_warn, send_reply, Inbox};
use supervisor::logging::Subsystem;

// ── Firewall rule model ─────────────────────────────────────────────────────

/// Action to take when a rule matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirewallAction {
    Allow,
    Deny,
}

/// Traffic direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Inbound,
    Outbound,
}

/// A single firewall rule scoped to an app.
#[derive(Debug, Clone)]
pub struct FirewallRule {
    pub app_name: String,
    pub direction: Direction,
    /// `None` means match all ports.
    pub port: Option<u16>,
    /// `None` means match all hosts.
    pub host: Option<String>,
    pub action: FirewallAction,
}

// ── Rule storage ────────────────────────────────────────────────────────────

static FIREWALL_RULES: OnceLock<Mutex<Vec<FirewallRule>>> = OnceLock::new();

fn rules() -> &'static Mutex<Vec<FirewallRule>> {
    FIREWALL_RULES.get_or_init(|| Mutex::new(Vec::new()))
}

/// Load firewall rules from `/data/firewall.toml`.
///
/// Expected TOML format:
/// ```toml
/// [[rule]]
/// app  = "http-server"
/// action = "allow"
/// direction = "inbound"
/// port = 8080
///
/// [[rule]]
/// app  = "browser"
/// action = "deny"
/// direction = "outbound"
/// host = "evil.example.com"
/// ```
pub fn load_rules() {
    const PATH: &str = "/data/firewall.toml";
    let raw = match std::fs::read_to_string(PATH) {
        Ok(s) => s,
        Err(_) => return, // No file — start with empty rules.
    };

    let doc: toml::Value = match toml::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            log_warn!(Subsystem::Lifecycle, None, "firewall.toml parse error: {e}");
            return;
        }
    };

    let entries = match doc.get("rule").and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return,
    };

    let mut loaded = Vec::new();
    for entry in entries {
        let app_name = match entry.get("app").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let action = match entry.get("action").and_then(|v| v.as_str()) {
            Some("allow") => FirewallAction::Allow,
            Some("deny") => FirewallAction::Deny,
            _ => continue,
        };
        let direction = match entry.get("direction").and_then(|v| v.as_str()) {
            Some("inbound") => Direction::Inbound,
            Some("outbound") => Direction::Outbound,
            _ => continue,
        };
        let port = entry.get("port").and_then(|v| v.as_integer()).map(|p| p as u16);
        let host = entry.get("host").and_then(|v| v.as_str()).map(|s| s.to_string());

        loaded.push(FirewallRule { app_name, direction, port, host, action });
    }

    let count = loaded.len();
    *rules().lock().unwrap() = loaded;
    log_info!(Subsystem::Lifecycle, None, "firewall: loaded {count} rule(s) from {PATH}");
}

// ── Check function ──────────────────────────────────────────────────────────

/// Check whether a network operation should be permitted.
///
/// Returns `true` if the operation is allowed.
///
/// Semantics:
/// - Iterate rules in order; the first rule matching the app, direction,
///   host, and port determines the outcome.
/// - If no rule matches, the default is **allow** (apps with `network = true`
///   are allowed by default; apps without it never reach this function).
pub fn check_network_access(
    app: &str,
    direction: Direction,
    host: &str,
    port: u16,
) -> bool {
    let rules_guard = rules().lock().unwrap();
    for rule in rules_guard.iter() {
        if rule.app_name != app {
            continue;
        }
        if rule.direction != direction {
            continue;
        }
        if let Some(ref rule_host) = rule.host {
            if rule_host != host {
                continue;
            }
        }
        if let Some(rule_port) = rule.port {
            if rule_port != port {
                continue;
            }
        }
        // First matching rule wins.
        return rule.action == FirewallAction::Allow;
    }
    // Default: allow (the manifest capability gate is the primary control).
    true
}

// ── Rule management helpers ─────────────────────────────────────────────────

/// Add a rule to the end of the rule list. Returns the new index.
pub fn add_rule(rule: FirewallRule) -> usize {
    let mut r = rules().lock().unwrap();
    r.push(rule);
    r.len() - 1
}

/// Remove a rule by index. Returns the removed rule or `None` if out of bounds.
pub fn remove_rule(index: usize) -> Option<FirewallRule> {
    let mut r = rules().lock().unwrap();
    if index < r.len() {
        Some(r.remove(index))
    } else {
        None
    }
}

/// Return a snapshot of all current rules.
pub fn list_rules() -> Vec<FirewallRule> {
    rules().lock().unwrap().clone()
}

// ── Formatting helpers ──────────────────────────────────────────────────────

fn format_action(a: FirewallAction) -> &'static str {
    match a {
        FirewallAction::Allow => "allow",
        FirewallAction::Deny => "deny",
    }
}

fn format_direction(d: Direction) -> &'static str {
    match d {
        Direction::Inbound => "inbound",
        Direction::Outbound => "outbound",
    }
}

fn format_rule(idx: usize, r: &FirewallRule) -> String {
    let host = r.host.as_deref().unwrap_or("*");
    let port = r.port.map(|p| p.to_string()).unwrap_or_else(|| "*".to_string());
    format!(
        "[{idx}] {} {} {} host={} port={}",
        r.app_name,
        format_action(r.action),
        format_direction(r.direction),
        host,
        port,
    )
}

// ── Parse helpers ───────────────────────────────────────────────────────────

fn parse_action(s: &str) -> Option<FirewallAction> {
    match s {
        "allow" => Some(FirewallAction::Allow),
        "deny" => Some(FirewallAction::Deny),
        _ => None,
    }
}

fn parse_direction(s: &str) -> Option<Direction> {
    match s {
        "inbound" => Some(Direction::Inbound),
        "outbound" => Some(Direction::Outbound),
        _ => None,
    }
}

// ── IPC command handlers ────────────────────────────────────────────────────

/// Handle firewall IPC commands. Returns `true` if the command was handled.
pub fn handle_firewall_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "firewall-list" => {
            let all = list_rules();
            if all.is_empty() {
                send_reply(sender, "REPLY:firewall: no rules", inbox);
            } else {
                let lines: Vec<String> = all.iter().enumerate()
                    .map(|(i, r)| format_rule(i, r))
                    .collect();
                send_reply(sender, &format!("REPLY:firewall:{}", lines.join("|")), inbox);
            }
        }

        // firewall-add <app> <allow|deny> <inbound|outbound> [host] [port]
        "firewall-add" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            if tokens.len() < 3 {
                send_reply(
                    sender,
                    "REPLY:error: usage: firewall-add <app> <allow|deny> <inbound|outbound> [host] [port]",
                    inbox,
                );
                return true;
            }
            let app_name = tokens[0].to_string();
            let action = match parse_action(tokens[1]) {
                Some(a) => a,
                None => {
                    send_reply(sender, "REPLY:error: action must be allow or deny", inbox);
                    return true;
                }
            };
            let direction = match parse_direction(tokens[2]) {
                Some(d) => d,
                None => {
                    send_reply(sender, "REPLY:error: direction must be inbound or outbound", inbox);
                    return true;
                }
            };
            let host = tokens.get(3).and_then(|s| {
                if *s == "*" { None } else { Some(s.to_string()) }
            });
            let port: Option<u16> = tokens.get(4).and_then(|s| {
                if *s == "*" { None } else { s.parse().ok() }
            });

            let rule = FirewallRule { app_name: app_name.clone(), direction, port, host, action };
            let idx = add_rule(rule);
            log_info!(
                Subsystem::Lifecycle, None,
                "firewall: rule {idx} added by {sender} for {app_name}"
            );
            send_reply(
                sender,
                &format!("REPLY:firewall: rule {idx} added for {app_name}"),
                inbox,
            );
        }

        // firewall-remove <index>
        "firewall-remove" => {
            let idx_str = parts.get(1).unwrap_or(&"").trim();
            let idx: usize = match idx_str.parse() {
                Ok(i) => i,
                Err(_) => {
                    send_reply(sender, "REPLY:error: usage: firewall-remove <index>", inbox);
                    return true;
                }
            };
            match remove_rule(idx) {
                Some(removed) => {
                    log_info!(
                        Subsystem::Lifecycle, None,
                        "firewall: rule {idx} removed by {sender} (was: {} {})",
                        removed.app_name, format_action(removed.action)
                    );
                    send_reply(
                        sender,
                        &format!("REPLY:firewall: rule {idx} removed"),
                        inbox,
                    );
                }
                None => {
                    send_reply(
                        sender,
                        &format!("REPLY:error: rule index {idx} out of range"),
                        inbox,
                    );
                }
            }
        }

        // firewall-check <app> <host> <port>
        "firewall-check" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let tokens: Vec<&str> = rest.split_whitespace().collect();
            if tokens.len() < 3 {
                send_reply(
                    sender,
                    "REPLY:error: usage: firewall-check <app> <host> <port>",
                    inbox,
                );
                return true;
            }
            let app_name = tokens[0];
            let host = tokens[1];
            let port: u16 = match tokens[2].parse() {
                Ok(p) => p,
                Err(_) => {
                    send_reply(sender, "REPLY:error: invalid port number", inbox);
                    return true;
                }
            };
            let out_ok = check_network_access(app_name, Direction::Outbound, host, port);
            let in_ok = check_network_access(app_name, Direction::Inbound, host, port);
            send_reply(
                sender,
                &format!(
                    "REPLY:firewall-check {app_name} {host}:{port} outbound={} inbound={}",
                    if out_ok { "allow" } else { "deny" },
                    if in_ok { "allow" } else { "deny" },
                ),
                inbox,
            );
        }

        _ => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(app: &str, act: FirewallAction, dir: Direction,
            host: Option<&str>, port: Option<u16>) -> FirewallRule {
        FirewallRule {
            app_name: app.to_string(), direction: dir,
            port, host: host.map(|s| s.to_string()), action: act,
        }
    }

    #[test]
    fn default_allows_when_no_rules() {
        assert!(check_network_access("fw-no-rules", Direction::Outbound, "example.com", 443));
        assert!(check_network_access("fw-no-rules", Direction::Inbound, "0.0.0.0", 80));
    }

    #[test]
    fn deny_rule_blocks_access() {
        let i = add_rule(rule("fw-deny", FirewallAction::Deny, Direction::Outbound, Some("evil.com"), Some(443)));
        assert!(!check_network_access("fw-deny", Direction::Outbound, "evil.com", 443));
        assert!(check_network_access("fw-deny", Direction::Outbound, "good.com", 443));
        remove_rule(i);
    }

    #[test]
    fn allow_rule_permits() {
        let i = add_rule(rule("fw-allow", FirewallAction::Allow, Direction::Inbound, None, Some(8080)));
        assert!(check_network_access("fw-allow", Direction::Inbound, "any", 8080));
        remove_rule(i);
    }

    #[test]
    fn first_match_wins() {
        let i1 = add_rule(rule("fw-fmw", FirewallAction::Deny, Direction::Outbound, Some("blocked.com"), None));
        let i2 = add_rule(rule("fw-fmw", FirewallAction::Allow, Direction::Outbound, None, None));
        assert!(!check_network_access("fw-fmw", Direction::Outbound, "blocked.com", 80));
        assert!(check_network_access("fw-fmw", Direction::Outbound, "other.com", 80));
        remove_rule(i2); remove_rule(i1);
    }

    #[test]
    fn port_filter() {
        let i = add_rule(rule("fw-port", FirewallAction::Deny, Direction::Outbound, None, Some(22)));
        assert!(!check_network_access("fw-port", Direction::Outbound, "ssh.example.com", 22));
        assert!(check_network_access("fw-port", Direction::Outbound, "ssh.example.com", 443));
        remove_rule(i);
    }

    #[test]
    fn host_filter() {
        let i = add_rule(rule("fw-host", FirewallAction::Deny, Direction::Outbound, Some("bad.net"), None));
        assert!(!check_network_access("fw-host", Direction::Outbound, "bad.net", 80));
        assert!(check_network_access("fw-host", Direction::Outbound, "good.net", 80));
        remove_rule(i);
    }

    #[test]
    fn direction_filter() {
        let i = add_rule(rule("fw-dir", FirewallAction::Deny, Direction::Inbound, None, None));
        assert!(!check_network_access("fw-dir", Direction::Inbound, "any", 80));
        assert!(check_network_access("fw-dir", Direction::Outbound, "any", 80));
        remove_rule(i);
    }

    #[test]
    fn wildcard_denies_all() {
        let i = add_rule(rule("fw-wild", FirewallAction::Deny, Direction::Outbound, None, None));
        assert!(!check_network_access("fw-wild", Direction::Outbound, "any.host", 12345));
        remove_rule(i);
    }

    #[test]
    fn remove_out_of_bounds() { assert!(remove_rule(999999).is_none()); }

    #[test]
    fn list_rules_snapshot() {
        let before = list_rules().len();
        let i = add_rule(rule("fw-snap", FirewallAction::Allow, Direction::Outbound, None, Some(80)));
        assert_eq!(list_rules().len(), before + 1);
        remove_rule(i);
    }

    #[test]
    fn format_rule_output() {
        let r = rule("myapp", FirewallAction::Deny, Direction::Outbound, Some("example.com"), Some(443));
        let s = format_rule(0, &r);
        assert!(s.contains("myapp") && s.contains("deny") && s.contains("443"));
    }

    #[test]
    fn parse_action_and_direction() {
        assert_eq!(parse_action("allow"), Some(FirewallAction::Allow));
        assert_eq!(parse_action("deny"), Some(FirewallAction::Deny));
        assert_eq!(parse_action("block"), None);
        assert_eq!(parse_direction("inbound"), Some(Direction::Inbound));
        assert_eq!(parse_direction("outbound"), Some(Direction::Outbound));
        assert_eq!(parse_direction("both"), None);
    }
}
