// IPC module — pure parsing functions for the @target: message protocol.

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

/// Format the pong reply payload for a ping command.
/// Returns `"pong <ms>"` where `ms` is the provided millisecond timestamp.
pub fn format_pong_reply(ms: u64) -> String {
    format!("pong {ms}")
}
