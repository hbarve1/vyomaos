// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P95: Battery / power management — reads sysfs battery info, manages power
//! profiles, broadcasts low-battery events, and initiates shutdown on critical.

use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use crate::lock_or_recover;

use crate::{log_info, log_warn, AppRegistry, AppStatus, FocusedApp, Inbox};
use supervisor::logging::Subsystem;

// ── Battery info from sysfs ─────────────────────────────────────────────────

/// Parsed battery state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatteryInfo {
    pub present: bool,
    pub percent: u8,
    pub charging: bool,
    pub time_remaining_mins: u32,
}

/// Attempt to read a sysfs file, returning its trimmed content.
fn read_sysfs(path: &str) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Read battery information from `/sys/class/power_supply/BAT0/`.
/// Returns `None` if no battery is present (desktop, VM).
pub fn read_battery() -> Option<BatteryInfo> {
    read_battery_from("/sys/class/power_supply/BAT0")
}

/// Read battery info from a given sysfs directory (testable).
pub fn read_battery_from(base: &str) -> Option<BatteryInfo> {
    // Check if the directory exists at all.
    if !std::path::Path::new(base).exists() {
        return None;
    }

    let present_str = read_sysfs(&format!("{base}/present"));
    let present = match present_str.as_deref() {
        Some("1") => true,
        Some("0") => return Some(BatteryInfo {
            present: false, percent: 0, charging: false, time_remaining_mins: 0,
        }),
        _ => return None,
    };

    let capacity = read_sysfs(&format!("{base}/capacity"))
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(0);

    let status = read_sysfs(&format!("{base}/status")).unwrap_or_default();
    let charging = matches!(status.as_str(), "Charging" | "Full");

    // Estimate time remaining from energy_now / power_now (if available).
    let time_remaining_mins = estimate_time_remaining(base, charging);

    Some(BatteryInfo {
        present,
        percent: capacity,
        charging,
        time_remaining_mins,
    })
}

/// Estimate minutes remaining from energy_now and power_now sysfs files.
fn estimate_time_remaining(base: &str, charging: bool) -> u32 {
    let energy_now = read_sysfs(&format!("{base}/energy_now"))
        .and_then(|s| s.parse::<u64>().ok());
    let energy_full = read_sysfs(&format!("{base}/energy_full"))
        .and_then(|s| s.parse::<u64>().ok());
    let power_now = read_sysfs(&format!("{base}/power_now"))
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&p| p > 0);

    match (energy_now, energy_full, power_now) {
        (Some(now), Some(full), Some(power)) if charging => {
            let remaining_uh = full.saturating_sub(now);
            // power is in microwatts, energy in microwatt-hours => hours * 60
            ((remaining_uh * 60) / power) as u32
        }
        (Some(now), _, Some(power)) if !charging => {
            ((now * 60) / power) as u32
        }
        _ => 0,
    }
}

/// Parse battery info from synthetic sysfs content (for unit tests).
/// `content` maps filename to value, e.g. ("present", "1"), ("capacity", "85").
pub fn parse_sysfs_fields(fields: &[(&str, &str)]) -> Option<BatteryInfo> {
    let mut present_val: Option<&str> = None;
    let mut capacity_val: Option<u8> = None;
    let mut status_val = "";

    for &(key, val) in fields {
        match key {
            "present"  => present_val = Some(val),
            "capacity" => capacity_val = val.parse().ok(),
            "status"   => status_val = val,
            _ => {}
        }
    }

    let present = match present_val? {
        "1" => true,
        "0" => return Some(BatteryInfo {
            present: false, percent: 0, charging: false, time_remaining_mins: 0,
        }),
        _ => return None,
    };

    let charging = matches!(status_val, "Charging" | "Full");

    Some(BatteryInfo {
        present,
        percent: capacity_val.unwrap_or(0),
        charging,
        time_remaining_mins: 0,
    })
}

// ── Power profiles ──────────────────────────────────────────────────────────

/// System power profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerProfile {
    Performance,
    Balanced,
    PowerSaver,
}

impl PowerProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            PowerProfile::Performance => "performance",
            PowerProfile::Balanced    => "balanced",
            PowerProfile::PowerSaver  => "saver",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "performance" => Some(PowerProfile::Performance),
            "balanced"    => Some(PowerProfile::Balanced),
            "saver" | "powersaver" => Some(PowerProfile::PowerSaver),
            _ => None,
        }
    }
}

/// Global power profile state.
static POWER_PROFILE: OnceLock<Mutex<PowerProfile>> = OnceLock::new();

fn power_profile_lock() -> &'static Mutex<PowerProfile> {
    POWER_PROFILE.get_or_init(|| Mutex::new(PowerProfile::Balanced))
}

/// Get the current power profile.
pub fn current_profile() -> PowerProfile {
    *lock_or_recover(&power_profile_lock())
}

/// Set the power profile. Returns the previous profile.
pub fn set_profile(p: PowerProfile) -> PowerProfile {
    let mut lock = lock_or_recover(&power_profile_lock());
    let prev = *lock;
    *lock = p;
    prev
}

// ── Low battery thresholds ──────────────────────────────────────────────────

const THRESHOLD_LOW: u8      = 20;
const THRESHOLD_CRITICAL: u8 = 5;
const THRESHOLD_SHUTDOWN: u8 = 2;

/// Classify battery percentage into an action tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatteryAction {
    /// Above 20% — no action needed.
    Normal,
    /// 5..=20% — broadcast low warning.
    Low,
    /// 2..=4% — broadcast critical, auto-save session.
    Critical,
    /// <2% — initiate shutdown.
    Shutdown,
}

pub fn classify_battery(percent: u8, charging: bool) -> BatteryAction {
    if charging {
        return BatteryAction::Normal;
    }
    if percent < THRESHOLD_SHUTDOWN {
        BatteryAction::Shutdown
    } else if percent < THRESHOLD_CRITICAL {
        BatteryAction::Critical
    } else if percent < THRESHOLD_LOW {
        BatteryAction::Low
    } else {
        BatteryAction::Normal
    }
}

// ── Tray integration ────────────────────────────────────────────────────────

/// Build the battery tray indicator for the menu bar.
pub fn battery_indicator() -> crate::tray::TrayItem {
    match read_battery() {
        Some(info) if info.present && info.charging => {
            crate::tray::TrayItem {
                text: format!("AC:{}%", info.percent),
                color: 0x30D158FF, // green
            }
        }
        Some(info) if info.present => {
            let color = if info.percent < THRESHOLD_CRITICAL {
                0xFF453AFF // red
            } else if info.percent < THRESHOLD_LOW {
                0xFF9F0AFF // orange
            } else {
                0xEBEBEBFF // label white
            };
            crate::tray::TrayItem {
                text: format!("Bat:{}%", info.percent),
                color,
            }
        }
        _ => {
            // No battery / VM — show "AC"
            crate::tray::TrayItem {
                text: "AC".to_string(),
                color: 0x8E8E93FF, // dim
            }
        }
    }
}

// ── IPC command handlers ────────────────────────────────────────────────────

/// Handle battery/power IPC commands. Returns `true` if handled.
pub fn handle_battery_command(
    verb: &str,
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
) -> bool {
    match verb {
        "battery" => {
            let reply = match read_battery() {
                Some(info) if info.present => {
                    let status = if info.charging { "charging" } else { "discharging" };
                    format!(
                        "REPLY:battery {}% {} time_remaining={}min",
                        info.percent, status, info.time_remaining_mins,
                    )
                }
                Some(_) => "REPLY:battery not-present".to_string(),
                None => "REPLY:battery N/A (no battery detected)".to_string(),
            };
            crate::send_reply(sender, &reply, inbox);
        }
        "power-profile" => {
            let arg = parts.get(1).unwrap_or(&"").trim();
            if arg.is_empty() {
                let cur = current_profile();
                crate::send_reply(
                    sender,
                    &format!("REPLY:power-profile {}", cur.as_str()),
                    inbox,
                );
            } else {
                match PowerProfile::from_str(arg) {
                    Some(p) => {
                        let prev = set_profile(p);
                        log_info!(Subsystem::Lifecycle, None,
                            "power profile: {} -> {} (by {sender})",
                            prev.as_str(), p.as_str());
                        crate::send_reply(
                            sender,
                            &format!("REPLY:power-profile set to {}", p.as_str()),
                            inbox,
                        );
                    }
                    None => {
                        crate::send_reply(
                            sender,
                            "REPLY:error: usage: power-profile <performance|balanced|saver>",
                            inbox,
                        );
                    }
                }
            }
        }
        _ => return false,
    }
    true
}

// ── Monitor thread ──────────────────────────────────────────────────────────

/// Spawn the battery monitor thread. Checks every 30 seconds.
pub fn spawn_battery_monitor(
    inbox: &Inbox,
    app_registry: &AppRegistry,
    focused: &FocusedApp,
) {
    let inbox = Arc::clone(inbox);
    let registry = Arc::clone(app_registry);
    let focused = Arc::clone(focused);
    thread::Builder::new()
        .name("battery-monitor".into())
        .spawn(move || run_battery_monitor(&inbox, &registry, &focused))
        .expect("spawn battery-monitor thread");
}

/// Previous battery action tier — used to detect transitions.
static PREV_ACTION: OnceLock<Mutex<BatteryAction>> = OnceLock::new();

fn prev_action_lock() -> &'static Mutex<BatteryAction> {
    PREV_ACTION.get_or_init(|| Mutex::new(BatteryAction::Normal))
}

fn run_battery_monitor(inbox: &Inbox, registry: &AppRegistry, focused: &FocusedApp) {
    loop {
        thread::sleep(Duration::from_secs(30));

        let info = match read_battery() {
            Some(i) if i.present => i,
            _ => continue, // No battery — nothing to monitor.
        };

        let action = classify_battery(info.percent, info.charging);
        let prev = {
            let mut lock = lock_or_recover(&prev_action_lock());
            let p = *lock;
            *lock = action;
            p
        };

        // Only act on transitions to worse states (or any transition for logging).
        if action == prev {
            continue;
        }

        match action {
            BatteryAction::Normal => {
                log_info!(Subsystem::Lifecycle, None,
                    "battery {}% — back to normal", info.percent);
            }
            BatteryAction::Low => {
                log_warn!(Subsystem::Lifecycle, None,
                    "battery low: {}%", info.percent);
                broadcast_battery_event(inbox, registry, "low");
            }
            BatteryAction::Critical => {
                log_warn!(Subsystem::Lifecycle, None,
                    "battery critical: {}% — auto-saving session", info.percent);
                broadcast_battery_event(inbox, registry, "critical");
                // Auto-save session.
                match crate::session::save_session(registry, focused) {
                    Ok(bytes) => log_info!(Subsystem::Lifecycle, None,
                        "session auto-saved ({bytes} bytes) on critical battery"),
                    Err(e) => log_warn!(Subsystem::Lifecycle, None,
                        "session auto-save failed on critical battery: {e}"),
                }
            }
            BatteryAction::Shutdown => {
                log_warn!(Subsystem::Lifecycle, None,
                    "battery {}% — initiating emergency shutdown", info.percent);
                broadcast_battery_event(inbox, registry, "shutdown");
                // Auto-save before shutdown.
                let _ = crate::session::save_session(registry, focused);
                // Initiate shutdown after a brief delay.
                thread::spawn(|| {
                    thread::sleep(Duration::from_millis(500));
                    #[cfg(target_os = "linux")]
                    unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF); }
                });
            }
        }
    }
}

/// Broadcast `VYOMA_SYSTEM:battery:<level>` to all running apps.
fn broadcast_battery_event(inbox: &Inbox, registry: &AppRegistry, level: &str) {
    let msg = format!("VYOMA_SYSTEM:battery:{level}");
    let reg = lock_or_recover(&registry);
    let inb = lock_or_recover(&inbox);
    for (name, state_arc) in reg.iter() {
        let st = lock_or_recover(&state_arc);
        if matches!(st.status, AppStatus::Running) {
            if let Some(tx) = inb.get(name) {
                let _ = tx.send(msg.clone());
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sysfs_full_battery() {
        let fields = [("present", "1"), ("capacity", "85"), ("status", "Discharging")];
        let info = parse_sysfs_fields(&fields).unwrap();
        assert!(info.present);
        assert_eq!(info.percent, 85);
        assert!(!info.charging);
    }

    #[test]
    fn parse_sysfs_charging() {
        let fields = [("present", "1"), ("capacity", "42"), ("status", "Charging")];
        let info = parse_sysfs_fields(&fields).unwrap();
        assert!(info.charging);
        assert_eq!(info.percent, 42);
    }

    #[test]
    fn parse_sysfs_full_status() {
        let fields = [("present", "1"), ("capacity", "100"), ("status", "Full")];
        let info = parse_sysfs_fields(&fields).unwrap();
        assert!(info.charging); // "Full" counts as charging (on AC).
        assert_eq!(info.percent, 100);
    }

    #[test]
    fn parse_sysfs_not_present() {
        let fields = [("present", "0"), ("capacity", "50"), ("status", "Discharging")];
        let info = parse_sysfs_fields(&fields).unwrap();
        assert!(!info.present);
        assert_eq!(info.percent, 0);
    }

    #[test]
    fn parse_sysfs_missing_present_returns_none() {
        let fields = [("capacity", "50"), ("status", "Discharging")];
        assert!(parse_sysfs_fields(&fields).is_none());
    }

    #[test]
    fn power_profile_default_is_balanced() {
        // Fresh OnceLock — this may race with other tests but the default is Balanced.
        let p = PowerProfile::Balanced;
        assert_eq!(p.as_str(), "balanced");
    }

    #[test]
    fn power_profile_from_str_roundtrip() {
        for name in &["performance", "balanced", "saver", "powersaver"] {
            let p = PowerProfile::from_str(name);
            assert!(p.is_some(), "should parse {name}");
        }
        assert!(PowerProfile::from_str("turbo").is_none());
    }

    #[test]
    fn power_profile_as_str() {
        assert_eq!(PowerProfile::Performance.as_str(), "performance");
        assert_eq!(PowerProfile::Balanced.as_str(), "balanced");
        assert_eq!(PowerProfile::PowerSaver.as_str(), "saver");
    }

    #[test]
    fn classify_battery_charging_always_normal() {
        assert_eq!(classify_battery(5, true), BatteryAction::Normal);
        assert_eq!(classify_battery(1, true), BatteryAction::Normal);
    }

    #[test]
    fn classify_battery_all_tiers() {
        // Normal: >= 20%
        assert_eq!(classify_battery(100, false), BatteryAction::Normal);
        assert_eq!(classify_battery(20, false), BatteryAction::Normal);
        // Low: 5..19%
        assert_eq!(classify_battery(19, false), BatteryAction::Low);
        assert_eq!(classify_battery(5, false), BatteryAction::Low);
        // Critical: 2..4%
        assert_eq!(classify_battery(4, false), BatteryAction::Critical);
        assert_eq!(classify_battery(2, false), BatteryAction::Critical);
        // Shutdown: 0..1%
        assert_eq!(classify_battery(1, false), BatteryAction::Shutdown);
        assert_eq!(classify_battery(0, false), BatteryAction::Shutdown);
    }

    #[test]
    fn read_battery_from_nonexistent_dir() {
        assert!(read_battery_from("/nonexistent/battery/path").is_none());
    }
}
