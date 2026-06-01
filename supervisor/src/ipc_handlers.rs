// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! @supervisor IPC command handler — process management commands (P12T03, P13T01, P16T01).
//! Extended commands (pkg-*, wallpaper, resize, tcp-*, …) are handled in `ipc_commands`.

use std::{fs, sync::Arc};

use sha2::{Digest, Sha256};

use crate::{
    log_info, log_warn, log_error,
    AppRegistry, AppStatus, FocusedApp, Inbox,
    BOOT_CONFIG_PATH, LOG_DIR, LOG_TAIL_LINES,
};
use crate::send_reply;
use crate::app_threads::{spawn_app, launch_app_threads};
use supervisor::lifecycle::format_ps_line;
use supervisor::logging::Subsystem;
use supervisor::manifest::{AppManifest, BootConfig, BootEntry};
use crate::net::http_get;

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
        // ── P12T03 legacy commands ────────────────────────────────────────────
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
                log_info!(Subsystem::Input, Some(name), "focus → {name}");
            }
        }

        // P54: win-info <app> — return window region of an app
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

        // P55: font-size <s|m|l> — store global font size preference
        "font-size" => {
            let size = parts.get(1).unwrap_or(&"m").trim().to_string();
            let valid = matches!(size.as_str(), "s" | "m" | "l");
            if valid {
                *crate::FONT_SIZE.get().unwrap().lock().unwrap() = size.clone();
                log_info!(Subsystem::Lifecycle, None, "font-size → {size}");
                send_reply(sender, &format!("REPLY:font-size {size}"), inbox);
            } else {
                send_reply(sender, "REPLY:font-size error invalid-size", inbox);
            }
        }

        // P52: input <char> — forward a character to the focused app's stdin
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
            let entry = BootEntry { manifest: path.clone(), restart: "never".to_string() };
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
        // ── P13T01 process management commands ───────────────────────────────
        // ps-raw — machine-readable: name:status:uptime_secs:restarts per entry
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

        // ps — list all apps with status, uptime, restart count, cpu%
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

        // kill <app> — send SIGKILL to a running app
        "kill" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: kill <app>", inbox);
                    return;
                }
            };
            let (pid, running_names, manifest_path) = {
                let reg = app_registry.lock().unwrap();
                let (pid, mfst) = reg.get(&app_name).map(|st| {
                    let s = st.lock().unwrap();
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
                    if let Some(st) = app_registry.lock().unwrap().get(&app_name) {
                        use crate::display::animator::{Animation, AnimKind, now_ms};
                        st.lock().unwrap().pending_anim = Some(Animation::new(AnimKind::Close, now_ms()));
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
        // restart <app> — kill + re-spawn an app
        "restart" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: restart <app>", inbox);
                    return;
                }
            };
            let (pid, entry) = {
                let reg = app_registry.lock().unwrap();
                match reg.get(&app_name) {
                    Some(st) => {
                        let st = st.lock().unwrap();
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

        // P30: update <app> <url> — download new wasm, verify sha256, hot-swap, restart
        "update" => {
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
                let reg = app_registry.lock().unwrap();
                reg.get(&app_name).map(|st| st.lock().unwrap().entry.clone())
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
                    let reg = registry_bg.lock().unwrap();
                    if let Some(st) = reg.get(&app_name) {
                        if let Some(pid) = st.lock().unwrap().child_pid {
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

        // reload — re-read boot.toml, launch any apps not currently running
        "reload" => {
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
                    let reg = app_registry.lock().unwrap();
                    reg.get(&name).map(|st| {
                        matches!(st.lock().unwrap().status, AppStatus::Running)
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
                &format!("REPLY:reload done — {launched} new app(s) started"),
                inbox,
            );
        }

        // log <app> — show last LOG_BUF_SIZE lines of an app's stdout
        "log" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: log <app>", inbox);
                    return;
                }
            };
            let reply = {
                let reg = app_registry.lock().unwrap();
                match reg.get(&app_name) {
                    Some(st) => {
                        let st = st.lock().unwrap();
                        if st.log_buf.is_empty() {
                            format!("REPLY:no output captured for {app_name}")
                        } else {
                            // Replace any | in log lines so they don't break the reply protocol
                            let lines: Vec<String> = st.log_buf.iter()
                                .map(|s| s.replace('|', " "))
                                .collect();
                            format!("REPLY:{}", lines.join("|"))
                        }
                    }
                    None => format!("REPLY:error: app {app_name} not found"),
                }
            };
            send_reply(sender, &reply, inbox);
        }

        // ── P16T01: persistent log commands ──────────────────────────────────

        // logs — list apps that have a log file in /data/logs/
        "logs" => {
            let mut names: Vec<String> = fs::read_dir(LOG_DIR)
                .map(|rd| {
                    rd.filter_map(|e| {
                        let e = e.ok()?;
                        let fname = e.file_name().into_string().ok()?;
                        fname.strip_suffix(".log").map(|s| s.to_string())
                    }).collect()
                })
                .unwrap_or_default();
            names.sort();
            if names.is_empty() {
                send_reply(sender, "REPLY:no logs yet (apps haven't produced output)", inbox);
            } else {
                send_reply(sender, &format!("REPLY:{}", names.join("|")), inbox);
            }
        }

        // logf <app> — last LOG_TAIL_LINES lines from /data/logs/<app>.log
        "logf" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: logf <app>", inbox);
                    return;
                }
            };
            let log_path = format!("{LOG_DIR}/{app_name}.log");
            match fs::read_to_string(&log_path) {
                Ok(content) => {
                    let all: Vec<&str> = content.lines().collect();
                    let start = all.len().saturating_sub(LOG_TAIL_LINES);
                    let recent: Vec<String> = all[start..]
                        .iter()
                        .map(|s| s.replace('|', " "))
                        .collect();
                    if recent.is_empty() {
                        send_reply(sender, &format!("REPLY:{app_name}.log is empty"), inbox);
                    } else {
                        send_reply(sender, &format!("REPLY:{}", recent.join("|")), inbox);
                    }
                }
                Err(_) => {
                    send_reply(sender, &format!("REPLY:no log file for {app_name}"), inbox);
                }
            }
        }

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

// ── Inline tests: command parsing patterns ──────────────────────────────────

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
