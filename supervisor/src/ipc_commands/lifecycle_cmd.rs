use std::thread;
use crate::lock_or_recover;

use crate::{log_info, log_warn, send_reply, AppRegistry, FocusedApp, Inbox};
use supervisor::logging::Subsystem;

pub fn handle(
    verb: &str, parts: &[&str], sender: &str,
    inbox: &Inbox, focused: &FocusedApp, app_registry: &AppRegistry,
) -> bool {
    match verb {
        "shutdown" => {
            log_info!(Subsystem::Lifecycle, None, "shutdown requested by {sender}");
            match crate::session::save_session(app_registry, focused) {
                Ok(bytes) => log_info!(Subsystem::Lifecycle, None, "auto-saved session ({bytes} bytes) before shutdown"),
                Err(e) => log_warn!(Subsystem::Lifecycle, None, "session auto-save failed: {e}"),
            }
            send_reply(sender, "REPLY:shutting down...", inbox);
            thread::spawn(|| {
                thread::sleep(std::time::Duration::from_millis(500));
                #[cfg(target_os = "linux")]
                unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF); }
            });
        }
        "reboot" => {
            log_info!(Subsystem::Lifecycle, None, "reboot requested by {sender}");
            match crate::session::save_session(app_registry, focused) {
                Ok(bytes) => log_info!(Subsystem::Lifecycle, None, "auto-saved session ({bytes} bytes) before reboot"),
                Err(e) => log_warn!(Subsystem::Lifecycle, None, "session auto-save failed: {e}"),
            }
            send_reply(sender, "REPLY:rebooting...", inbox);
            thread::spawn(|| {
                thread::sleep(std::time::Duration::from_millis(500));
                #[cfg(target_os = "linux")]
                unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_RESTART); }
            });
        }
        "notify" => {
            let rest = parts.get(1).unwrap_or(&"").trim().to_string();
            let (title, msg) = rest.split_once('|')
                .map(|(a, b)| (a.to_string(), b.to_string()))
                .or_else(|| rest.split_once(' ').map(|(a, b)| (a.to_string(), b.to_string())))
                .unwrap_or_else(|| (rest.clone(), String::new()));
            let icon_path: Option<String> = {
                let reg = lock_or_recover(&app_registry);
                reg.get(sender).and_then(|st| {
                    let st = lock_or_recover(&st);
                    let manifest_path = &st.entry.manifest;
                    let raw = std::fs::read_to_string(manifest_path).ok()?;
                    let m: supervisor::manifest::AppManifest = toml::from_str(&raw).ok()?;
                    let icon_rel = m.app.icon?;
                    let parent = std::path::Path::new(manifest_path).parent()?;
                    Some(parent.join(icon_rel).to_string_lossy().to_string())
                })
            };
            crate::toast::enqueue_banner_with_icon(sender, &title, &msg, icon_path.as_deref());
            log_info!(Subsystem::Display, None, "notify title={title:?} msg={msg:?}");
            send_reply(sender, "REPLY:notified", inbox);
        }
        "session-save" => {
            match crate::session::save_session(app_registry, focused) {
                Ok(bytes) => {
                    log_info!(Subsystem::Lifecycle, None, "session saved ({bytes} bytes)");
                    send_reply(sender, "REPLY:session saved", inbox);
                }
                Err(e) => {
                    log_warn!(Subsystem::Lifecycle, None, "session save failed: {e}");
                    send_reply(sender, &format!("REPLY:error: session save failed: {e}"), inbox);
                }
            }
        }
        "session-restore" => {
            let entries = crate::session::restore_session();
            let restored = crate::session::apply_session(&entries, app_registry, focused, inbox);
            log_info!(Subsystem::Lifecycle, None, "session restored {restored} windows");
            send_reply(sender, &format!("REPLY:restored {restored} windows"), inbox);
        }
        _ => return false,
    }
    true
}
