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

/// Return `true` if `name` exactly matches one of the entries in `running_names`.
/// Pure function — no I/O, no side effects.
pub fn validate_kill_target<'a>(name: &str, running_names: &'a [&'a str]) -> bool {
    running_names.iter().any(|&n| n == name)
}
