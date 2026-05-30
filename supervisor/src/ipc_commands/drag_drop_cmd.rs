// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! IPC command handlers for inter-app drag & drop: `drag-start`, `drag-cancel`.

use crate::{log_info, send_reply, Inbox};
use supervisor::logging::Subsystem;

/// Handle drag-and-drop IPC commands. Returns `true` if handled.
pub fn handle_drag_drop(
    verb:   &str,
    parts:  &[&str],
    sender: &str,
    inbox:  &Inbox,
) -> bool {
    match verb {
        "drag-start" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (mime, data) = match rest.split_once(' ') {
                Some((m, d)) => (m.trim(), d.trim()),
                None => {
                    send_reply(sender, "REPLY:error: usage: drag-start <mime> <data>", inbox);
                    return true;
                }
            };
            match crate::drag_drop::drag_start(sender, mime, data) {
                Ok(()) => {
                    log_info!(Subsystem::Ipc, Some(sender), "drag-start mime={mime} data={data}");
                    send_reply(sender, "REPLY:drag-start ok", inbox);
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:error: {e}"), inbox);
                }
            }
            true
        }
        "drag-cancel" => {
            let was_active = crate::drag_drop::drag_cancel();
            if was_active {
                log_info!(Subsystem::Ipc, Some(sender), "drag-cancel: cleared");
                send_reply(sender, "REPLY:drag-cancel ok", inbox);
            } else {
                send_reply(sender, "REPLY:drag-cancel: no active drag", inbox);
            }
            true
        }
        _ => false,
    }
}
