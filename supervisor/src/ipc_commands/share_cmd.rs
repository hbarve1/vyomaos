// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! IPC command handlers for the share sheet: `share`, `share-accept`, `share-cancel`.

use crate::{log_info, log_warn, send_reply, AppRegistry, AppStatus, Inbox};
use supervisor::logging::Subsystem;
use crate::lock_or_recover;

/// Handle share-sheet IPC commands. Returns `true` if handled.
pub fn handle_share(
    verb:         &str,
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    app_registry: &AppRegistry,
) -> bool {
    match verb {
        "share" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (mime, data) = match rest.split_once(' ') {
                Some((m, d)) => (m.trim(), d.trim()),
                None => {
                    send_reply(
                        sender,
                        "REPLY:error: usage: share <mime> <data>",
                        inbox,
                    );
                    return true;
                }
            };
            match crate::share::share_start(sender, mime, data) {
                Ok(()) => {
                    log_info!(
                        Subsystem::Ipc,
                        Some(sender),
                        "share: mime={mime} data_len={}",
                        data.len()
                    );
                    send_reply(sender, "REPLY:share ok", inbox);
                    // Broadcast share-available to all running display apps
                    let msg = crate::share::format_share_available(mime);
                    broadcast_to_display_apps(&msg, inbox, app_registry);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
            true
        }
        "share-accept" => {
            match crate::share::share_accept() {
                Ok(payload) => {
                    let msg = crate::share::format_share_data(&payload);
                    send_reply(sender, &msg, inbox);
                    log_info!(
                        Subsystem::Ipc,
                        Some(sender),
                        "share-accept: mime={} from={}",
                        payload.mime,
                        payload.source_app
                    );
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                    log_warn!(Subsystem::Ipc, Some(sender), "share-accept failed: {e}");
                }
            }
            true
        }
        "share-cancel" => {
            let was_active = crate::share::share_cancel();
            if was_active {
                log_info!(Subsystem::Ipc, Some(sender), "share-cancel: cleared");
                send_reply(sender, "REPLY:share-cancel ok", inbox);
            } else {
                send_reply(sender, "REPLY:share-cancel: no pending share", inbox);
            }
            true
        }
        _ => false,
    }
}

/// Broadcast a message to all running apps that have `display = true`.
fn broadcast_to_display_apps(msg: &str, inbox: &Inbox, app_registry: &AppRegistry) {
    let reg = lock_or_recover(&app_registry);
    let inb = lock_or_recover(&inbox);
    for (name, state_arc) in reg.iter() {
        let st = lock_or_recover(&state_arc);
        if st.has_display && matches!(st.status, AppStatus::Running) {
            if let Some(tx) = inb.get(name) {
                let _ = tx.send(msg.to_string());
            }
        }
    }
}
