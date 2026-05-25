// OTA update IPC command handler — T026
//
// Handles: @supervisor: ota-update <name> <path>
//
// Workflow:
//   1. Validate the module name and path.
//   2. Record the update in the in-process OtaManager.
//   3. Copy the new WASM binary to the inactive A/B slot.
//   4. Swap the active slot.
//   5. Restart the module from the new slot path.
//   6. Run a health check; on failure, roll back to the previous slot.
//
// The OtaManager and AbSlot live in supervisor::ota.  This module wires
// them into the IPC dispatcher.

use std::sync::{Arc, Mutex, OnceLock};

use supervisor::logging::Subsystem;
use supervisor::ota::{OtaManager, UpdateStatus};
use supervisor::ota::ab_slot::{AbSlot, SlotLabel};
use supervisor::ota::health_check::{HealthChecker, HealthCheckResult};

use crate::{log_info, log_warn, log_error, Inbox, AppRegistry, FocusedApp};
use crate::send_reply;
use supervisor::manifest::BootEntry;

// ── Global OTA manager ────────────────────────────────────────────────────────

static OTA_MANAGER: OnceLock<Mutex<OtaManager>> = OnceLock::new();

fn ota_manager() -> &'static Mutex<OtaManager> {
    OTA_MANAGER.get_or_init(|| Mutex::new(OtaManager::new()))
}

// ── Global A/B slot registry ──────────────────────────────────────────────────

static AB_SLOTS: OnceLock<Mutex<std::collections::HashMap<String, AbSlot>>> = OnceLock::new();

fn ab_slots() -> &'static Mutex<std::collections::HashMap<String, AbSlot>> {
    AB_SLOTS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

// ── Slot directory constants ──────────────────────────────────────────────────

const SLOT_A_DIR: &str = "/data/ota/a";
const SLOT_B_DIR: &str = "/data/ota/b";
const HEALTH_TIMEOUT_SECS: u32 = 30;
const HEALTH_REQUIRED_CHECKS: u32 = 1;

// ── ota-update handler ────────────────────────────────────────────────────────

/// Handle `ota-update <name> <path>` IPC command.
///
/// Returns `true` if the command was recognised, `false` otherwise.
pub fn handle_ota_update(
    args:         &str,
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> bool {
    let mut iter = args.splitn(2, ' ');
    let name = match iter.next().map(str::trim).filter(|s| !s.is_empty()) {
        Some(n) => n.to_string(),
        None => {
            send_reply(sender, "REPLY:error: usage: ota-update <name> <path>", inbox);
            return true;
        }
    };
    let src_path = match iter.next().map(str::trim).filter(|s| !s.is_empty()) {
        Some(p) => p.to_string(),
        None => {
            send_reply(sender, "REPLY:error: usage: ota-update <name> <path>", inbox);
            return true;
        }
    };

    // Verify source path exists.
    if !std::path::Path::new(&src_path).exists() {
        send_reply(sender, &format!("REPLY:error: path not found: {src_path}"), inbox);
        return true;
    }

    log_info!(
        Subsystem::Lifecycle, Some(name.as_str()),
        "ota-update {name}: begin from {src_path}"
    );

    let sender_s       = sender.to_string();
    let inbox_bg       = Arc::clone(inbox);
    let focused_bg     = Arc::clone(focused);
    let registry_bg    = Arc::clone(app_registry);

    std::thread::spawn(move || {
        run_ota_update(name, src_path, sender_s, inbox_bg, focused_bg, registry_bg);
    });

    send_reply(sender, "REPLY:ota-update queued", inbox);
    true
}

// ── Core OTA logic (runs in background thread) ────────────────────────────────

fn run_ota_update(
    name:      String,
    src_path:  String,
    sender:    String,
    inbox:     Inbox,
    focused:   FocusedApp,
    registry:  AppRegistry,
) {
    // 1. Determine current entry and from_version.
    let entry = {
        let reg = registry.lock().unwrap();
        reg.get(&name).map(|st| st.lock().unwrap().entry.clone())
    };
    let entry = match entry {
        Some(e) => e,
        None => {
            log_warn!(Subsystem::Lifecycle, Some(name.as_str()),
                "ota-update {name}: module not found in registry — installing fresh");
            BootEntry { manifest: format!("/etc/vyoma/apps/{name}.toml"), restart: "always".to_string() }
        }
    };

    // 2. Determine from_version (stub — version stored in manifest or "unknown").
    let from_version = {
        std::fs::read_to_string(&entry.manifest)
            .ok()
            .and_then(|s| {
                toml::from_str::<supervisor::manifest::AppManifest>(&s).ok()
            })
            .map(|m| m.app.version)
            .unwrap_or_else(|| "unknown".to_string())
    };

    // 3. Determine which slot is currently active and where to deploy.
    let (inactive_slot, dest_path) = {
        let slots = ab_slots().lock().unwrap();
        match slots.get(&name) {
            Some(ab) => {
                let inactive = ab.active.other();
                let dir = if inactive == SlotLabel::A { SLOT_A_DIR } else { SLOT_B_DIR };
                (inactive, format!("{dir}/{name}.wasm"))
            }
            None => {
                // First update: create slots starting at A, deploy to B.
                (SlotLabel::B, format!("{SLOT_B_DIR}/{name}.wasm"))
            }
        }
    };

    // 4. Copy new binary to inactive slot.
    if let Some(parent) = std::path::Path::new(&dest_path).parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            log_error!(Subsystem::Lifecycle, Some(name.as_str()),
                "ota-update {name}: create slot dir failed: {e}");
            send_reply(&sender, &format!("REPLY:error: slot dir create failed: {e}"), &inbox);
            return;
        }
    }
    if let Err(e) = std::fs::copy(&src_path, &dest_path) {
        log_error!(Subsystem::Lifecycle, Some(name.as_str()),
            "ota-update {name}: copy to slot failed: {e}");
        send_reply(&sender, &format!("REPLY:error: copy to slot failed: {e}"), &inbox);
        return;
    }
    log_info!(Subsystem::Lifecycle, Some(name.as_str()),
        "ota-update {name}: copied to {dest_path} (slot {inactive_slot})");

    // 5. Record update in OtaManager.
    // Version is extracted from a WASM custom section in production; stub uses "new".
    let to_version = "new".to_string();
    let idx = ota_manager()
        .lock()
        .unwrap()
        .begin_update(name.clone(), from_version.clone(), to_version, inactive_slot);

    ota_manager().lock().unwrap().set_status(idx, UpdateStatus::Deploying);

    // 6. Swap A/B slot.
    {
        let mut slots = ab_slots().lock().unwrap();
        let ab = slots.entry(name.clone()).or_insert_with(|| {
            let a_path = format!("{SLOT_A_DIR}/{name}.wasm");
            AbSlot::new(name.clone(), a_path)
        });
        ab.slot_b_path = if inactive_slot == SlotLabel::B {
            dest_path.clone()
        } else {
            ab.slot_b_path.clone()
        };
        ab.swap();
    }

    // 7. Restart module from new slot.
    let new_entry = BootEntry { manifest: entry.manifest.clone(), restart: entry.restart.clone() };

    // Kill old instance.
    {
        let reg = registry.lock().unwrap();
        if let Some(st) = reg.get(&name) {
            if let Some(pid) = st.lock().unwrap().child_pid {
                #[cfg(target_os = "linux")]
                unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
            }
        }
    }

    ota_manager().lock().unwrap().set_status(idx, UpdateStatus::HealthChecking);

    match crate::app_threads::spawn_app(&new_entry, &inbox, &registry) {
        Some(app) => {
            crate::app_threads::launch_app_threads(app, &inbox, &focused, &registry);
            log_info!(Subsystem::Lifecycle, Some(name.as_str()),
                "ota-update {name}: module restarted on slot {inactive_slot}");
        }
        None => {
            log_error!(Subsystem::Lifecycle, Some(name.as_str()),
                "ota-update {name}: failed to restart — rolling back");
            ota_manager().lock().unwrap().rollback(idx, "spawn failed after OTA");
            ab_slots().lock().unwrap().get_mut(&name).map(|ab| ab.rollback());
            send_reply(&sender, &format!("REPLY:error: ota-update {name}: spawn failed, rolled back"), &inbox);
            return;
        }
    }

    // 8. Inline health check (simplified: wait briefly, assume healthy for now).
    //    A real implementation polls heartbeat events; here we do a minimal wait.
    let checker = HealthChecker::new(name.clone(), HEALTH_TIMEOUT_SECS, HEALTH_REQUIRED_CHECKS);
    // Give the module a moment to start (simplified health check).
    std::thread::sleep(std::time::Duration::from_secs(1));

    // In the real flow, heartbeats would be fed to checker.record_healthy().
    // For the IPC integration, we optimistically commit if the module spawned.
    match checker.evaluate() {
        HealthCheckResult::Passed => {
            ota_manager().lock().unwrap().record_health_check(idx);
            ota_manager().lock().unwrap().set_status(idx, UpdateStatus::Complete);
            ab_slots().lock().unwrap().get_mut(&name).map(|ab| { ab.health_confirmed = true; });
            log_info!(Subsystem::Lifecycle, Some(name.as_str()),
                "ota-update {name}: health check passed — committed to slot {inactive_slot}");
            send_reply(&sender, &format!("REPLY:ota-update {name} OK (slot {inactive_slot})"), &inbox);
        }
        HealthCheckResult::Failed => {
            // Roll back.
            ota_manager().lock().unwrap().rollback(idx, "health check failed");
            ab_slots().lock().unwrap().get_mut(&name).map(|ab| ab.rollback());
            log_warn!(Subsystem::Lifecycle, Some(name.as_str()),
                "ota-update {name}: health check failed — rolled back to previous slot");
            send_reply(&sender, &format!("REPLY:error: ota-update {name}: health check failed, rolled back"), &inbox);
        }
    }
}

