use crate::{log_info, log_warn, send_reply, app_log_levels, AppRegistry, AppStatus, Inbox, BOOT_INSTANT, CLIPBOARD};
use supervisor::logging::Subsystem;

pub fn handle(
    verb: &str, parts: &[&str], sender: &str,
    inbox: &Inbox, app_registry: &AppRegistry,
) -> bool {
    match verb {
        "uptime" => {
            let secs = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
            let reply = supervisor::lifecycle::format_system_uptime(secs);
            send_reply(sender, &format!("REPLY:{reply}"), inbox);
            log_info!(Subsystem::Ipc, None, "uptime query from {sender}: {reply}");
        }
        "loglevel" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let sub_parts: Vec<&str> = rest.splitn(2, ' ').collect();
            if sub_parts.len() == 2 {
                let app_name = sub_parts[0].trim();
                let level_str = sub_parts[1].trim();
                if let Some(lvl) = supervisor::ipc::parse_log_level(level_str) {
                    app_log_levels().lock().unwrap().insert(app_name.to_string(), lvl);
                    log_info!(Subsystem::Ipc, Some(app_name),
                        "app={} log_level set to {:?}", app_name, lvl);
                } else {
                    log_warn!(Subsystem::Ipc, None, "loglevel: unknown level {:?}", level_str);
                }
            } else {
                log_warn!(Subsystem::Ipc, None,
                    "loglevel: usage: loglevel <app> <debug|info|warn|error>");
            }
        }
        "ping" => {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let reply = supervisor::ipc::format_pong_reply(ms);
            log_info!(Subsystem::Ipc, None, "ping from={sender} reply={reply}");
            send_reply(sender, &format!("REPLY:{reply}"), inbox);
        }
        "version" => {
            let v = supervisor::ipc::format_version(0, 19, 0);
            send_reply(sender, &format!("REPLY:{v}"), inbox);
            log_info!(Subsystem::Ipc, None, "version query from {sender}: {v}");
        }
        "screen-size" => {
            const DEFAULT_SCREEN_W: u32 = 1920;
            const DEFAULT_SCREEN_H: u32 = 1080;
            #[cfg(target_os = "linux")]
            let (w, h) = crate::display::screen_size().unwrap_or((DEFAULT_SCREEN_W, DEFAULT_SCREEN_H));
            #[cfg(not(target_os = "linux"))]
            let (w, h) = (DEFAULT_SCREEN_W, DEFAULT_SCREEN_H);
            let reply = format!("REPLY:{}x{}", w, h);
            log_info!(Subsystem::Display, None, "screen-size query from {sender}: {w}x{h}");
            send_reply(sender, &reply, inbox);
        }
        "monitors" => {
            let count = crate::net::count_drm_connectors();
            log_info!(Subsystem::Display, None, "monitors = {count}");
            send_reply(sender, &format!("REPLY:monitors {count}"), inbox);
        }
        "clipboard-set" => {
            let text = parts.get(1).unwrap_or(&"").trim().to_string();
            *CLIPBOARD.get().unwrap().lock().unwrap() = text;
            send_reply(sender, &supervisor::ipc::format_clipboard_set_reply(), inbox);
        }
        "clipboard-get" => {
            let text = CLIPBOARD.get().unwrap().lock().unwrap().clone();
            send_reply(sender, &supervisor::ipc::format_clipboard_get_reply(&text), inbox);
        }
        "clipboard-clear" => {
            CLIPBOARD.get().unwrap().lock().unwrap().clear();
            send_reply(sender, "REPLY:clipboard-clear ok", inbox);
        }
        "screenshot" => {
            #[cfg(target_os = "linux")]
            {
                match crate::display::get() {
                    Some(fb_lock) => {
                        let fb = fb_lock.lock().unwrap();
                        let explicit_path = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
                        let result = if let Some(p) = explicit_path {
                            fb.screenshot(p).map(|()| p.to_string())
                        } else {
                            crate::screenshot::capture_screenshot(&fb)
                        };
                        match result {
                            Ok(saved_path) => {
                                log_info!(Subsystem::Display, None, "screenshot saved to {saved_path}");
                                send_reply(sender, &format!("REPLY:screenshot ok {saved_path}"), inbox);
                                crate::toast::enqueue_banner(
                                    "supervisor",
                                    "Screenshot",
                                    &format!("Saved to {saved_path}"),
                                );
                            }
                            Err(e) => send_reply(sender, &format!("REPLY:screenshot error {e}"), inbox),
                        }
                    }
                    None => send_reply(sender, "REPLY:screenshot error no-display", inbox),
                }
            }
            #[cfg(not(target_os = "linux"))]
            send_reply(sender, "REPLY:screenshot error linux-only", inbox);
        }
        "locale" => {
            let code = parts.get(1).unwrap_or(&"").trim();
            if code.is_empty() {
                let cur = crate::i18n::current_locale();
                let all = crate::i18n::supported_locales().join(", ");
                send_reply(sender, &format!("REPLY:locale {cur} (available: {all})"), inbox);
            } else if crate::i18n::set_locale(code) {
                log_info!(Subsystem::Ipc, None, "locale switched to {code} by {sender}");
                send_reply(sender, &format!("REPLY:locale set to {code}"), inbox);
                let msg = format!("VYOMA_SYSTEM:locale:{code}");
                let reg = app_registry.lock().unwrap();
                let inb = inbox.lock().unwrap();
                for (name, state_arc) in reg.iter() {
                    let st = state_arc.lock().unwrap();
                    if matches!(st.status, AppStatus::Running) {
                        if let Some(tx) = inb.get(name) {
                            let _ = tx.send(msg.clone());
                        }
                    }
                }
            } else {
                let all = crate::i18n::supported_locales().join(", ");
                send_reply(
                    sender,
                    &format!("REPLY:error: unknown locale '{code}' (available: {all})"),
                    inbox,
                );
            }
        }
        "open" => {
            let path = match parts.get(1).map(|s| s.trim()) {
                Some(p) if !p.is_empty() => p.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: open <path>", inbox);
                    return true;
                }
            };
            match crate::file_assoc::app_for_file(&path) {
                Some(app_name) => {
                    let running = {
                        let reg = app_registry.lock().unwrap();
                        reg.get(&app_name)
                            .map(|st| matches!(st.lock().unwrap().status, AppStatus::Running))
                            .unwrap_or(false)
                    };
                    if running {
                        let msg = crate::file_assoc::format_open_message(&path);
                        send_reply(&app_name, &msg, inbox);
                        send_reply(sender, &format!("REPLY:open {path} with {app_name}"), inbox);
                    } else {
                        send_reply(sender, &format!("REPLY:error: app '{app_name}' not running"), inbox);
                    }
                }
                None => {
                    let ext = crate::file_assoc::extract_extension(&path)
                        .unwrap_or_else(|| "(none)".to_string());
                    send_reply(sender, &format!("REPLY:error: no app for .{ext}"), inbox);
                }
            }
        }
        "assoc-list" => {
            let assocs = crate::file_assoc::list_associations();
            let rows: Vec<String> = assocs.iter().map(|a| format!(".{} → {}", a.extension, a.app_name)).collect();
            if rows.is_empty() { send_reply(sender, "REPLY:no file associations", inbox); }
            else { send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox); }
        }
        "assoc-set" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let sub: Vec<&str> = rest.splitn(2, ' ').collect();
            if sub.len() < 2 || sub[0].is_empty() || sub[1].is_empty() {
                send_reply(sender, "REPLY:error: usage: assoc-set <ext> <app>", inbox);
                return true;
            }
            let ext = sub[0].trim_start_matches('.');
            let app = sub[1].trim();
            match crate::file_assoc::set_association(ext, app) {
                Ok(()) => send_reply(sender, &format!("REPLY:assoc .{ext} → {app}"), inbox),
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }
        _ => return false,
    }
    true
}
