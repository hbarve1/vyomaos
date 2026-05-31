// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! IPC handlers for `@supervisor: undo`, `redo`, `undo-history`.

use crate::{AppRegistry, FocusedApp, Inbox};

/// Handle undo/redo/undo-history commands.  Returns `true` if handled.
pub fn handle_undo_redo(
    verb:         &str,
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> bool {
    match verb {
        "undo" => {
            crate::undo::handle_undo_command("undo", sender, inbox);
            if let Some(cmd) = crate::undo::undo_command_to_execute() {
                crate::ipc_handlers::handle_supervisor_command(
                    &cmd, sender, inbox, focused, app_registry,
                );
            }
            true
        }
        "redo" => {
            crate::undo::handle_undo_command("redo", sender, inbox);
            if let Some(cmd) = crate::undo::redo_command_to_execute() {
                crate::ipc_handlers::handle_supervisor_command(
                    &cmd, sender, inbox, focused, app_registry,
                );
            }
            true
        }
        "undo-history" => {
            crate::undo::handle_undo_command("undo-history", sender, inbox);
            true
        }
        _ => false,
    }
}
