// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P82: IPC commands for keyboard-only focus cycling (`focus-next`, `focus-prev`).

use crate::{AppRegistry, FocusedApp, Inbox};
use crate::lock_or_recover;
use crate::chrome::{
    cycle_focus_backward, cycle_focus_forward, repaint_all_borders,
    windowed_apps_sorted, z_order_push_front,
};
use crate::{log_info, send_reply};
use supervisor::logging::Subsystem;

/// Handle `focus-next` or `focus-prev` IPC command.  Returns `true` if handled.
pub fn handle_focus_command(
    verb:         &str,
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> bool {
    let names = windowed_apps_sorted(app_registry);
    if names.is_empty() {
        send_reply(sender, "REPLY:no windowed apps", inbox);
        return true;
    }

    let cur = lock_or_recover(&focused).clone();
    let next = match verb {
        "focus-next" => cycle_focus_forward(&names, cur.as_deref()),
        "focus-prev" => cycle_focus_backward(&names, cur.as_deref()),
        _ => return false,
    };

    if let Some(ref name) = next {
        z_order_push_front(name);
        *lock_or_recover(&focused) = Some(name.clone());
        log_info!(Subsystem::Input, Some(name.as_str()), "{verb} → {name}");
        send_reply(sender, &format!("REPLY:focus {name}"), inbox);
        repaint_all_borders(app_registry, focused);
    } else {
        send_reply(sender, "REPLY:focus unchanged", inbox);
    }
    true
}
