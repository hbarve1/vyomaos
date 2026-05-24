// IPC module — pure parsing functions for the @target: message protocol.

use std::collections::HashMap;

/// Per-app log level filter — controls which messages are forwarded to an app's log.
/// Ordered so that `Debug < Info < Warn < Error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel { Debug = 0, Info = 1, Warn = 2, Error = 3 }

/// Parse a log level name (case-insensitive).
/// Recognised values: "debug", "info", "warn", "error".
/// Returns `None` for any unrecognised string.
pub fn parse_log_level(s: &str) -> Option<LogLevel> {
    match s.trim().to_lowercase().as_str() {
        "debug" => Some(LogLevel::Debug),
        "info"  => Some(LogLevel::Info),
        "warn"  => Some(LogLevel::Warn),
        "error" => Some(LogLevel::Error),
        _       => None,
    }
}

/// Return a newline-separated list of app names with no trailing newline.
/// Pure function: no I/O, no side-effects.
pub fn format_app_list(names: &[&str]) -> String {
    names.join("\n")
}

/// Parse an IPC line in the format `@<target>: <message>`.
/// Returns `Ok((target, payload))` or `Err` if the format is invalid or target is empty.
pub fn parse_ipc_target(line: &str) -> Result<(&str, &str), String> {
    let rest = line.strip_prefix('@')
        .ok_or_else(|| format!("IPC line does not start with '@': {line}"))?;
    let sep = rest.find(": ")
        .ok_or_else(|| format!("IPC line missing ': ' separator: {line}"))?;
    let target = &rest[..sep];
    if target.is_empty() {
        return Err(format!("IPC line has empty target: {line}"));
    }
    let payload = &rest[sep + 2..];
    Ok((target, payload))
}

/// Return `true` if `target` is the reserved broadcast address.
/// When an app writes `@broadcast: <message>` the supervisor delivers
/// `<message>` to every currently running app (including the sender).
pub fn is_broadcast_target(target: &str) -> bool {
    target == "broadcast"
}

/// Returns `true` if `target` is the special `"reply"` pseudo-target.
pub fn is_reply_target(target: &str) -> bool {
    target == "reply"
}

/// Look up who last sent an IPC message to `sender`.
/// Returns `Some(original_sender)` if found, or `None` if no prior message exists.
pub fn resolve_reply<'a>(sender: &str, last_senders: &'a HashMap<String, String>) -> Option<&'a str> {
    last_senders.get(sender).map(|s| s.as_str())
}

/// Format the pong reply payload for a ping command.
/// Returns `"pong <ms>"` where `ms` is the provided millisecond timestamp.
pub fn format_pong_reply(ms: u64) -> String {
    format!("pong {ms}")
}

/// Return `true` if `name` exactly matches one of the entries in `running_names`.
/// Pure function — no I/O, no side effects.
pub fn validate_kill_target<'a>(name: &str, running_names: &'a [&'a str]) -> bool {
    running_names.iter().any(|&n| n == name)
}
