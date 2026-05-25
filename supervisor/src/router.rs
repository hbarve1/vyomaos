// IPC router + display dispatcher — extracted from main.rs (spec-044 line-limit refactor).
//
// `route_or_print` dispatches each line of app stdout to the appropriate handler:
//   - VYOMA_DRAW: commands → display subsystem
//   - @supervisor: commands → ipc_handlers::handle_supervisor_command
//   - @__mgmt__: replies → management server exec handler
//   - @broadcast: or @all: → all app inboxes
//   - @reply: → LAST_SENDER routing
//   - @<app>: → specific app inbox
//   - anything else → println to console

use crate::{
    AppRegistry, FocusedApp, Inbox,
    EXEC_REPLY_CHANNELS, LAST_SENDER,
};
use supervisor::logging::Subsystem;

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
            {
                let reg = app_registry.lock().unwrap();
                if let Some(st) = reg.get(sender) {
                    st.lock().unwrap().draw_ticks += 1;
                }
            }
            #[cfg(target_os = "linux")]
            {
                crate::draw_cmd::handle_draw_command(cmd, sender, win_region, focused, app_registry);
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
            // spec-044: route replies to the management exec handler.
            if target == "__mgmt__" {
                if let Some(channels) = EXEC_REPLY_CHANNELS.get() {
                    let map = channels.lock().unwrap();
                    if let Some(tx) = map.get(sender) {
                        let _ = tx.send(msg.to_string());
                    }
                }
                return;
            }
            if supervisor::ipc::is_broadcast_target(target) {
                let map = inbox.lock().unwrap();
                for tx in map.values() {
                    let _ = tx.send(msg.to_string());
                }
                crate::log_info!(
                    Subsystem::Ipc, Some(sender),
                    "broadcast: \"{}\" → {} app(s)", msg, map.len()
                );
                return;
            }
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
                        crate::log_warn!(
                            Subsystem::Ipc, Some(sender),
                            "@reply: no last sender recorded for {sender}, message dropped"
                        );
                    }
                }
                return;
            }
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

pub fn send_reply(target: &str, msg: &str, inbox: &Inbox) {
    if let Some(tx) = inbox.lock().unwrap().get(target) {
        let _ = tx.send(msg.to_string());
    }
}
