// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! IPC handler for `@supervisor: theme <dark|light|auto>`.

use crate::{log_info, send_reply, AppRegistry, Inbox};
use supervisor::logging::Subsystem;

/// Handle the `theme` IPC command.  Returns `true` (always handled).
pub fn handle_theme_command(
    parts: &[&str],
    sender: &str,
    inbox: &Inbox,
    app_registry: &AppRegistry,
) -> bool {
    let name = parts.get(1).unwrap_or(&"").trim();
    if name.is_empty() {
        let cur = crate::theme::active_name();
        send_reply(sender, &format!("REPLY:theme {cur}"), inbox);
    } else {
        let old_name = crate::theme::active_name().to_string();
        if crate::theme::set_active(name) {
            crate::undo::capture_theme(&old_name, name);
            log_info!(Subsystem::Display, None, "theme switched to {name}");
            crate::theme::broadcast_theme_change(name, inbox, app_registry);
            send_reply(sender, &format!("REPLY:theme {name}"), inbox);
        } else if crate::theme::VALID_NAMES.contains(&name) {
            send_reply(sender, &format!("REPLY:theme {name}"), inbox);
        } else {
            send_reply(sender, "REPLY:error: usage: theme <dark|light|auto>", inbox);
        }
    }
    true
}
