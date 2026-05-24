// IPC module — pure parsing functions for the @target: message protocol.

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
