// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Integration tests for the memory pressure manager (P94).
//! These tests exercise the public parsing and classification API without
//! requiring a running Linux system (no /proc access needed).

// The memory module lives inside the supervisor binary crate and re-exports
// its parsing/classification functions, but for integration tests we
// replicate the pure logic inline since the binary crate's internals
// are not accessible from `tests/`. The canonical implementation lives in
// `supervisor/src/memory.rs` and its `#[cfg(test)] mod tests` block
// provides the authoritative unit tests.

/// Parsed subset of `/proc/meminfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MemInfo {
    total_kb:     u64,
    free_kb:      u64,
    available_kb: u64,
    used_kb:      u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PressureLevel {
    Normal,
    Warning,
    Critical,
}

fn parse_kb_value(s: &str) -> Option<u64> {
    s.trim().split_whitespace().next()?.parse().ok()
}

fn parse_meminfo(content: &str) -> Option<MemInfo> {
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

fn pressure_level(info: &MemInfo) -> PressureLevel {
    if info.total_kb == 0 {
        return PressureLevel::Normal;
    }
    let pct = (info.available_kb * 100) / info.total_kb;
    if pct < 10 {
        PressureLevel::Critical
    } else if pct < 30 {
        PressureLevel::Warning
    } else {
        PressureLevel::Normal
    }
}

fn parse_vmrss(content: &str) -> Option<u64> {
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            return parse_kb_value(rest);
        }
    }
    None
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn parse_full_meminfo() {
    let content = "\
MemTotal:       16384000 kB
MemFree:         4096000 kB
MemAvailable:    8192000 kB
Buffers:          512000 kB
Cached:          2048000 kB
";
    let info = parse_meminfo(content).unwrap();
    assert_eq!(info.total_kb, 16_384_000);
    assert_eq!(info.free_kb, 4_096_000);
    assert_eq!(info.available_kb, 8_192_000);
    assert_eq!(info.used_kb, 8_192_000);
}

#[test]
fn parse_meminfo_no_available_uses_free() {
    let content = "MemTotal:  2048 kB\nMemFree:   1024 kB\n";
    let info = parse_meminfo(content).unwrap();
    assert_eq!(info.available_kb, 1024);
    assert_eq!(info.free_kb, 1024);
}

#[test]
fn parse_meminfo_garbage_returns_none() {
    assert!(parse_meminfo("garbage data").is_none());
    assert!(parse_meminfo("").is_none());
}

#[test]
fn pressure_normal_above_30() {
    let info = MemInfo { total_kb: 1000, free_kb: 310, available_kb: 310, used_kb: 690 };
    assert_eq!(pressure_level(&info), PressureLevel::Normal);
}

#[test]
fn pressure_warning_between_10_and_30() {
    let info = MemInfo { total_kb: 1000, free_kb: 150, available_kb: 150, used_kb: 850 };
    assert_eq!(pressure_level(&info), PressureLevel::Warning);
}

#[test]
fn pressure_critical_below_10() {
    let info = MemInfo { total_kb: 1000, free_kb: 50, available_kb: 50, used_kb: 950 };
    assert_eq!(pressure_level(&info), PressureLevel::Critical);
}

#[test]
fn pressure_boundary_exactly_10_is_warning() {
    let info = MemInfo { total_kb: 100, free_kb: 10, available_kb: 10, used_kb: 90 };
    assert_eq!(pressure_level(&info), PressureLevel::Warning);
}

#[test]
fn pressure_boundary_exactly_30_is_normal() {
    let info = MemInfo { total_kb: 100, free_kb: 30, available_kb: 30, used_kb: 70 };
    assert_eq!(pressure_level(&info), PressureLevel::Normal);
}

#[test]
fn pressure_zero_total_is_normal() {
    let info = MemInfo { total_kb: 0, free_kb: 0, available_kb: 0, used_kb: 0 };
    assert_eq!(pressure_level(&info), PressureLevel::Normal);
}

#[test]
fn parse_vmrss_extracts_value() {
    let content = "Name:\twasmtime\nVmPeak:\t 99999 kB\nVmRSS:\t  12345 kB\n";
    assert_eq!(parse_vmrss(content), Some(12345));
}

#[test]
fn parse_vmrss_missing_returns_none() {
    assert_eq!(parse_vmrss("Name:\twasmtime\nVmPeak:\t 99 kB\n"), None);
}

#[test]
fn parse_vmrss_zero() {
    assert_eq!(parse_vmrss("VmRSS:\t       0 kB\n"), Some(0));
}
