// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! @supervisor `update <app> <url>` IPC command — download, verify SHA-256, hot-swap, restart.

use std::{fs, sync::Arc};
use crate::lock_or_recover;

use sha2::{Digest, Sha256};

use crate::{
    log_info, log_error,
    AppRegistry, FocusedApp, Inbox,
};
use crate::send_reply;
use crate::app_threads::{spawn_app, launch_app_threads};
use supervisor::logging::Subsystem;
use supervisor::manifest::AppManifest;
use crate::net::http_get;

/// Handle `@supervisor: update <app> <url>`.
///
/// Downloads the new WASM binary from `url`, optionally verifies its SHA-256
/// against the value declared in the app's manifest, hot-swaps the binary on
/// disk, kills the old instance, and re-spawns it.  The download and restart
/// run on a background thread so the calling app is not blocked.
pub fn handle_update(
    rest:         &str,
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    let (app_name, url) = match rest.split_once(' ') {
        Some((a, u)) if !a.trim().is_empty() && !u.trim().is_empty() => {
            (a.trim().to_string(), u.trim().to_string())
        }
        _ => {
            send_reply(sender, "REPLY:error: usage: update <app> <url>", inbox);
            return;
        }
    };
    let entry = {
        let reg = lock_or_recover(&app_registry);
        reg.get(&app_name).map(|st| lock_or_recover(&st).entry.clone())
    };
    let entry = match entry {
        Some(e) => e,
        None => {
            send_reply(sender, &format!("REPLY:error: unknown app: {app_name}"), inbox);
            return;
        }
    };
    send_reply(sender, &format!("REPLY:downloading {url}…"), inbox);
    log_info!(Subsystem::Ipc, Some(app_name.as_str()), "@supervisor: update {app_name} from {url}");

    let sender_name  = sender.to_string();
    let inbox_bg     = Arc::clone(inbox);
    let focused_bg   = Arc::clone(focused);
    let registry_bg  = Arc::clone(app_registry);

    std::thread::spawn(move || {
        let bytes = match http_get(&url) {
            Ok(b)  => b,
            Err(e) => {
                log_error!(Subsystem::Lifecycle, Some(app_name.as_str()), "update {app_name}: download failed: {e}");
                send_reply(&sender_name, &format!("REPLY:error: download failed: {e}"), &inbox_bg);
                return;
            }
        };

        let tmp = format!("/tmp/{app_name}.wasm.new");
        if let Err(e) = fs::write(&tmp, &bytes) {
            send_reply(&sender_name, &format!("REPLY:error: write tmp failed: {e}"), &inbox_bg);
            return;
        }

        // Verify SHA-256 if declared in manifest (reuses P28 Sha256)
        if let Ok(raw) = fs::read_to_string(&entry.manifest) {
            if let Ok(m) = toml::from_str::<AppManifest>(&raw) {
                if let Some(expected) = &m.app.wasm_sha256 {
                    let actual = format!("{:x}", Sha256::digest(&bytes));
                    if actual != expected.to_lowercase() {
                        let _ = fs::remove_file(&tmp);
                        send_reply(&sender_name, "REPLY:error: SHA-256 mismatch — update rejected", &inbox_bg);
                        return;
                    }
                    log_info!(Subsystem::Capability, Some(app_name.as_str()), "update {app_name}: SHA-256 verified OK");
                }
            }
        }

        use std::path::Path;
        let dest = Path::new(&entry.manifest)
            .parent()
            .unwrap_or(Path::new("/apps"))
            .join(format!("{app_name}.wasm"));
        if let Err(e) = fs::copy(&tmp, &dest) {
            let _ = fs::remove_file(&tmp);
            send_reply(&sender_name, &format!("REPLY:error: install failed: {e}"), &inbox_bg);
            return;
        }
        let _ = fs::remove_file(&tmp);
        log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "update {app_name}: installed → {dest:?}");
        send_reply(&sender_name, &format!("REPLY:installed — restarting {app_name}…"), &inbox_bg);

        // Kill old instance then respawn
        {
            let reg = lock_or_recover(&registry_bg);
            if let Some(st) = reg.get(&app_name) {
                if let Some(pid) = lock_or_recover(&st).child_pid {
                    #[cfg(target_os = "linux")]
                    unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
                }
            }
        }
        match spawn_app(&entry, &inbox_bg, &registry_bg) {
            Some(app) => {
                launch_app_threads(app, &inbox_bg, &focused_bg, &registry_bg);
                log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "update {app_name}: restarted OK");
                send_reply(&sender_name, &format!("REPLY:updated {app_name} OK"), &inbox_bg);
            }
            None => {
                send_reply(&sender_name, "REPLY:error: restart failed after update", &inbox_bg);
            }
        }
    });
}
