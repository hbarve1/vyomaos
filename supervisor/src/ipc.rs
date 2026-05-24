// IPC module — pure parsing functions for the @target: message protocol.

use std::collections::HashMap;

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

/// Returns `true` if `target` is the special `"reply"` pseudo-target.
pub fn is_reply_target(target: &str) -> bool {
    target == "reply"
}

/// Look up who last sent an IPC message to `sender`.
/// Returns `Some(original_sender)` if found, or `None` if no prior message exists.
pub fn resolve_reply<'a>(sender: &str, last_senders: &'a HashMap<String, String>) -> Option<&'a str> {
    last_senders.get(sender).map(|s| s.as_str())
}
