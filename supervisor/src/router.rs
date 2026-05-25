// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! IPC router and display dispatcher — routes app stdout lines to the correct
//! subsystem: display pipeline, IPC broker, or terminal print.

use supervisor::logging::Subsystem;
use crate::{log_info, log_warn, AppRegistry, FocusedApp, Inbox, LAST_SENDER};

// ── route_or_print ────────────────────────────────────────────────────────────

pub fn route_or_print(
    line:         &str,
    sender:       &str,
    inbox:        &Inbox,
    has_display:  bool,
    win_region:   Option<(u32, u32, u32, u32)>,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) {
    if has_display {
        if let Some(cmd) = line.strip_prefix("VYOMA_DRAW:") {
            // Increment draw_ticks for CPU% tracking
            {
                let reg = app_registry.lock().unwrap();
                if let Some(st) = reg.get(sender) {
                    st.lock().unwrap().draw_ticks += 1;
                }
            }
            #[cfg(target_os = "linux")]
            {
                crate::draw_cmd::handle_draw_command(
                    cmd, sender, win_region, focused, app_registry,
                );
            }
            #[cfg(not(target_os = "linux"))]
            let _ = cmd;
            return;
        }
    }
    let _ = has_display;
    let _ = win_region;

    if let Some(rest) = line.strip_prefix('@') {
        if let Some((target, msg)) = rest.split_once(": ") {
            if target == "supervisor" {
                crate::ipc_handlers::handle_supervisor_command(
                    msg, sender, inbox, focused, app_registry,
                );
                return;
            }
            if supervisor::ipc::is_broadcast_target(target) {
                let map = inbox.lock().unwrap();
                for tx in map.values() {
                    let _ = tx.send(msg.to_string());
                }
                log_info!(
                    Subsystem::Ipc, Some(sender),
                    "broadcast: \"{}\" → {} app(s)", msg, map.len()
                );
                return;
            }
            // @reply: routes message back to whichever app last sent to the sender.
            if supervisor::ipc::is_reply_target(target) {
                let reply_target = LAST_SENDER
                    .get()
                    .and_then(|m| m.lock().ok())
                    .and_then(|map| map.get(sender).cloned());
                match reply_target {
                    Some(orig) => {
                        let map = inbox.lock().unwrap();
                        if let Some(tx) = map.get(&orig) {
                            let _ = tx.send(msg.to_string());
                        }
                    }
                    None => {
                        log_warn!(
                            Subsystem::Ipc, Some(sender),
                            "@reply: no last sender recorded for {sender}, message dropped"
                        );
                    }
                }
                return;
            }
            // Record that `sender` sent a message to `target` so `target` can @reply:.
            if let Some(ls) = LAST_SENDER.get() {
                if let Ok(mut map) = ls.lock() {
                    map.insert(target.to_string(), sender.to_string());
                }
            }
            let map = inbox.lock().unwrap();
            if let Some(tx) = map.get(target) {
                if tx.send(msg.to_string()).is_ok() {
                    return;
                }
            }
        }
    }
    println!("[{sender}] {line}");
}

// ── send_reply ────────────────────────────────────────────────────────────────

pub fn send_reply(target: &str, msg: &str, inbox: &Inbox) {
    if let Some(tx) = inbox.lock().unwrap().get(target) {
        let _ = tx.send(msg.to_string());
    }
}
