// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Log-related IPC handlers: log, logs, logf.

use std::fs;
use crate::lock_or_recover;

use crate::{AppRegistry, Inbox, LOG_DIR, LOG_TAIL_LINES};
use crate::send_reply;

/// Handle `log <app>` -- show last LOG_BUF_SIZE lines of an app's stdout.
pub fn handle_log(
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    app_registry: &AppRegistry,
) {
    let app_name = match parts.get(1).map(|s| s.trim()) {
        Some(n) if !n.is_empty() => n.to_string(),
        _ => {
            send_reply(sender, "REPLY:error: usage: log <app>", inbox);
            return;
        }
    };
    let reply = {
        let reg = lock_or_recover(&app_registry);
        match reg.get(&app_name) {
            Some(st) => {
                let st = lock_or_recover(&st);
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

/// Handle `logs` -- list apps that have a log file in /data/logs/.
pub fn handle_logs(
    sender: &str,
    inbox:  &Inbox,
) {
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

/// Handle `logf <app>` -- last LOG_TAIL_LINES lines from /data/logs/<app>.log.
pub fn handle_logf(
    parts:  &[&str],
    sender: &str,
    inbox:  &Inbox,
) {
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
