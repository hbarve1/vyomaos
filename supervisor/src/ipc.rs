// IPC module — pure parsing functions for the @target: message protocol.

/// Return a formatted VyomaOS version string, e.g. `"VyomaOS 0.19.0"`.
/// Pure function: no I/O, no global state.
pub fn format_version(major: u32, minor: u32, patch: u32) -> String {
    format!("VyomaOS {major}.{minor}.{patch}")
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
