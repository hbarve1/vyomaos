// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! @supervisor IPC command handler — process management commands (P12T03, P13T01, P16T01).
//! Extended commands (pkg-*, wallpaper, resize, tcp-*, ...) are handled in `ipc_commands`.
//!
//! Submodules:
//! - `process_mgmt`: kill, restart, update, reload
//! - `log_cmds`: log, logs, logf

mod process_mgmt;
mod log_cmds;

use crate::{
    log_info, log_warn,
    AppRegistry, AppStatus, FocusedApp, Inbox,
};
use crate::send_reply;
use crate::app_threads::{spawn_app, launch_app_threads};
use supervisor::lifecycle::format_ps_line;
use supervisor::logging::Subsystem;

pub fn handle_supervisor_command(
    cmd:          &str,
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();

    // P89: audit every @supervisor command.
    crate::audit::log_audit(
        sender,
        "ipc",
        cmd,
        crate::audit::AuditResult::Allowed,
    );

    match parts[0] {
        // -- P12T03 legacy commands -----------------------------------------------
        "list" => {
            let names = inbox.lock().unwrap().keys().cloned().collect::<Vec<_>>().join("|");
            send_reply(sender, &format!("REPLY:{names}"), inbox);
        }
        "status" => {
            let count = inbox.lock().unwrap().len();
            send_reply(sender, &format!("REPLY:{{\"running\":{count}}}"), inbox);
        }

        // 030-ipc-list-apps: return newline-separated list of running app names
        "apps" => {
            let names: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut v: Vec<String> = reg.iter()
                    .filter(|(_, st)| matches!(st.lock().unwrap().status, AppStatus::Running))
                    .map(|(name, _)| name.clone())
                    .collect();
                v.sort();
                v
            };
            let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
            let reply = supervisor::ipc::format_app_list(&name_refs);
            send_reply(sender, &format!("REPLY:{reply}"), inbox);
            log_info!(Subsystem::Ipc, None, "apps list sent to {sender}: {} apps", names.len());
        }

        "focus" => {
            if let Some(name) = parts.get(1).map(|s| s.trim()) {
                *focused.lock().unwrap() = Some(name.to_string());
                log_info!(Subsystem::Input, Some(name), "focus \u{2192} {name}");
            }
        }

        // P54: win-info <app> -- return window region of an app
        "win-info" => {
            let app_name = parts.get(1).unwrap_or(&"").trim().to_string();
            let reg = app_registry.lock().unwrap();
            if let Some(st) = reg.get(&app_name) {
                let region = st.lock().unwrap().win_region;
                let coords = match region {
                    Some((x, y, w, h)) => format!("{x},{y},{w},{h}"),
                    None => "none".to_string(),
                };
                send_reply(sender, &format!("REPLY:win-info {app_name} {coords}"), inbox);
            } else {
                send_reply(sender, &format!("REPLY:win-info {app_name} not-found"), inbox);
            }
        }

        // P55: font-size <s|m|l> -- store global font size preference
        "font-size" => {
            let size = parts.get(1).unwrap_or(&"m").trim().to_string();
            let valid = matches!(size.as_str(), "s" | "m" | "l");
            if valid {
                *crate::FONT_SIZE.get().unwrap().lock().unwrap() = size.clone();
                log_info!(Subsystem::Lifecycle, None, "font-size \u{2192} {size}");
                send_reply(sender, &format!("REPLY:font-size {size}"), inbox);
            } else {
                send_reply(sender, "REPLY:font-size error invalid-size", inbox);
            }
        }

        // P52: input <char> -- forward a character to the focused app's stdin
        "input" => {
            let ch = parts.get(1).map(|s| s.to_string()).unwrap_or_default();
            let target = focused.lock().unwrap().clone();
            if let Some(name) = target {
                let map = inbox.lock().unwrap();
                if let Some(tx) = map.get(&name) {
                    let _ = tx.send(ch);
                }
            }
        }
        "run" => {
            let path = match parts.get(1).map(|s| s.trim()) {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: run <manifest_path>", inbox);
                    return;
                }
            };
            log_info!(Subsystem::Ipc, None, "@supervisor: run {path}");
            let entry = supervisor::manifest::BootEntry {
                manifest: path.clone(),
                restart: "never".to_string(),
            };
            match spawn_app(&entry, inbox, app_registry) {
                Some(app) => {
                    let app_name = app.name.clone();
                    launch_app_threads(app, inbox, focused, app_registry);
                    crate::undo::capture_run(&app_name, &path);
                    send_reply(sender, &format!("REPLY:launched {app_name}"), inbox);
                }
                None => {
                    send_reply(sender, &format!("REPLY:error: could not spawn {path}"), inbox);
                }
            }
        }

        // -- P13T01 process management commands -----------------------------------
        // ps-raw -- machine-readable: name:status:uptime_secs:restarts per entry
        "ps-raw" => {
            let entries: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut rows: Vec<(String, String)> = reg.iter().map(|(name, st)| {
                    let st = st.lock().unwrap();
                    let uptime = st.start_time.elapsed().as_secs();
                    let status = match &st.status {
                        AppStatus::Running    => "run",
                        AppStatus::Stopped(_) => "stop",
                    };
                    let bg_tag = if st.is_background { "[bg]" } else { "" };
                    let entry = format!("{}{}:{}:{}:{}", name, bg_tag, status, uptime, st.restart_count);
                    (name.clone(), entry)
                }).collect();
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                rows.into_iter().map(|(_, v)| v).collect()
            };
            send_reply(sender, &format!("REPLY:{}", entries.join("|")), inbox);
        }

        // ps -- list all apps with status, uptime, restart count, cpu%
        "ps" => {
            let entries: Vec<String> = {
                let reg = app_registry.lock().unwrap();
                let mut rows: Vec<(String, String)> = reg.iter().map(|(name, st)| {
                    let st = st.lock().unwrap();
                    let pid = st.child_pid.unwrap_or(0);
                    let uptime_secs = st.start_time.elapsed().as_secs();
                    let uptime_str = supervisor::lifecycle::format_uptime(uptime_secs);
                    let state_str = match &st.status {
                        AppStatus::Running    => "running".to_string(),
                        AppStatus::Stopped(c) => format!("stopped({})", c),
                    };
                    let wd_tag = if st.watchdog_secs > 0 {
                        format!(" [watchdog={}s]", st.watchdog_secs)
                    } else {
                        String::new()
                    };
                    let cpu = supervisor::lifecycle::format_cpu(
                        st.draw_ticks,
                        st.last_cpu_reset.elapsed().as_millis() as u64,
                    );
                    let bg_tag = if st.is_background { " [bg]" } else { "" };
                    let base = format_ps_line(name, pid, &state_str, st.restart_count);
                    let info = format!("{}{}{} up:{} cpu:{}", base, wd_tag, bg_tag, uptime_str, cpu);
                    (name.clone(), info)
                }).collect();
                rows.sort_by(|a, b| a.0.cmp(&b.0));
                rows.into_iter().map(|(_, v)| v).collect()
            };
            send_reply(sender, &format!("REPLY:{}", entries.join("|")), inbox);
        }

        // -- Process management (submodule) ---------------------------------------
        "kill"    => process_mgmt::handle_kill(&parts, sender, inbox, app_registry),
        "restart" => process_mgmt::handle_restart(&parts, sender, inbox, focused, app_registry),
        "update"  => process_mgmt::handle_update(&parts, sender, inbox, focused, app_registry),
        "reload"  => process_mgmt::handle_reload(sender, inbox, focused, app_registry),

        // -- Log commands (submodule) ---------------------------------------------
        "log"  => log_cmds::handle_log(&parts, sender, inbox, app_registry),
        "logs" => log_cmds::handle_logs(sender, inbox),
        "logf" => log_cmds::handle_logf(&parts, sender, inbox),

        // All remaining commands delegated to ipc_commands module.
        other => {
            if !crate::ipc_commands::handle_extended_command(
                other, &parts, sender, inbox, focused, app_registry
            ) {
                log_warn!(Subsystem::Ipc, None, "unknown @supervisor command from {sender}: {other}");
            }
        }
    }
}

// -- Inline tests: command parsing patterns -----------------------------------

#[cfg(test)]
mod tests {
    /// Verify splitn(2, ' ') correctly splits command + argument.
    #[test]
    fn test_splitn_single_word() {
        let parts: Vec<&str> = "ps".splitn(2, ' ').collect();
        assert_eq!(parts, vec!["ps"]);
    }

    #[test]
    fn test_splitn_command_with_arg() {
        let parts: Vec<&str> = "kill calc".splitn(2, ' ').collect();
        assert_eq!(parts, vec!["kill", "calc"]);
    }

    #[test]
    fn test_splitn_command_with_multi_word_arg() {
        let parts: Vec<&str> = "update calc http://example.com/calc.wasm".splitn(2, ' ').collect();
        assert_eq!(parts, vec!["update", "calc http://example.com/calc.wasm"]);
    }

    /// The update command does a nested split_once to separate app name from URL.
    #[test]
    fn test_update_arg_parsing() {
        let rest = "calc http://example.com/calc.wasm";
        let (app, url) = rest.split_once(' ').unwrap();
        assert_eq!(app.trim(), "calc");
        assert_eq!(url.trim(), "http://example.com/calc.wasm");
    }

    #[test]
    fn test_update_arg_missing_url() {
        let rest = "calc";
        assert!(rest.split_once(' ').is_none());
    }

    /// Verify log line pipe replacement (prevents breaking the reply protocol).
    #[test]
    fn test_log_line_pipe_replacement() {
        let line = "hello | world | test";
        let cleaned = line.replace('|', " ");
        assert_eq!(cleaned, "hello   world   test");
        assert!(!cleaned.contains('|'));
    }

    /// Verify the font-size validation logic.
    #[test]
    fn test_font_size_valid() {
        for s in &["s", "m", "l"] {
            assert!(matches!(*s, "s" | "m" | "l"), "should be valid: {s}");
        }
    }

    #[test]
    fn test_font_size_invalid() {
        for s in &["xl", "xs", "medium", ""] {
            assert!(!matches!(*s, "s" | "m" | "l"), "should be invalid: {s}");
        }
    }

    /// Verify the ps-raw format: name:status:uptime:restarts
    #[test]
    fn test_ps_raw_format() {
        let entry = format!("{}:{}:{}:{}", "calc", "run", 120, 0);
        assert_eq!(entry, "calc:run:120:0");
        let parts: Vec<&str> = entry.split(':').collect();
        assert_eq!(parts.len(), 4);
    }

    /// Verify background tag format.
    #[test]
    fn test_bg_tag_format() {
        let is_bg = true;
        let bg_tag = if is_bg { "[bg]" } else { "" };
        let entry = format!("calc{}", bg_tag);
        assert_eq!(entry, "calc[bg]");
    }

    /// Verify the installed.txt tail logic for logf command.
    #[test]
    fn test_log_tail_lines() {
        let log_tail_lines: usize = 30;
        let all: Vec<&str> = (0..100).map(|_| "line").collect();
        let start = all.len().saturating_sub(log_tail_lines);
        assert_eq!(start, 70);
        assert_eq!(all[start..].len(), 30);
    }

    #[test]
    fn test_log_tail_fewer_lines() {
        let log_tail_lines: usize = 30;
        let all: Vec<&str> = (0..10).map(|_| "line").collect();
        let start = all.len().saturating_sub(log_tail_lines);
        assert_eq!(start, 0);
        assert_eq!(all[start..].len(), 10);
    }
}
