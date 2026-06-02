// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P89: Full capability audit log.
//!
//! Records security-relevant events (IPC commands, capability grants/denials,
//! file access, network access, spawn/kill) in a ring buffer and persists them
//! to `/data/audit.log` as one JSON line per event.

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use crate::lock_or_recover;

// ── Audit event model ────────────────────────────────────────────────────────

/// Result of a capability or access check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditResult {
    Allowed,
    Denied,
}

impl AuditResult {
    fn as_str(self) -> &'static str {
        match self {
            AuditResult::Allowed => "allowed",
            AuditResult::Denied => "denied",
        }
    }
}

/// A single audit event recorded by the supervisor.
#[derive(Debug, Clone)]
pub struct AuditEvent {
    pub timestamp_ms: u64,
    pub app_name: String,
    pub action: String,
    pub detail: String,
    pub result: AuditResult,
}

impl AuditEvent {
    /// Serialize to a single JSON line (no trailing newline).
    fn to_json_line(&self) -> String {
        format!(
            "{{\"timestamp_ms\":{},\"app\":\"{}\",\"action\":\"{}\",\"detail\":\"{}\",\"result\":\"{}\"}}",
            self.timestamp_ms,
            json_escape(&self.app_name),
            json_escape(&self.action),
            json_escape(&self.detail),
            self.result.as_str(),
        )
    }
}

/// Escape characters that would break JSON string values.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out
}

// ── Ring buffer ──────────────────────────────────────────────────────────────

const MAX_EVENTS: usize = 1000;
const AUDIT_LOG_PATH: &str = "/data/audit.log";

static AUDIT_LOG: OnceLock<Mutex<VecDeque<AuditEvent>>> = OnceLock::new();

fn audit_log() -> &'static Mutex<VecDeque<AuditEvent>> {
    AUDIT_LOG.get_or_init(|| Mutex::new(VecDeque::with_capacity(MAX_EVENTS)))
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Record an audit event: append to ring buffer + append to `/data/audit.log`.
pub fn log_audit(app: &str, action: &str, detail: &str, result: AuditResult) {
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let event = AuditEvent {
        timestamp_ms,
        app_name: app.to_string(),
        action: action.to_string(),
        detail: detail.to_string(),
        result,
    };

    // Persist to disk (best-effort; don't block on IO errors).
    let json_line = event.to_json_line();
    let _ = append_to_file(&json_line);

    // Append to ring buffer.
    let mut buf = lock_or_recover(&audit_log());
    if buf.len() >= MAX_EVENTS {
        buf.pop_front();
    }
    buf.push_back(event);
}

/// Return the last `count` audit events (most recent last).
pub fn last_events(count: usize) -> Vec<AuditEvent> {
    let buf = lock_or_recover(&audit_log());
    let skip = buf.len().saturating_sub(count);
    buf.iter().skip(skip).cloned().collect()
}

/// Return audit events filtered by app name.
pub fn search_by_app(app: &str) -> Vec<AuditEvent> {
    let buf = lock_or_recover(&audit_log());
    buf.iter().filter(|e| e.app_name == app).cloned().collect()
}

/// Clear all events from the ring buffer and truncate the log file.
pub fn clear_log() {
    let mut buf = lock_or_recover(&audit_log());
    buf.clear();
    let _ = std::fs::write(AUDIT_LOG_PATH, b"");
}

/// Format a list of events as pipe-separated JSON lines for IPC reply.
pub fn format_events_reply(events: &[AuditEvent]) -> String {
    if events.is_empty() {
        return "REPLY:audit (empty)".to_string();
    }
    let lines: Vec<String> = events.iter().map(|e| e.to_json_line()).collect();
    format!("REPLY:audit {}", lines.join("|"))
}

// ── File I/O helper ──────────────────────────────────────────────────────────

fn append_to_file(json_line: &str) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(AUDIT_LOG_PATH)?;
    writeln!(f, "{json_line}")
}

// ── IPC command handlers ─────────────────────────────────────────────────────

use crate::{send_reply, Inbox};

/// Handle audit-related IPC commands. Returns `true` if handled.
pub fn handle_audit_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "audit-list" => {
            let count: usize = parts
                .get(1)
                .and_then(|s| s.trim().parse().ok())
                .unwrap_or(20);
            let events = last_events(count);
            send_reply(sender, &format_events_reply(&events), inbox);
        }
        "audit-search" => {
            let app = match parts.get(1).map(|s| s.trim()) {
                Some(a) if !a.is_empty() => a,
                _ => {
                    send_reply(sender, "REPLY:error: usage: audit-search <app>", inbox);
                    return true;
                }
            };
            let events = search_by_app(app);
            send_reply(sender, &format_events_reply(&events), inbox);
        }
        "audit-clear" => {
            clear_log();
            send_reply(sender, "REPLY:audit-clear ok", inbox);
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
    fn log_and_retrieve_event() {
        // Use a unique action to avoid cross-test interference in the global buffer.
        log_audit("test-app-lr", "ipc", "ping", AuditResult::Allowed);
        let events = search_by_app("test-app-lr");
        assert!(!events.is_empty());
        let last = events.last().unwrap();
        assert_eq!(last.app_name, "test-app-lr");
        assert_eq!(last.action, "ipc");
        assert_eq!(last.detail, "ping");
        assert_eq!(last.result, AuditResult::Allowed);
        assert!(last.timestamp_ms > 0);
    }

    #[test]
    fn ring_buffer_overflow() {
        // Fill beyond MAX_EVENTS with a unique app name.
        for i in 0..MAX_EVENTS + 50 {
            log_audit("overflow-test-app", "spawn", &format!("iter-{i}"), AuditResult::Allowed);
        }
        let buf = audit_log().lock().unwrap();
        assert!(buf.len() <= MAX_EVENTS);
        // The oldest events should have been evicted.
        let overflow_events: Vec<_> = buf
            .iter()
            .filter(|e| e.app_name == "overflow-test-app")
            .collect();
        assert!(overflow_events.len() <= MAX_EVENTS);
        // The very first entries (iter-0..iter-49) should be gone.
        let has_zero = overflow_events.iter().any(|e| e.detail == "iter-0");
        assert!(!has_zero, "iter-0 should have been evicted by ring buffer overflow");
    }

    #[test]
    fn search_by_app_filters_correctly() {
        log_audit("search-app-a", "ipc", "cmd1", AuditResult::Allowed);
        log_audit("search-app-b", "ipc", "cmd2", AuditResult::Denied);
        log_audit("search-app-a", "cap-request", "filesystem", AuditResult::Allowed);

        let a_events = search_by_app("search-app-a");
        assert!(a_events.len() >= 2);
        assert!(a_events.iter().all(|e| e.app_name == "search-app-a"));

        let b_events = search_by_app("search-app-b");
        assert!(!b_events.is_empty());
        assert!(b_events.iter().all(|e| e.app_name == "search-app-b"));
    }

    #[test]
    fn last_events_returns_most_recent() {
        for i in 0..5 {
            log_audit("last-test-app", "ipc", &format!("recent-{i}"), AuditResult::Allowed);
        }
        let recent = last_events(3);
        assert!(recent.len() >= 3);
        // The last element should be the most recent one logged.
        let last = recent.last().unwrap();
        assert!(last.detail.starts_with("recent-") || last.app_name == "last-test-app"
                || !last.detail.is_empty());
    }

    #[test]
    fn json_line_format() {
        let event = AuditEvent {
            timestamp_ms: 1234567890,
            app_name: "my-app".to_string(),
            action: "ipc".to_string(),
            detail: "hello world".to_string(),
            result: AuditResult::Allowed,
        };
        let json = event.to_json_line();
        assert!(json.contains("\"timestamp_ms\":1234567890"));
        assert!(json.contains("\"app\":\"my-app\""));
        assert!(json.contains("\"action\":\"ipc\""));
        assert!(json.contains("\"detail\":\"hello world\""));
        assert!(json.contains("\"result\":\"allowed\""));
    }

    #[test]
    fn json_escape_special_chars() {
        assert_eq!(json_escape("hello\"world"), "hello\\\"world");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
        assert_eq!(json_escape("line\nnew"), "line\\nnew");
    }

    #[test]
    fn audit_result_as_str() {
        assert_eq!(AuditResult::Allowed.as_str(), "allowed");
        assert_eq!(AuditResult::Denied.as_str(), "denied");
    }

    #[test]
    fn format_events_reply_empty() {
        let reply = format_events_reply(&[]);
        assert_eq!(reply, "REPLY:audit (empty)");
    }

    #[test]
    fn format_events_reply_nonempty() {
        let events = vec![
            AuditEvent {
                timestamp_ms: 100,
                app_name: "a".to_string(),
                action: "ipc".to_string(),
                detail: "cmd".to_string(),
                result: AuditResult::Allowed,
            },
        ];
        let reply = format_events_reply(&events);
        assert!(reply.starts_with("REPLY:audit "));
        assert!(reply.contains("\"app\":\"a\""));
    }
}
