// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P104: ACPI table reading and power event handling — reads sysfs ACPI tables,
//! monitors thermal zones, models ACPI events, and provides IPC commands for
//! querying ACPI/thermal state.

use crate::{log_info, log_warn, AppRegistry, AppStatus, Inbox};
use supervisor::logging::Subsystem;

// ── ACPI table discovery ────────────────────────────────────────────────────

/// Metadata about a single ACPI table found in sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcpiTable {
    pub name: String,
    pub size: u64,
}

/// Read `/sys/firmware/acpi/tables/` and return metadata for each table file.
pub fn list_acpi_tables() -> Vec<AcpiTable> {
    list_acpi_tables_from("/sys/firmware/acpi/tables")
}

/// Read ACPI tables from a given directory (testable).
pub fn list_acpi_tables_from(dir: &str) -> Vec<AcpiTable> {
    let path = std::path::Path::new(dir);
    if !path.is_dir() {
        return Vec::new();
    }
    let mut tables = Vec::new();
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            // Skip subdirectories (e.g. dynamic/ on some kernels).
            if meta.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            tables.push(AcpiTable {
                name,
                size: meta.len(),
            });
        }
    }
    tables.sort_by(|a, b| a.name.cmp(&b.name));
    tables
}

// ── Thermal zone monitoring ─────────────────────────────────────────────────

/// Parsed thermal zone from sysfs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThermalZone {
    /// Sysfs directory name, e.g. "thermal_zone0".
    pub name: String,
    /// Temperature in milliCelsius (e.g. 45000 = 45.0 C).
    pub temp_mc: i32,
    /// Type string reported by the zone, e.g. "x86_pkg_temp".
    pub type_name: String,
}

/// Read all thermal zones from `/sys/class/thermal/thermal_zone*/`.
pub fn read_thermal_zones() -> Vec<ThermalZone> {
    read_thermal_zones_from("/sys/class/thermal")
}

/// Read thermal zones from a given base directory (testable).
pub fn read_thermal_zones_from(base: &str) -> Vec<ThermalZone> {
    let base_path = std::path::Path::new(base);
    if !base_path.is_dir() {
        return Vec::new();
    }
    let mut zones = Vec::new();
    if let Ok(entries) = std::fs::read_dir(base_path) {
        for entry in entries.flatten() {
            let fname = entry.file_name().to_string_lossy().to_string();
            if !fname.starts_with("thermal_zone") {
                continue;
            }
            let dir = entry.path();
            let temp_mc = read_sysfs_i32(&dir.join("temp")).unwrap_or(0);
            let type_name = read_sysfs_trimmed(&dir.join("type"))
                .unwrap_or_else(|| "unknown".to_string());
            zones.push(ThermalZone {
                name: fname,
                temp_mc,
                type_name,
            });
        }
    }
    zones.sort_by(|a, b| a.name.cmp(&b.name));
    zones
}

/// Convenience: return the temperature of the first thermal zone in Celsius.
pub fn current_temp_celsius() -> Option<f32> {
    let zones = read_thermal_zones();
    zones.first().map(|z| z.temp_mc as f32 / 1000.0)
}

/// Parse temperature from a milliCelsius value into Celsius.
pub fn mc_to_celsius(temp_mc: i32) -> f32 {
    temp_mc as f32 / 1000.0
}

// ── sysfs helpers ───────────────────────────────────────────────────────────

fn read_sysfs_trimmed(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn read_sysfs_i32(path: &std::path::Path) -> Option<i32> {
    read_sysfs_trimmed(path).and_then(|s| s.parse::<i32>().ok())
}

// ── ACPI event model ────────────────────────────────────────────────────────

/// Modeled ACPI events that the supervisor can react to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpiEvent {
    PowerButton,
    SleepButton,
    LidClose,
    LidOpen,
    ThermalTrip(String),
}

impl AcpiEvent {
    /// Human-readable label for the event.
    pub fn as_str(&self) -> &str {
        match self {
            AcpiEvent::PowerButton => "power-button",
            AcpiEvent::SleepButton => "sleep-button",
            AcpiEvent::LidClose => "lid-close",
            AcpiEvent::LidOpen => "lid-open",
            AcpiEvent::ThermalTrip(_) => "thermal-trip",
        }
    }

    /// Parse an ACPI event string (e.g. from /proc/acpi/event or acpid).
    pub fn parse(s: &str) -> Option<Self> {
        let lower = s.trim().to_lowercase();
        if lower.contains("power") && lower.contains("button") {
            Some(AcpiEvent::PowerButton)
        } else if lower.contains("sleep") && lower.contains("button") {
            Some(AcpiEvent::SleepButton)
        } else if lower.contains("lid") && lower.contains("close") {
            Some(AcpiEvent::LidClose)
        } else if lower.contains("lid") && lower.contains("open") {
            Some(AcpiEvent::LidOpen)
        } else if lower.contains("thermal") {
            let zone = lower.split_whitespace().last().unwrap_or("unknown").to_string();
            Some(AcpiEvent::ThermalTrip(zone))
        } else {
            None
        }
    }
}

// ── Thermal thresholds ──────────────────────────────────────────────────────

const THERMAL_WARNING_C: f32 = 85.0;
const THERMAL_CRITICAL_C: f32 = 95.0;

/// Thermal severity classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalLevel {
    Normal,
    Warning,
    Critical,
}

/// Classify a temperature (in Celsius) into a thermal level.
pub fn classify_thermal(temp_c: f32) -> ThermalLevel {
    if temp_c >= THERMAL_CRITICAL_C {
        ThermalLevel::Critical
    } else if temp_c >= THERMAL_WARNING_C {
        ThermalLevel::Warning
    } else {
        ThermalLevel::Normal
    }
}

/// Check all thermal zones and broadcast warnings/critical events.
/// Returns the worst thermal level encountered.
pub fn check_thermal_zones(inbox: &Inbox, registry: &AppRegistry) -> ThermalLevel {
    let zones = read_thermal_zones();
    let mut worst = ThermalLevel::Normal;
    for zone in &zones {
        let temp_c = mc_to_celsius(zone.temp_mc);
        let level = classify_thermal(temp_c);
        match level {
            ThermalLevel::Warning if worst == ThermalLevel::Normal => {
                worst = ThermalLevel::Warning;
                log_warn!(Subsystem::Lifecycle, None,
                    "thermal warning: {} at {:.1}C (>{THERMAL_WARNING_C}C)",
                    zone.type_name, temp_c);
                broadcast_thermal_event(inbox, registry, "warning");
            }
            ThermalLevel::Critical => {
                worst = ThermalLevel::Critical;
                log_warn!(Subsystem::Lifecycle, None,
                    "thermal CRITICAL: {} at {:.1}C (>{THERMAL_CRITICAL_C}C) — throttle advised",
                    zone.type_name, temp_c);
                broadcast_thermal_event(inbox, registry, "critical");
            }
            _ => {}
        }
    }
    worst
}

/// Broadcast `VYOMA_SYSTEM:thermal:<level>` to all running apps.
fn broadcast_thermal_event(inbox: &Inbox, registry: &AppRegistry, level: &str) {
    let msg = format!("VYOMA_SYSTEM:thermal:{level}");
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

// ── IPC command handlers ────────────────────────────────────────────────────

/// Handle ACPI-related IPC commands. Returns `true` if handled.
pub fn handle_acpi_command(
    verb: &str,
    _parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "acpi-tables" => {
            let tables = list_acpi_tables();
            if tables.is_empty() {
                crate::send_reply(sender, "REPLY:acpi-tables: none found", inbox);
            } else {
                let lines: Vec<String> = tables
                    .iter()
                    .map(|t| format!("  {} ({} bytes)", t.name, t.size))
                    .collect();
                let reply = format!(
                    "REPLY:acpi-tables ({} tables):\n{}",
                    tables.len(),
                    lines.join("\n")
                );
                crate::send_reply(sender, &reply, inbox);
            }
        }
        "acpi-thermal" => {
            let zones = read_thermal_zones();
            if zones.is_empty() {
                crate::send_reply(sender, "REPLY:acpi-thermal: no thermal zones", inbox);
            } else {
                let lines: Vec<String> = zones
                    .iter()
                    .map(|z| {
                        let temp_c = mc_to_celsius(z.temp_mc);
                        let level = classify_thermal(temp_c);
                        let tag = match level {
                            ThermalLevel::Normal => "",
                            ThermalLevel::Warning => " [WARNING]",
                            ThermalLevel::Critical => " [CRITICAL]",
                        };
                        format!("  {} ({}): {:.1}C{}", z.name, z.type_name, temp_c, tag)
                    })
                    .collect();
                let reply = format!(
                    "REPLY:acpi-thermal ({} zones):\n{}",
                    zones.len(),
                    lines.join("\n")
                );
                crate::send_reply(sender, &reply, inbox);
            }
        }
        "acpi-info" => {
            let tables = list_acpi_tables();
            let zones = read_thermal_zones();
            let temp_str = match current_temp_celsius() {
                Some(c) => format!("{:.1}C", c),
                None => "N/A".to_string(),
            };
            let reply = format!(
                "REPLY:acpi-info: tables={} thermal_zones={} current_temp={}",
                tables.len(),
                zones.len(),
                temp_str,
            );
            crate::send_reply(sender, &reply, inbox);
        }
        _ => return false,
    }
    true
}

// ── Thermal monitor thread ──────────────────────────────────────────────────

/// Spawn a background thread that monitors thermal zones every 10 seconds.
pub fn spawn_thermal_monitor(inbox: &Inbox, app_registry: &AppRegistry) {
    let inbox = std::sync::Arc::clone(inbox);
    let registry = std::sync::Arc::clone(app_registry);
    std::thread::Builder::new()
        .name("thermal-monitor".into())
        .spawn(move || run_thermal_monitor(&inbox, &registry))
        .expect("spawn thermal-monitor thread");
}

fn run_thermal_monitor(inbox: &Inbox, registry: &AppRegistry) {
    loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
        let _ = check_thermal_zones(inbox, registry);
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mc_to_celsius_conversion() {
        assert!((mc_to_celsius(45000) - 45.0).abs() < 0.01);
        assert!((mc_to_celsius(0) - 0.0).abs() < 0.01);
        assert!((mc_to_celsius(-5000) - (-5.0)).abs() < 0.01);
        assert!((mc_to_celsius(100500) - 100.5).abs() < 0.01);
    }

    #[test]
    fn classify_thermal_levels() {
        assert_eq!(classify_thermal(25.0), ThermalLevel::Normal);
        assert_eq!(classify_thermal(84.9), ThermalLevel::Normal);
        assert_eq!(classify_thermal(85.0), ThermalLevel::Warning);
        assert_eq!(classify_thermal(90.0), ThermalLevel::Warning);
        assert_eq!(classify_thermal(94.9), ThermalLevel::Warning);
        assert_eq!(classify_thermal(95.0), ThermalLevel::Critical);
        assert_eq!(classify_thermal(120.0), ThermalLevel::Critical);
    }

    #[test]
    fn acpi_event_parse_power_button() {
        let ev = AcpiEvent::parse("button/power PBTN 00000080 00000000");
        assert_eq!(ev, Some(AcpiEvent::PowerButton));
    }

    #[test]
    fn acpi_event_parse_sleep_button() {
        let ev = AcpiEvent::parse("button/sleep SLPB 00000080 00000000");
        assert_eq!(ev, Some(AcpiEvent::SleepButton));
    }

    #[test]
    fn acpi_event_parse_lid() {
        assert_eq!(
            AcpiEvent::parse("button/lid LID close"),
            Some(AcpiEvent::LidClose)
        );
        assert_eq!(
            AcpiEvent::parse("button/lid LID open"),
            Some(AcpiEvent::LidOpen)
        );
    }

    #[test]
    fn acpi_event_parse_thermal() {
        let ev = AcpiEvent::parse("thermal zone0 trip");
        assert!(matches!(ev, Some(AcpiEvent::ThermalTrip(_))));
    }

    #[test]
    fn acpi_event_parse_unknown() {
        assert_eq!(AcpiEvent::parse("some random text"), None);
    }

    #[test]
    fn acpi_event_as_str() {
        assert_eq!(AcpiEvent::PowerButton.as_str(), "power-button");
        assert_eq!(AcpiEvent::SleepButton.as_str(), "sleep-button");
        assert_eq!(AcpiEvent::LidClose.as_str(), "lid-close");
        assert_eq!(AcpiEvent::LidOpen.as_str(), "lid-open");
        assert_eq!(
            AcpiEvent::ThermalTrip("zone0".to_string()).as_str(),
            "thermal-trip"
        );
    }

    #[test]
    fn list_tables_nonexistent_dir() {
        let tables = list_acpi_tables_from("/nonexistent/acpi/tables");
        assert!(tables.is_empty());
    }

    #[test]
    fn read_zones_nonexistent_dir() {
        let zones = read_thermal_zones_from("/nonexistent/thermal");
        assert!(zones.is_empty());
    }

    #[test]
    fn list_tables_from_tmpdir() {
        let dir = std::env::temp_dir().join("vyoma_acpi_test_tables");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create fake table files.
        std::fs::write(dir.join("DSDT"), &[0u8; 128]).unwrap();
        std::fs::write(dir.join("FACP"), &[0u8; 64]).unwrap();
        std::fs::write(dir.join("APIC"), &[0u8; 32]).unwrap();

        let tables = list_acpi_tables_from(dir.to_str().unwrap());
        assert_eq!(tables.len(), 3);
        assert_eq!(tables[0].name, "APIC");
        assert_eq!(tables[0].size, 32);
        assert_eq!(tables[1].name, "DSDT");
        assert_eq!(tables[1].size, 128);
        assert_eq!(tables[2].name, "FACP");
        assert_eq!(tables[2].size, 64);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_zones_from_tmpdir() {
        let dir = std::env::temp_dir().join("vyoma_acpi_test_thermal");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create fake thermal_zone0.
        let zone0 = dir.join("thermal_zone0");
        std::fs::create_dir_all(&zone0).unwrap();
        std::fs::write(zone0.join("temp"), "45000\n").unwrap();
        std::fs::write(zone0.join("type"), "x86_pkg_temp\n").unwrap();

        // Create fake thermal_zone1.
        let zone1 = dir.join("thermal_zone1");
        std::fs::create_dir_all(&zone1).unwrap();
        std::fs::write(zone1.join("temp"), "38500\n").unwrap();
        std::fs::write(zone1.join("type"), "acpitz\n").unwrap();

        // Non-thermal directory should be ignored.
        let other = dir.join("cooling_device0");
        std::fs::create_dir_all(&other).unwrap();

        let zones = read_thermal_zones_from(dir.to_str().unwrap());
        assert_eq!(zones.len(), 2);
        assert_eq!(zones[0].name, "thermal_zone0");
        assert_eq!(zones[0].temp_mc, 45000);
        assert_eq!(zones[0].type_name, "x86_pkg_temp");
        assert_eq!(zones[1].name, "thermal_zone1");
        assert_eq!(zones[1].temp_mc, 38500);
        assert_eq!(zones[1].type_name, "acpitz");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn thermal_zone_missing_temp_defaults_zero() {
        let dir = std::env::temp_dir().join("vyoma_acpi_test_thermal_nofile");
        let _ = std::fs::remove_dir_all(&dir);
        let zone = dir.join("thermal_zone0");
        std::fs::create_dir_all(&zone).unwrap();
        // No temp file, only type.
        std::fs::write(zone.join("type"), "test_zone\n").unwrap();

        let zones = read_thermal_zones_from(dir.to_str().unwrap());
        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].temp_mc, 0);
        assert_eq!(zones[0].type_name, "test_zone");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
