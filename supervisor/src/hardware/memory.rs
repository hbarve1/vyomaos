// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P94: Memory pressure manager — monitors system memory, broadcasts pressure
//! events to apps, and kills lowest-priority background apps under critical pressure.

use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::{log_info, log_warn, AppRegistry, AppStatus, Inbox};
use supervisor::logging::Subsystem;

// ── Memory info from /proc/meminfo ──────────────────────────────────────────

/// Parsed subset of `/proc/meminfo` fields (all values in KB).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemInfo {
    pub total_kb:     u64,
    pub free_kb:      u64,
    pub available_kb: u64,
    pub used_kb:      u64,
}

/// Parse `/proc/meminfo` content into a `MemInfo`.
///
/// Expected format (Linux standard):
/// ```text
/// MemTotal:       16384000 kB
/// MemFree:         8192000 kB
/// MemAvailable:   12000000 kB
/// Buffers:          512000 kB
/// Cached:          2048000 kB
/// ...
/// ```
pub fn parse_meminfo(content: &str) -> Option<MemInfo> {
    let mut total: Option<u64> = None;
    let mut free: Option<u64> = None;
    let mut available: Option<u64> = None;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("MemTotal:") {
            total = parse_kb_value(rest);
        } else if let Some(rest) = line.strip_prefix("MemFree:") {
            free = parse_kb_value(rest);
        } else if let Some(rest) = line.strip_prefix("MemAvailable:") {
            available = parse_kb_value(rest);
        }
    }

    let total_kb = total?;
    let available_kb = available.or(free)?;
    let free_kb = free.unwrap_or(available_kb);
    let used_kb = total_kb.saturating_sub(available_kb);

    Some(MemInfo { total_kb, free_kb, available_kb, used_kb })
}

/// Extract the numeric KB value from a meminfo line remainder like "  16384000 kB".
fn parse_kb_value(s: &str) -> Option<u64> {
    s.trim().split_whitespace().next()?.parse().ok()
}

/// Read and parse `/proc/meminfo` from the live system.
pub fn read_meminfo() -> Option<MemInfo> {
    let content = std::fs::read_to_string("/proc/meminfo").ok()?;
    parse_meminfo(&content)
}

// ── Pressure levels ─────────────────────────────────────────────────────────

/// System memory pressure classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PressureLevel {
    /// Available > 30% of total.
    Normal,
    /// Available 10-30% of total.
    Warning,
    /// Available < 10% of total.
    Critical,
}

impl PressureLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            PressureLevel::Normal   => "normal",
            PressureLevel::Warning  => "warning",
            PressureLevel::Critical => "critical",
        }
    }
}

/// Compute the pressure level from a `MemInfo`.
pub fn pressure_level(info: &MemInfo) -> PressureLevel {
    if info.total_kb == 0 {
        return PressureLevel::Normal;
    }
    // Percentage of total memory that is available (0-100).
    let pct = (info.available_kb * 100) / info.total_kb;
    if pct < 10 {
        PressureLevel::Critical
    } else if pct < 30 {
        PressureLevel::Warning
    } else {
        PressureLevel::Normal
    }
}

// ── Per-app memory tracking ─────────────────────────────────────────────────

/// Read VmRSS (Resident Set Size) for a given PID from `/proc/<pid>/status`.
pub fn app_memory_kb(pid: u32) -> Option<u64> {
    let path = format!("/proc/{pid}/status");
    let content = std::fs::read_to_string(path).ok()?;
    parse_vmrss(&content)
}

/// Parse VmRSS from `/proc/<pid>/status` content.
pub fn parse_vmrss(content: &str) -> Option<u64> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return parse_kb_value(rest);
        }
    }
    None
}

// ── IPC command handlers ────────────────────────────────────────────────────

/// Handle memory-related @supervisor IPC commands.
/// Returns `true` if the command was handled.
pub fn handle_memory_command(
    verb: &str,
    sender: &str,
    inbox: &Inbox,
    app_registry: &AppRegistry,
) -> bool {
    match verb {
        "memory-info" => {
            let reply = match read_meminfo() {
                Some(info) => {
                    let level = pressure_level(&info);
                    format!(
                        "REPLY:memory total={}KB free={}KB available={}KB used={}KB pressure={}",
                        info.total_kb, info.free_kb, info.available_kb, info.used_kb,
                        level.as_str(),
                    )
                }
                None => "REPLY:memory error: cannot read /proc/meminfo".to_string(),
            };
            crate::send_reply(sender, &reply, inbox);
        }
        "memory-apps" => {
            let reg = app_registry.lock().unwrap();
            let mut lines: Vec<String> = Vec::new();
            for (name, state_arc) in reg.iter() {
                let st = state_arc.lock().unwrap();
                if !matches!(st.status, AppStatus::Running) {
                    continue;
                }
                let rss = st.child_pid
                    .and_then(app_memory_kb)
                    .map(|kb| format!("{kb}KB"))
                    .unwrap_or_else(|| "n/a".to_string());
                lines.push(format!("{name}={rss}"));
            }
            lines.sort();
            if lines.is_empty() {
                crate::send_reply(sender, "REPLY:memory-apps: no running apps", inbox);
            } else {
                crate::send_reply(sender, &format!("REPLY:{}", lines.join("|")), inbox);
            }
        }
        "memory-pressure" => {
            let reply = match read_meminfo() {
                Some(info) => {
                    let level = pressure_level(&info);
                    format!("REPLY:memory-pressure {}", level.as_str())
                }
                None => "REPLY:memory-pressure error: cannot read /proc/meminfo".to_string(),
            };
            crate::send_reply(sender, &reply, inbox);
        }
        _ => return false,
    }
    true
}

// ── Pressure monitor thread ─────────────────────────────────────────────────

/// Shared state for the pressure level so the monitor can detect transitions.
static CURRENT_LEVEL: std::sync::OnceLock<Mutex<PressureLevel>> = std::sync::OnceLock::new();

fn current_level_lock() -> &'static Mutex<PressureLevel> {
    CURRENT_LEVEL.get_or_init(|| Mutex::new(PressureLevel::Normal))
}

/// Spawn the memory pressure monitor thread. Checks every 5 seconds and
/// broadcasts pressure events to all apps via the inbox.
pub fn spawn_pressure_monitor(inbox: &Inbox, app_registry: &AppRegistry) {
    let inbox = Arc::clone(inbox);
    let registry = Arc::clone(app_registry);
    thread::Builder::new()
        .name("memory-pressure".into())
        .spawn(move || run_pressure_monitor(&inbox, &registry))
        .expect("spawn memory-pressure thread");
}

fn run_pressure_monitor(inbox: &Inbox, registry: &AppRegistry) {
    loop {
        thread::sleep(Duration::from_secs(5));

        let info = match read_meminfo() {
            Some(i) => i,
            None => continue,
        };

        let new_level = pressure_level(&info);
        let prev_level = {
            let mut lock = current_level_lock().lock().unwrap();
            let prev = *lock;
            *lock = new_level;
            prev
        };

        // Only act on transitions.
        if new_level == prev_level {
            continue;
        }

        log_info!(Subsystem::Lifecycle, None,
            "memory pressure: {} -> {} (available={}KB/{}KB, {}%)",
            prev_level.as_str(), new_level.as_str(),
            info.available_kb, info.total_kb,
            if info.total_kb > 0 { info.available_kb * 100 / info.total_kb } else { 0 });

        match new_level {
            PressureLevel::Normal => {
                // Pressure eased — no broadcast needed.
            }
            PressureLevel::Warning => {
                broadcast_pressure(inbox, registry, "warning");
            }
            PressureLevel::Critical => {
                broadcast_pressure(inbox, registry, "critical");
                kill_lowest_priority_bg_app(registry);
            }
        }
    }
}

/// Broadcast `VYOMA_SYSTEM:memory-pressure:<level>` to all running apps.
fn broadcast_pressure(inbox: &Inbox, registry: &AppRegistry, level: &str) {
    let msg = format!("VYOMA_SYSTEM:memory-pressure:{level}");
    let reg = registry.lock().unwrap();
    let inb = inbox.lock().unwrap();
    for (name, state_arc) in reg.iter() {
        let st = state_arc.lock().unwrap();
        if matches!(st.status, AppStatus::Running) {
            if let Some(tx) = inb.get(name) {
                let _ = tx.send(msg.clone());
            }
        }
    }
}

/// Kill the lowest-priority running background app under critical memory pressure.
/// "Lowest priority" = background app with the highest restart_count (most restarts
/// means least stable), falling back to alphabetical ordering as a tiebreaker.
fn kill_lowest_priority_bg_app(registry: &AppRegistry) {
    let reg = registry.lock().unwrap();
    let mut candidates: Vec<(String, u32, Option<u32>)> = Vec::new();

    for (name, state_arc) in reg.iter() {
        let st = state_arc.lock().unwrap();
        if !st.is_background || !matches!(st.status, AppStatus::Running) {
            continue;
        }
        candidates.push((name.clone(), st.restart_count, st.child_pid));
    }

    // Sort: highest restart_count first, then alphabetical name as tiebreaker.
    candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));

    if let Some((name, _restarts, Some(pid))) = candidates.first() {
        log_warn!(Subsystem::Lifecycle, Some(name.as_str()),
            "critical memory pressure — killing background app (pid {pid})");
        #[cfg(target_os = "linux")]
        unsafe {
            libc::kill(*pid as libc::pid_t, libc::SIGKILL);
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_MEMINFO: &str = "\
MemTotal:       16384000 kB
MemFree:         4096000 kB
MemAvailable:    8192000 kB
Buffers:          512000 kB
Cached:          2048000 kB
SwapCached:            0 kB
Active:          6000000 kB
Inactive:        3000000 kB
";

    #[test]
    fn parse_meminfo_basic() {
        let info = parse_meminfo(SAMPLE_MEMINFO).unwrap();
        assert_eq!(info.total_kb, 16_384_000);
        assert_eq!(info.free_kb, 4_096_000);
        assert_eq!(info.available_kb, 8_192_000);
        assert_eq!(info.used_kb, 16_384_000 - 8_192_000);
    }

    #[test]
    fn parse_meminfo_missing_available_falls_back_to_free() {
        let content = "MemTotal:  1024 kB\nMemFree:   512 kB\n";
        let info = parse_meminfo(content).unwrap();
        assert_eq!(info.available_kb, 512);
        assert_eq!(info.used_kb, 512);
    }

    #[test]
    fn parse_meminfo_empty_returns_none() {
        assert!(parse_meminfo("").is_none());
    }

    #[test]
    fn pressure_level_normal() {
        let info = MemInfo { total_kb: 1000, free_kb: 400, available_kb: 400, used_kb: 600 };
        assert_eq!(pressure_level(&info), PressureLevel::Normal);
    }

    #[test]
    fn pressure_level_warning_at_20_pct() {
        let info = MemInfo { total_kb: 1000, free_kb: 200, available_kb: 200, used_kb: 800 };
        assert_eq!(pressure_level(&info), PressureLevel::Warning);
    }

    #[test]
    fn pressure_level_warning_boundary_30_pct() {
        // 29% should be Warning (< 30).
        let info = MemInfo { total_kb: 1000, free_kb: 290, available_kb: 290, used_kb: 710 };
        assert_eq!(pressure_level(&info), PressureLevel::Warning);
    }

    #[test]
    fn pressure_level_normal_boundary_30_pct() {
        // Exactly 30% should be Normal (>= 30).
        let info = MemInfo { total_kb: 1000, free_kb: 300, available_kb: 300, used_kb: 700 };
        assert_eq!(pressure_level(&info), PressureLevel::Normal);
    }

    #[test]
    fn pressure_level_critical_at_5_pct() {
        let info = MemInfo { total_kb: 1000, free_kb: 50, available_kb: 50, used_kb: 950 };
        assert_eq!(pressure_level(&info), PressureLevel::Critical);
    }

    #[test]
    fn pressure_level_warning_boundary_10_pct() {
        // Exactly 10% should be Warning (>= 10, < 30).
        let info = MemInfo { total_kb: 1000, free_kb: 100, available_kb: 100, used_kb: 900 };
        assert_eq!(pressure_level(&info), PressureLevel::Warning);
    }

    #[test]
    fn pressure_level_critical_boundary_9_pct() {
        let info = MemInfo { total_kb: 1000, free_kb: 90, available_kb: 90, used_kb: 910 };
        assert_eq!(pressure_level(&info), PressureLevel::Critical);
    }

    #[test]
    fn pressure_level_zero_total_is_normal() {
        let info = MemInfo { total_kb: 0, free_kb: 0, available_kb: 0, used_kb: 0 };
        assert_eq!(pressure_level(&info), PressureLevel::Normal);
    }

    #[test]
    fn parse_vmrss_basic() {
        let content = "\
Name:\twasmtime
VmPeak:\t 123456 kB
VmRSS:\t  65432 kB
VmSize:\t 234567 kB
";
        assert_eq!(parse_vmrss(content), Some(65432));
    }

    #[test]
    fn parse_vmrss_missing() {
        let content = "Name:\twasmtime\nVmPeak:\t 123456 kB\n";
        assert_eq!(parse_vmrss(content), None);
    }

    #[test]
    fn pressure_level_as_str() {
        assert_eq!(PressureLevel::Normal.as_str(), "normal");
        assert_eq!(PressureLevel::Warning.as_str(), "warning");
        assert_eq!(PressureLevel::Critical.as_str(), "critical");
    }
}
