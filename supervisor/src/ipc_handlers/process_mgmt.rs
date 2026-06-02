// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Process management IPC handlers: kill, restart, update, reload.

use std::{fs, sync::Arc};
use crate::lock_or_recover;

use sha2::{Digest, Sha256};

use crate::{
    log_info, log_error,
    AppRegistry, AppStatus, FocusedApp, Inbox,
    BOOT_CONFIG_PATH,
};
use crate::send_reply;
use crate::app_threads::{spawn_app, launch_app_threads};
use supervisor::logging::Subsystem;
use supervisor::manifest::{AppManifest, BootConfig, BootEntry};
use crate::net::http_get;

/// Handle `kill <app>` -- send SIGKILL to a running app.
pub fn handle_kill(
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    app_registry: &AppRegistry,
) {
    let app_name = match parts.get(1).map(|s| s.trim()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            send_reply(sender, "REPLY:error: usage: kill <app>", inbox);
            return;
        }
    };
    let (pid, running_names, manifest_path) = {
        let reg = lock_or_recover(&app_registry);
        let (pid, mfst) = reg.get(&app_name).map(|st| {
            let s = lock_or_recover(&st);
            (s.child_pid, s.entry.manifest.clone())
        }).unwrap_or((None, String::new()));
        (pid, reg.keys().cloned().collect::<Vec<_>>(), mfst)
    };
    let running_refs: Vec<&str> = running_names.iter().map(|s| s.as_str()).collect();
    if !supervisor::ipc::validate_kill_target(&app_name, &running_refs) {
        log_info!(Subsystem::Ipc, Some(app_name.as_str()), "kill: app={app_name} not found or not running");
        send_reply(sender, &format!("REPLY:{app_name} not running"), inbox);
        return;
    }
    match pid {
        Some(pid) => {
            // T054: enqueue Close animation before killing
            if let Some(st) = lock_or_recover(&app_registry).get(&app_name) {
                use crate::display::animator::{Animation, AnimKind, now_ms};
                lock_or_recover(&st).pending_anim = Some(Animation::new(AnimKind::Close, now_ms()));
            }
            #[cfg(target_os = "linux")]
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
            log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "killed {app_name} (pid {pid})");
            crate::undo::capture_kill(&app_name, &manifest_path);
            send_reply(sender, &format!("REPLY:killed {app_name}"), inbox);
        }
        None => {
            log_info!(Subsystem::Ipc, Some(app_name.as_str()), "kill: app={app_name} not found or not running");
            send_reply(sender, &format!("REPLY:{app_name} not running"), inbox);
        }
    }
}

/// Handle `restart <app>` -- kill + re-spawn an app.
pub fn handle_restart(
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    let app_name = match parts.get(1).map(|s| s.trim()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            send_reply(sender, "REPLY:error: usage: restart <app>", inbox);
            return;
        }
    };
    let (pid, entry) = {
        let reg = lock_or_recover(&app_registry);
        match reg.get(&app_name) {
            Some(st) => {
                let st = lock_or_recover(&st);
                (st.child_pid, st.entry.clone())
            }
            None => {
                send_reply(sender, &format!("REPLY:error: app {app_name} not found"), inbox);
                return;
            }
        }
    };
    // Kill old instance (if still running)
    if let Some(pid) = pid {
        #[cfg(target_os = "linux")]
        unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
        log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "restart: killed old {app_name} (pid {pid})");
    }
    // Spawn new instance
    match spawn_app(&entry, inbox, app_registry) {
        Some(app) => {
            launch_app_threads(app, inbox, focused, app_registry);
            log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "restarted {app_name}");
            send_reply(sender, &format!("REPLY:restarted {app_name}"), inbox);
        }
        None => {
            send_reply(sender, &format!("REPLY:error: could not restart {app_name}"), inbox);
        }
    }
}

/// Handle `update <app> <url>` -- download new wasm, verify sha256, hot-swap, restart.
pub fn handle_update(
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    let rest = parts.get(1).unwrap_or(&"").trim();
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
    send_reply(sender, &format!("REPLY:downloading {url}\u{2026}"), inbox);
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
                        send_reply(&sender_name, "REPLY:error: SHA-256 mismatch \u{2014} update rejected", &inbox_bg);
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
        log_info!(Subsystem::Lifecycle, Some(app_name.as_str()), "update {app_name}: installed \u{2192} {dest:?}");
        send_reply(&sender_name, &format!("REPLY:installed \u{2014} restarting {app_name}\u{2026}"), &inbox_bg);

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

/// Handle `reload` -- re-read boot.toml, launch any apps not currently running.
pub fn handle_reload(
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    let boot_raw = match fs::read_to_string(BOOT_CONFIG_PATH) {
        Ok(s) => s,
        Err(e) => {
            send_reply(sender, &format!("REPLY:error: cannot read boot.toml: {e}"), inbox);
            return;
        }
    };
    let boot: BootConfig = match toml::from_str(&boot_raw) {
        Ok(c) => c,
        Err(e) => {
            send_reply(sender, &format!("REPLY:error: malformed boot.toml: {e}"), inbox);
            return;
        }
    };
    let mut launched = 0usize;
    for entry in &boot.apps {
        // Parse manifest to get name
        let name = match fs::read_to_string(&entry.manifest).ok()
            .and_then(|s| toml::from_str::<AppManifest>(&s).ok())
            .map(|m| m.app.name)
        {
            Some(n) => n,
            None => continue,
        };
        // Skip already-running apps
        let already_running = {
            let reg = lock_or_recover(&app_registry);
            reg.get(&name).map(|st| {
                matches!(lock_or_recover(&st).status, AppStatus::Running)
            }).unwrap_or(false)
        };
        if already_running { continue; }

        if let Some(app) = spawn_app(entry, inbox, app_registry) {
            launch_app_threads(app, inbox, focused, app_registry);
            launched += 1;
        }
    }
    send_reply(
        sender,
        &format!("REPLY:reload done \u{2014} {launched} new app(s) started"),
        inbox,
    );
}
