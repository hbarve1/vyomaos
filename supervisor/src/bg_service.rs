// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P63: Background services — apps with `background = true` run without a
//! display surface.  They are excluded from window tiling and the window
//! manager z-order, but participate in IPC and appear in `ps` / `ps-raw`
//! output with a `[bg]` marker.

use crate::{
    log_info,
    AppRegistry, AppStatus, FocusedApp, Inbox,
};
use crate::app_threads::{spawn_app, launch_app_threads};
use crate::send_reply;
use supervisor::logging::Subsystem;
use supervisor::manifest::BootEntry;

/// Check whether `app_name` is a background service in the registry.
pub fn is_background(app_name: &str, registry: &AppRegistry) -> bool {
    registry
        .lock()
        .unwrap()
        .get(app_name)
        .map(|st| st.lock().unwrap().is_background)
        .unwrap_or(false)
}

/// Handle `bg-list`, `bg-start`, and `bg-stop` IPC commands.
/// Returns `true` if the verb was recognised and handled.
pub fn handle_bg_command(
    verb:         &str,
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> bool {
    match verb {
        "bg-list" => {
            let entries: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut rows: Vec<String> = reg
                    .iter()
                    .filter_map(|(name, st)| {
                        let st = st.lock().unwrap();
                        if st.is_background {
                            let status = match &st.status {
                                AppStatus::Running    => "running",
                                AppStatus::Stopped(_) => "stopped",
                            };
                            Some(format!("{name} [{status}]"))
                        } else {
                            None
                        }
                    })
                    .collect();
                rows.sort();
                rows
            };
            if entries.is_empty() {
                send_reply(sender, "REPLY:no background services", inbox);
            } else {
                send_reply(sender, &format!("REPLY:{}", entries.join("|")), inbox);
            }
            log_info!(Subsystem::Ipc, None,
                "bg-list query from {sender}: {} service(s)", entries.len());
            true
        }

        "bg-start" => {
            let manifest_path = match parts.get(1).map(|s| s.trim()) {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => {
                    send_reply(
                        sender,
                        "REPLY:error: usage: bg-start <manifest_path>",
                        inbox,
                    );
                    return true;
                }
            };

            let entry = BootEntry {
                manifest: manifest_path.clone(),
                restart: "never".to_string(),
            };

            match spawn_app(&entry, inbox, app_registry) {
                Some(app) => {
                    let app_name = app.name.clone();
                    // Mark as background in the registry (in case the manifest
                    // did not already set background = true).
                    {
                        let reg = app_registry.lock().unwrap();
                        if let Some(st) = reg.get(&app_name) {
                            st.lock().unwrap().is_background = true;
                        }
                    }
                    launch_app_threads(app, inbox, focused, app_registry);
                    log_info!(Subsystem::Lifecycle, Some(app_name.as_str()),
                        "bg-start: launched {app_name} as background service");
                    send_reply(
                        sender,
                        &format!("REPLY:bg-started {app_name}"),
                        inbox,
                    );
                }
                None => {
                    send_reply(
                        sender,
                        &format!("REPLY:error: could not spawn {manifest_path}"),
                        inbox,
                    );
                }
            }
            true
        }

        "bg-stop" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: bg-stop <app>", inbox);
                    return true;
                }
            };

            let pid = {
                let reg = app_registry.lock().unwrap();
                match reg.get(&app_name) {
                    Some(st) => {
                        let st = st.lock().unwrap();
                        if !st.is_background {
                            send_reply(
                                sender,
                                &format!("REPLY:error: {app_name} is not a background service"),
                                inbox,
                            );
                            return true;
                        }
                        st.child_pid
                    }
                    None => {
                        send_reply(
                            sender,
                            &format!("REPLY:error: app {app_name} not found"),
                            inbox,
                        );
                        return true;
                    }
                }
            };

            match pid {
                Some(pid) => {
                    #[cfg(target_os = "linux")]
                    unsafe {
                        libc::kill(pid as libc::pid_t, libc::SIGKILL);
                    }
                    log_info!(Subsystem::Lifecycle, Some(app_name.as_str()),
                        "bg-stop: killed {app_name} (pid {pid})");
                    send_reply(
                        sender,
                        &format!("REPLY:bg-stopped {app_name}"),
                        inbox,
                    );
                }
                None => {
                    send_reply(
                        sender,
                        &format!("REPLY:{app_name} not running"),
                        inbox,
                    );
                }
            }
            true
        }

        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    /// Helper: build a minimal AppState for testing.
    fn make_test_state(background: bool) -> crate::AppState {
        use std::collections::VecDeque;
        use std::time::Instant;
        crate::AppState {
            entry: BootEntry {
                manifest: "/tmp/test.toml".to_string(),
                restart: "never".to_string(),
            },
            status: AppStatus::Running,
            start_time: Instant::now(),
            restart_count: 0,
            log_buf: VecDeque::new(),
            child_pid: Some(999),
            watchdog_secs: 0,
            last_output: Arc::new(Mutex::new(Instant::now())),
            watchdog_backoff: Arc::new(Mutex::new(0)),
            has_mouse: false,
            has_display: false,
            has_audio: false,
            is_background: background,
            win_region: None,
            win_z: 10,
            min_size: (0, 0),
            draw_ticks: 0,
            last_cpu_reset: Instant::now(),
            minimized: false,
            pre_minimize_region: None,
            pending_anim: None,
            surface: None,
            frame_ready: false,
            menu_items: Vec::new(),
            log_subscribers: Vec::new(),
        }
    }

    #[test]
    fn test_is_background_true() {
        let registry: AppRegistry = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(Mutex::new(make_test_state(true)));
        registry.lock().unwrap().insert("bg-app".to_string(), state);
        assert!(is_background("bg-app", &registry));
    }

    #[test]
    fn test_is_background_false() {
        let registry: AppRegistry = Arc::new(Mutex::new(HashMap::new()));
        let state = Arc::new(Mutex::new(make_test_state(false)));
        registry.lock().unwrap().insert("fg-app".to_string(), state);
        assert!(!is_background("fg-app", &registry));
    }

    #[test]
    fn test_is_background_unknown_app() {
        let registry: AppRegistry = Arc::new(Mutex::new(HashMap::new()));
        assert!(!is_background("nonexistent", &registry));
    }
}
