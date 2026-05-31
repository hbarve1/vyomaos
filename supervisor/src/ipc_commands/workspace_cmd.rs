// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Workspace IPC commands: workspace, workspace-move, workspace-get.

use crate::{log_info, send_reply, AppRegistry, FocusedApp, Inbox};
use supervisor::logging::Subsystem;

/// Handle workspace-related @supervisor commands.
/// Returns `true` if handled, `false` if verb is not a workspace command.
pub fn handle_workspace_command(
    verb:         &str,
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> bool {
    match verb {
        "workspace" => {
            let idx_str = parts.get(1).unwrap_or(&"").trim();
            if idx_str.is_empty() {
                let ws = crate::workspace::current();
                send_reply(sender, &format!("REPLY:workspace {ws}"), inbox);
            } else if let Ok(idx) = idx_str.parse::<usize>() {
                let old_ws = crate::workspace::current();
                let actual = crate::workspace::switch_to(idx);
                crate::undo::capture_workspace(old_ws, actual);
                log_info!(Subsystem::Display, None, "workspace switched to {actual} by {sender}");
                send_reply(sender, &format!("REPLY:workspace {actual}"), inbox);
                #[cfg(target_os = "linux")]
                crate::draw_cmd::force_repaint(app_registry, focused);
            } else {
                send_reply(sender, "REPLY:error: workspace <n> — n must be a number", inbox);
            }
        }

        "workspace-move" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let sub_parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if sub_parts.len() == 2 {
                let app_name = sub_parts[0].trim();
                if let Ok(ws) = sub_parts[1].trim().parse::<usize>() {
                    crate::workspace::move_app_to(app_name, ws);
                    let actual = crate::workspace::app_workspace_index(app_name);
                    log_info!(Subsystem::Display, Some(app_name),
                        "moved {app_name} to workspace {actual}");
                    send_reply(sender, &format!("REPLY:moved {app_name} to workspace {actual}"), inbox);
                    #[cfg(target_os = "linux")]
                    crate::draw_cmd::force_repaint(app_registry, focused);
                } else {
                    send_reply(sender, "REPLY:error: workspace-move <app> <n>", inbox);
                }
            } else {
                send_reply(sender, "REPLY:error: usage: workspace-move <app> <n>", inbox);
            }
        }

        "workspace-get" => {
            let ws = crate::workspace::current();
            let count = crate::workspace::count();
            send_reply(sender, &format!("REPLY:workspace {ws}/{count}"), inbox);
        }

        _ => return false,
    }
    true
}
