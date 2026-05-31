// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Lock/unlock screen IPC commands.

use crate::{log_info, AppRegistry, FocusedApp, Inbox};
use crate::send_reply;
use crate::chrome::z_order_push_front;
use supervisor::logging::Subsystem;

/// Handle `@supervisor: lock` or `@supervisor: unlock`.
pub fn handle_lock_command(
    verb: &str,
    sender: &str,
    inbox: &Inbox,
    focused: &FocusedApp,
    app_registry: &AppRegistry,
) {
    if verb == "lock" {
        if crate::is_locked() {
            send_reply(sender, "REPLY:already locked", inbox);
            return;
        }
        crate::set_locked(true);
        let running = app_registry.lock().unwrap().get("screen-lock")
            .map(|st| matches!(st.lock().unwrap().status, crate::AppStatus::Running))
            .unwrap_or(false);
        if !running {
            let entry = supervisor::manifest::BootEntry {
                manifest: "/apps/screen-lock/vyoma.toml".to_string(),
                restart: "never".to_string(),
            };
            if let Some(app) = crate::app_threads::spawn_app(&entry, inbox, app_registry) {
                crate::app_threads::launch_app_threads(app, inbox, focused, app_registry);
            }
        }
        z_order_push_front("screen-lock");
        *focused.lock().unwrap() = Some("screen-lock".to_string());
        log_info!(Subsystem::Lifecycle, None, "screen locked by {sender}");
        send_reply(sender, "REPLY:locked", inbox);
    } else {
        // unlock
        if !crate::is_locked() {
            send_reply(sender, "REPLY:not locked", inbox);
            return;
        }
        crate::set_locked(false);
        let pid = app_registry.lock().unwrap().get("screen-lock")
            .and_then(|st| st.lock().unwrap().child_pid);
        if let Some(pid) = pid {
            #[cfg(target_os = "linux")]
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL); }
        }
        log_info!(Subsystem::Lifecycle, None, "screen unlocked by {sender}");
        send_reply(sender, "REPLY:unlocked", inbox);
    }
}
