// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P96: CPU governor/frequency management — reads CPU info from procfs/sysfs,
//! tracks usage, and exposes governor read/write plus IPC commands.

use crate::{log_info, log_warn, Inbox};
use supervisor::logging::Subsystem;

// ── CPU info from procfs/sysfs ──────────────────────────────────────────────

/// Parsed CPU information from `/proc/cpuinfo` and sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CpuInfo {
    pub cores: usize,
    pub model: String,
    pub freq_mhz: u32,
    pub governor: String,
}

/// Parse `/proc/cpuinfo` content to extract model name and core count.
pub fn parse_cpuinfo(content: &str) -> (String, usize) {
    let mut model = String::new();
    let mut core_count: usize = 0;

    for line in content.lines() {
        if model.is_empty() {
            if let Some(rest) = line.strip_prefix("model name") {
                if let Some(val) = rest.trim_start().strip_prefix(':') {
                    model = val.trim().to_string();
                }
            }
        }
        // Each "processor" line indicates a logical CPU.
        if line.starts_with("processor") {
            if let Some(rest) = line.strip_prefix("processor") {
                if rest.trim_start().starts_with(':') {
                    core_count += 1;
                }
            }
        }
    }

    if model.is_empty() {
        model = "unknown".to_string();
    }
    if core_count == 0 {
        core_count = 1;
    }

    (model, core_count)
}

/// Read the current CPU frequency in kHz from sysfs, return as MHz.
fn read_freq_mhz() -> u32 {
    let path = "/sys/devices/system/cpu/cpu0/cpufreq/scaling_cur_freq";
    match std::fs::read_to_string(path) {
        Ok(s) => s.trim().parse::<u64>().unwrap_or(0) as u32 / 1000,
        Err(_) => 0,
    }
}

/// Read full CPU info from the live system.
pub fn read_cpu_info() -> CpuInfo {
    let content = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let (model, cores) = parse_cpuinfo(&content);
    let freq_mhz = read_freq_mhz();
    let governor = get_governor();

    CpuInfo { cores, model, freq_mhz, governor }
}

// ── CPU governors (read/write sysfs) ────────────────────────────────────────

/// Supported CPU frequency governors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Governor {
    Performance,
    Powersave,
    Ondemand,
    Conservative,
}

impl Governor {
    pub fn as_str(self) -> &'static str {
        match self {
            Governor::Performance  => "performance",
            Governor::Powersave    => "powersave",
            Governor::Ondemand     => "ondemand",
            Governor::Conservative => "conservative",
        }
    }

    /// Parse a governor name string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "performance"  => Some(Governor::Performance),
            "powersave"    => Some(Governor::Powersave),
            "ondemand"     => Some(Governor::Ondemand),
            "conservative" => Some(Governor::Conservative),
            _ => None,
        }
    }
}

const GOVERNOR_SYSFS: &str =
    "/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor";

/// Read the current CPU governor from sysfs.
pub fn get_governor() -> String {
    match std::fs::read_to_string(GOVERNOR_SYSFS) {
        Ok(s) => s.trim().to_string(),
        Err(_) => "unknown".to_string(),
    }
}

/// Set the CPU governor via sysfs. May fail without root or if the governor
/// is not available on this kernel.
pub fn set_governor(gov: Governor) -> Result<(), String> {
    std::fs::write(GOVERNOR_SYSFS, gov.as_str())
        .map_err(|e| format!("cannot set governor: {e}"))
}

// ── Load average ────────────────────────────────────────────────────────────

/// Parse `/proc/loadavg` content into three f32 values (1, 5, 15 min).
pub fn parse_loadavg(content: &str) -> Option<(f32, f32, f32)> {
    let parts: Vec<&str> = content.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let a: f32 = parts[0].parse().ok()?;
    let b: f32 = parts[1].parse().ok()?;
    let c: f32 = parts[2].parse().ok()?;
    Some((a, b, c))
}

/// Read load average from the live system.
pub fn read_load_avg() -> (f32, f32, f32) {
    let content = std::fs::read_to_string("/proc/loadavg").unwrap_or_default();
    parse_loadavg(&content).unwrap_or((0.0, 0.0, 0.0))
}

// ── CPU usage tracking ──────────────────────────────────────────────────────

/// Parsed CPU time values from a single line of `/proc/stat`.
#[derive(Debug, Clone, Copy)]
pub struct CpuTimes {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
}

impl CpuTimes {
    /// Total active (non-idle) time.
    pub fn active(&self) -> u64 {
        self.user + self.nice + self.system + self.irq + self.softirq + self.steal
    }

    /// Total time (active + idle + iowait).
    pub fn total(&self) -> u64 {
        self.active() + self.idle + self.iowait
    }
}

/// Parse the aggregate `cpu` line from `/proc/stat` content.
///
/// Expected format:
/// ```text
/// cpu  12345 678 9012 345678 123 0 45 0 0 0
/// cpu0 ...
/// ```
pub fn parse_proc_stat(content: &str) -> Option<CpuTimes> {
    for line in content.lines() {
        // Match the aggregate "cpu " line (with trailing space), not "cpu0", "cpu1", etc.
        if let Some(rest) = line.strip_prefix("cpu ") {
            let vals: Vec<u64> = rest.split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            if vals.len() < 4 {
                return None;
            }
            return Some(CpuTimes {
                user:    vals[0],
                nice:    vals[1],
                system:  vals[2],
                idle:    vals[3],
                iowait:  vals.get(4).copied().unwrap_or(0),
                irq:     vals.get(5).copied().unwrap_or(0),
                softirq: vals.get(6).copied().unwrap_or(0),
                steal:   vals.get(7).copied().unwrap_or(0),
            });
        }
    }
    None
}

/// Compute CPU usage percentage from two snapshots taken at different times.
/// Returns a value in 0.0..100.0.
pub fn usage_between(prev: &CpuTimes, curr: &CpuTimes) -> f32 {
    let total_delta = curr.total().saturating_sub(prev.total());
    if total_delta == 0 {
        return 0.0;
    }
    let active_delta = curr.active().saturating_sub(prev.active());
    (active_delta as f64 / total_delta as f64 * 100.0) as f32
}

/// Read current CPU times from `/proc/stat`.
fn read_cpu_times() -> Option<CpuTimes> {
    let content = std::fs::read_to_string("/proc/stat").ok()?;
    parse_proc_stat(&content)
}

/// Compute instantaneous CPU usage by sampling `/proc/stat` twice with a
/// 200 ms sleep in between.
pub fn cpu_usage_percent() -> f32 {
    let t1 = match read_cpu_times() {
        Some(t) => t,
        None => return 0.0,
    };
    std::thread::sleep(std::time::Duration::from_millis(200));
    let t2 = match read_cpu_times() {
        Some(t) => t,
        None => return 0.0,
    };
    usage_between(&t1, &t2)
}

// ── IPC command handlers ────────────────────────────────────────────────────

/// Handle CPU-related `@supervisor:` IPC commands.
/// Returns `true` if the command was handled.
pub fn handle_cpu_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "cpu-info" => {
            let info = read_cpu_info();
            let (l1, l5, l15) = read_load_avg();
            let reply = format!(
                "REPLY:cpu model={} cores={} freq={}MHz governor={} load={:.2},{:.2},{:.2}",
                info.model, info.cores, info.freq_mhz, info.governor, l1, l5, l15,
            );
            log_info!(Subsystem::Ipc, None, "cpu-info query from {sender}");
            crate::send_reply(sender, &reply, inbox);
        }
        "cpu-usage" => {
            let pct = cpu_usage_percent();
            let reply = format!("REPLY:cpu-usage {:.1}%", pct);
            log_info!(Subsystem::Ipc, None, "cpu-usage query from {sender}: {pct:.1}%");
            crate::send_reply(sender, &reply, inbox);
        }
        "cpu-governor" => {
            let gov_str = parts.get(1).unwrap_or(&"").trim();
            if gov_str.is_empty() {
                let current = get_governor();
                crate::send_reply(
                    sender,
                    &format!("REPLY:cpu-governor current={current}"),
                    inbox,
                );
                return true;
            }
            match Governor::from_str(gov_str) {
                Some(gov) => {
                    match set_governor(gov) {
                        Ok(()) => {
                            log_info!(Subsystem::Ipc, None,
                                "cpu governor set to {} by {sender}", gov.as_str());
                            crate::send_reply(
                                sender,
                                &format!("REPLY:cpu-governor set to {}", gov.as_str()),
                                inbox,
                            );
                        }
                        Err(e) => {
                            log_warn!(Subsystem::Ipc, None,
                                "cpu governor set failed: {e}");
                            crate::send_reply(
                                sender,
                                &format!("REPLY:error: {e}"),
                                inbox,
                            );
                        }
                    }
                }
                None => {
                    crate::send_reply(
                        sender,
                        "REPLY:error: unknown governor (use: performance, powersave, ondemand, conservative)",
                        inbox,
                    );
                }
            }
        }
        _ => return false,
    }
    true
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_CPUINFO: &str = "\
processor\t: 0
vendor_id\t: GenuineIntel
model name\t: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz
cpu MHz\t\t: 2600.000
cache size\t: 12288 KB

processor\t: 1
vendor_id\t: GenuineIntel
model name\t: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz
cpu MHz\t\t: 2600.000
cache size\t: 12288 KB

processor\t: 2
vendor_id\t: GenuineIntel
model name\t: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz
cpu MHz\t\t: 2600.000

processor\t: 3
vendor_id\t: GenuineIntel
model name\t: Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz
cpu MHz\t\t: 2600.000
";

    #[test]
    fn parse_cpuinfo_model_and_cores() {
        let (model, cores) = parse_cpuinfo(SAMPLE_CPUINFO);
        assert_eq!(model, "Intel(R) Core(TM) i7-9750H CPU @ 2.60GHz");
        assert_eq!(cores, 4);
    }

    #[test]
    fn parse_cpuinfo_empty() {
        let (model, cores) = parse_cpuinfo("");
        assert_eq!(model, "unknown");
        assert_eq!(cores, 1);
    }

    #[test]
    fn parse_cpuinfo_single_core() {
        let content = "\
processor\t: 0
model name\t: QEMU Virtual CPU version 2.5+
";
        let (model, cores) = parse_cpuinfo(content);
        assert_eq!(model, "QEMU Virtual CPU version 2.5+");
        assert_eq!(cores, 1);
    }

    const SAMPLE_STAT: &str = "\
cpu  10132153 290696 3084719 46828483 16683 0 25195 0 0 0
cpu0 1393280 32966 572056 13343292 6130 0 17875 0 0 0
cpu1 1335498 28612 564581 13260498 3560 0 4386 0 0 0
";

    #[test]
    fn parse_proc_stat_basic() {
        let times = parse_proc_stat(SAMPLE_STAT).unwrap();
        assert_eq!(times.user, 10132153);
        assert_eq!(times.nice, 290696);
        assert_eq!(times.system, 3084719);
        assert_eq!(times.idle, 46828483);
        assert_eq!(times.iowait, 16683);
        assert_eq!(times.irq, 0);
        assert_eq!(times.softirq, 25195);
        assert_eq!(times.steal, 0);
    }

    #[test]
    fn parse_proc_stat_empty() {
        assert!(parse_proc_stat("").is_none());
    }

    #[test]
    fn parse_proc_stat_minimal_4_fields() {
        let content = "cpu  100 50 30 820\ncpu0 50 25 15 410\n";
        let times = parse_proc_stat(content).unwrap();
        assert_eq!(times.user, 100);
        assert_eq!(times.idle, 820);
        assert_eq!(times.iowait, 0); // missing field defaults to 0
    }

    #[test]
    fn usage_between_50_percent() {
        let prev = CpuTimes {
            user: 100, nice: 0, system: 0, idle: 100,
            iowait: 0, irq: 0, softirq: 0, steal: 0,
        };
        let curr = CpuTimes {
            user: 200, nice: 0, system: 0, idle: 200,
            iowait: 0, irq: 0, softirq: 0, steal: 0,
        };
        let pct = usage_between(&prev, &curr);
        assert!((pct - 50.0).abs() < 0.1, "expected ~50%, got {pct}");
    }

    #[test]
    fn usage_between_zero_delta() {
        let t = CpuTimes {
            user: 100, nice: 0, system: 0, idle: 100,
            iowait: 0, irq: 0, softirq: 0, steal: 0,
        };
        assert_eq!(usage_between(&t, &t), 0.0);
    }

    #[test]
    fn usage_between_100_percent() {
        let prev = CpuTimes {
            user: 100, nice: 0, system: 0, idle: 100,
            iowait: 0, irq: 0, softirq: 0, steal: 0,
        };
        let curr = CpuTimes {
            user: 300, nice: 0, system: 0, idle: 100,
            iowait: 0, irq: 0, softirq: 0, steal: 0,
        };
        let pct = usage_between(&prev, &curr);
        assert!((pct - 100.0).abs() < 0.1, "expected ~100%, got {pct}");
    }

    #[test]
    fn parse_loadavg_basic() {
        let (a, b, c) = parse_loadavg("0.42 0.38 0.35 1/234 5678").unwrap();
        assert!((a - 0.42).abs() < 0.001);
        assert!((b - 0.38).abs() < 0.001);
        assert!((c - 0.35).abs() < 0.001);
    }

    #[test]
    fn parse_loadavg_empty() {
        assert!(parse_loadavg("").is_none());
    }

    #[test]
    fn parse_loadavg_too_few_fields() {
        assert!(parse_loadavg("0.42 0.38").is_none());
    }

    #[test]
    fn governor_from_str_valid() {
        assert_eq!(Governor::from_str("performance"), Some(Governor::Performance));
        assert_eq!(Governor::from_str("Powersave"), Some(Governor::Powersave));
        assert_eq!(Governor::from_str("ONDEMAND"), Some(Governor::Ondemand));
        assert_eq!(Governor::from_str("conservative"), Some(Governor::Conservative));
    }

    #[test]
    fn governor_from_str_invalid() {
        assert_eq!(Governor::from_str("turbo"), None);
        assert_eq!(Governor::from_str(""), None);
    }

    #[test]
    fn governor_as_str_roundtrip() {
        for gov in [Governor::Performance, Governor::Powersave, Governor::Ondemand, Governor::Conservative] {
            assert_eq!(Governor::from_str(gov.as_str()), Some(gov));
        }
    }

    #[test]
    fn cpu_times_active_and_total() {
        let t = CpuTimes {
            user: 100, nice: 10, system: 50, idle: 500,
            iowait: 20, irq: 5, softirq: 3, steal: 2,
        };
        assert_eq!(t.active(), 100 + 10 + 50 + 5 + 3 + 2);
        assert_eq!(t.total(), 100 + 10 + 50 + 500 + 20 + 5 + 3 + 2);
    }
}
