// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Extended @supervisor IPC commands: pkg-*, wallpaper, resize, tcp-*, clipboard,
//! session-*, display, network helpers, uptime, loglevel, ping, version, workspace, lock.

mod audio_ipc;
mod drag_drop_cmd;
mod focus_cmd;
mod lock;
mod share_cmd;
mod tcp;
mod theme_cmd;
mod websocket;
mod undo_cmd;
mod workspace_cmd;
use std::{fs, path::Path, sync::Arc, thread};

use crate::{
    log_info, log_warn,
    AppRegistry, FocusedApp, Inbox,
    CLIPBOARD, BOOT_INSTANT,
};
use crate::packages::PACKAGES;
use crate::chrome::{z_order_push_back, z_order_push_front};
use crate::net::{count_drm_connectors, dns_resolve_a, http_get};
use crate::packages::{install_package, read_installed_apps, remove_package};
use crate::{send_reply, app_log_levels};
use supervisor::logging::Subsystem;

/// Handle an extended @supervisor command. Returns `true` if handled, `false` if unknown.
pub fn handle_extended_command(
    verb:         &str,
    parts:        &[&str],
    sender:       &str,
    inbox:        &Inbox,
    focused:      &FocusedApp,
    app_registry: &AppRegistry,
) -> bool {
    // P89: audit every IPC command reaching the extended handler.
    let detail = parts.get(1).unwrap_or(&"").trim();
    crate::audit::log_audit(
        sender,
        "ipc",
        &format!("{verb} {detail}"),
        crate::audit::AuditResult::Allowed,
    );

    match verb {
        // P14T01: package manager commands
        "pkg-list" => {
            let installed = read_installed_apps();
            let rows: Vec<String> = PACKAGES.iter().map(|(name, ver, desc)| {
                let mark = if installed.iter().any(|n| n == name) { "+" } else { " " };
                format!("[{mark}] {name} v{ver}  {desc}")
            }).collect();
            if rows.is_empty() {
                send_reply(sender, "REPLY:no packages in catalog", inbox);
            } else {
                send_reply(sender, &format!("REPLY:{}", rows.join("|")), inbox);
            }
        }

        "pkg-installed" => {
            let installed = read_installed_apps();
            if installed.is_empty() {
                send_reply(sender, "REPLY:no packages installed", inbox);
            } else {
                send_reply(sender, &format!("REPLY:{}", installed.join("|")), inbox);
            }
        }

        "pkg-install" => {
            let name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: pkg install <name>", inbox);
                    return true;
                }
            };
            if !PACKAGES.iter().any(|(n, _, _)| *n == name.as_str()) {
                send_reply(sender, &format!("REPLY:error: unknown package '{name}'  (try: pkg list)"), inbox);
                return true;
            }
            if read_installed_apps().iter().any(|n| n == &name) {
                send_reply(sender, &format!("REPLY:{name} is already installed"), inbox);
                return true;
            }
            match install_package(&name, inbox, focused, app_registry) {
                Ok(()) => send_reply(sender, &format!("REPLY:installed {name} — now running"), inbox),
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }

        "pkg-remove" => {
            let name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: pkg remove <name>", inbox);
                    return true;
                }
            };
            if !read_installed_apps().iter().any(|n| n == &name) {
                send_reply(sender, &format!("REPLY:error: '{name}' is not installed"), inbox);
                return true;
            }
            match remove_package(&name, app_registry) {
                Ok(()) => send_reply(sender, &format!("REPLY:removed {name}"), inbox),
                Err(e) => send_reply(sender, &format!("REPLY:error: {e}"), inbox),
            }
        }

        // P35: wallpaper <rgba_hex> or wallpaper <path_to_png>
        "wallpaper" => {
            let arg = parts.get(1).unwrap_or(&"").trim();
            match crate::wallpaper::parse_arg(arg) {
                Some(wp @ crate::wallpaper::Wallpaper::SolidColor(rgba)) => {
                    crate::wallpaper::set(wp);
                    #[cfg(target_os = "linux")]
                    if let Some(fb_lock) = crate::display::get() {
                        let mut fb = fb_lock.lock().unwrap();
                        let (w, h) = (fb.width, fb.height);
                        fb.fill_rect(0, 0, w, h, rgba);
                        fb.flush();
                    }
                    log_info!(Subsystem::Display, None, "wallpaper set to {rgba:#010x}");
                    send_reply(sender, &format!("REPLY:wallpaper {rgba:#010x}"), inbox);
                }
                Some(crate::wallpaper::Wallpaper::Image(ref path)) => {
                    let reply_path = path.clone();
                    crate::wallpaper::set(crate::wallpaper::Wallpaper::Image(reply_path.clone()));
                    log_info!(Subsystem::Display, None, "wallpaper set to image {reply_path}");
                    send_reply(sender, &format!("REPLY:wallpaper {reply_path}"), inbox);
                }
                None => {
                    // Fallback to default solid color
                    let rgba = 0x0D1117FFu32;
                    crate::wallpaper::set(crate::wallpaper::Wallpaper::SolidColor(rgba));
                    #[cfg(target_os = "linux")]
                    if let Some(fb_lock) = crate::display::get() {
                        let mut fb = fb_lock.lock().unwrap();
                        let (w, h) = (fb.width, fb.height);
                        fb.fill_rect(0, 0, w, h, rgba);
                        fb.flush();
                    }
                    log_info!(Subsystem::Display, None, "wallpaper set to {rgba:#010x}");
                    send_reply(sender, &format!("REPLY:wallpaper {rgba:#010x}"), inbox);
                }
            }
        }
        "raise" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: raise <app>", inbox);
                    return true;
                }
            };
            z_order_push_front(&app_name);
            *focused.lock().unwrap() = Some(app_name.clone());
            send_reply(sender, &format!("REPLY:raised {app_name}"), inbox);
        }
        "lower" => {
            let app_name = match parts.get(1).map(|s| s.trim()) {
                Some(n) if !n.is_empty() => n.to_string(),
                _ => {
                    send_reply(sender, "REPLY:error: usage: lower <app>", inbox);
                    return true;
                }
            };
            z_order_push_back(&app_name);
            send_reply(sender, &format!("REPLY:lowered {app_name}"), inbox);
        }
        // P36: resize <app> <x>,<y>,<w>,<h>  or  resize <app> <w> <h> (legacy)
        "resize" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (app_name, geom_str) = rest.split_once(' ').unwrap_or(("", ""));
            let app_name = app_name.trim().to_string();
            if app_name.is_empty() || geom_str.is_empty() {
                send_reply(sender, "REPLY:error: usage: resize <app> <x>,<y>,<w>,<h>", inbox);
                return true;
            }
            let cp: Vec<&str> = geom_str.split(',').collect();
            let geom = if cp.len() == 4 {
                match (cp[0].parse(), cp[1].parse(), cp[2].parse::<u32>(), cp[3].parse::<u32>()) {
                    (Ok(x), Ok(y), Ok(w), Ok(h)) if w > 0 && h > 0 => Some((x, y, w, h)),
                    _ => None,
                }
            } else if let Some((ws, hs)) = geom_str.split_once(' ') {
                match (ws.trim().parse::<u32>(), hs.trim().parse::<u32>()) {
                    (Ok(w), Ok(h)) if w > 0 && h > 0 => {
                        let (x, y) = app_registry.lock().unwrap().get(&app_name)
                            .and_then(|st| st.lock().unwrap().win_region.map(|(x, y, _, _)| (x, y)))
                            .unwrap_or((0, 0));
                        Some((x, y, w, h))
                    }
                    _ => None,
                }
            } else { None };
            let (nx, ny, nw, nh) = match geom {
                Some(g) => g,
                None => { send_reply(sender, "REPLY:error: usage: resize <app> <x>,<y>,<w>,<h>", inbox); return true; }
            };
            let ok = app_registry.lock().unwrap().get(&app_name)
                .map(|sa| { sa.lock().unwrap().win_region = Some((nx, ny, nw, nh)); }).is_some();
            if !ok { send_reply(sender, &format!("REPLY:error: app {app_name} not found"), inbox); return true; }
            send_reply(&app_name, &format!("VYOMA_SYSTEM:resize:{nw},{nh}"), inbox);
            log_info!(Subsystem::Display, Some(app_name.as_str()), "resize {app_name} → ({nx},{ny},{nw},{nh})");
            send_reply(sender, &format!("REPLY:resized {app_name} to {nx},{ny},{nw}x{nh}"), inbox);
        }

        // P41: shutdown / reboot (auto-save session before executing)
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
            // T067: Extract sender icon path from boot entry manifest for banner icon
            let icon_path: Option<String> = {
                let reg = app_registry.lock().unwrap();
                reg.get(sender).and_then(|st| {
                    let st = st.lock().unwrap();
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

        // P42: session-save / session-restore (delegated to session module)
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

        // P43: monitors — count DRM connectors
        "monitors" => {
            let count = count_drm_connectors();
            log_info!(Subsystem::Display, None, "monitors = {count}");
            send_reply(sender, &format!("REPLY:monitors {count}"), inbox);
        }

        // P44: dns-resolve <hostname>
        "dns-resolve" => {
            let hostname = parts.get(1).unwrap_or(&"").trim().to_string();
            if hostname.is_empty() {
                send_reply(sender, "REPLY:dns error: no hostname", inbox);
                return true;
            }
            let ip = dns_resolve_a(&hostname).unwrap_or_else(|| "NXDOMAIN".to_string());
            log_info!(Subsystem::Ipc, None, "dns {hostname} -> {ip}");
            send_reply(sender, &format!("REPLY:dns {hostname} {ip}"), inbox);
        }

        // P45: tls-info
        "tls-info" => {
            let cert = Path::new("/data/cert.pem").exists();
            let key  = Path::new("/data/key.pem").exists();
            let msg = if cert && key {
                "TLS: cert.pem and key.pem present in /data — ready for TLS termination proxy"
            } else {
                "TLS: no cert/key found — place cert.pem and key.pem in /data/ to enable TLS"
            };
            send_reply(sender, &format!("REPLY:{msg}"), inbox);
        }

        // P46: http-get <url>
        "http-get" => {
            let url = parts.get(1).unwrap_or(&"").trim().to_string();
            if url.is_empty() {
                send_reply(sender, "REPLY:http-get error no-url", inbox);
                return true;
            }
            match http_get(&url) {
                Ok(bytes) => {
                    let raw = String::from_utf8_lossy(&bytes);
                    let status_code = raw.lines().next()
                        .and_then(|l| l.split_whitespace().nth(1))
                        .unwrap_or("200")
                        .to_string();
                    let body = if let Some(p) = raw.find("\r\n\r\n") { &raw[p + 4..] }
                              else if let Some(p) = raw.find("\n\n") { &raw[p + 2..] }
                              else { &raw };
                    let escaped: String = body.chars().take(4096)
                        .collect::<String>()
                        .replace('\r', "")
                        .replace('\n', "\\n");
                    send_reply(sender, &format!("REPLY:http-get {status_code} {escaped}"), inbox);
                }
                Err(e) => send_reply(sender, &format!("REPLY:http-get error {e}"), inbox),
            }
        }

        // P47: TCP connection pool — delegated to tcp submodule
        "tcp-connect" | "tcp-send" | "tcp-recv" | "tcp-close" => {
            return tcp::handle_tcp(verb, parts, sender, inbox);
        }

        // P50: WebSocket connections — delegated to websocket submodule
        "ws-connect" | "ws-send" | "ws-close" => {
            return websocket::handle_ws(verb, parts, sender, inbox);
        }

        // P50: clipboard
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

        // P51: screenshot [path] — PNG to /data/screenshots/ (auto) or given path
        "screenshot" => {
            #[cfg(target_os = "linux")]
            {
                match crate::display::get() {
                    Some(fb_lock) => {
                        let fb = fb_lock.lock().unwrap();
                        let explicit_path = parts.get(1).map(|s| s.trim()).filter(|s| !s.is_empty());
                        let result = if let Some(p) = explicit_path {
                            // Legacy PPM path for explicit destinations
                            fb.screenshot(p).map(|()| p.to_string())
                        } else {
                            // Auto PNG capture via screenshot module
                            crate::screenshot::capture_screenshot(&fb)
                        };
                        match result {
                            Ok(saved_path) => {
                                log_info!(Subsystem::Display, None, "screenshot saved to {saved_path}");
                                send_reply(sender, &format!("REPLY:screenshot ok {saved_path}"), inbox);
                                // Fire a notification banner
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

        // P49: download <url> <dest>
        "download" => {
            let rest = parts.get(1).unwrap_or(&"").trim().to_string();
            let (url, dest) = match rest.split_once(' ') {
                Some((u, d)) => (u.trim().to_string(), d.trim().to_string()),
                None => {
                    send_reply(sender, "REPLY:download-error  missing dest", inbox);
                    return true;
                }
            };
            if url.is_empty() || dest.is_empty() {
                send_reply(sender, "REPLY:download-error  missing url or dest", inbox);
                return true;
            }
            let sender_name = sender.to_string();
            let inbox_clone = Arc::clone(inbox);
            let dest_clone  = dest.clone();
            thread::spawn(move || {
                let dest = dest_clone;
                send_reply(&sender_name, &format!("REPLY:download-progress {dest} 0"), &inbox_clone);
                match http_get(&url) {
                    Ok(bytes) => {
                        let n = bytes.len();
                        send_reply(&sender_name, &format!("REPLY:download-progress {dest} {n}"), &inbox_clone);
                        match fs::write(&dest, &bytes) {
                            Ok(()) => {
                                log_info!(Subsystem::Lifecycle, None, "download done {dest} ({n} bytes)");
                                send_reply(&sender_name, &format!("REPLY:download-done {dest}"), &inbox_clone);
                            }
                            Err(e) => send_reply(&sender_name, &format!("REPLY:download-error {dest} {e}"), &inbox_clone),
                        }
                    }
                    Err(e) => send_reply(&sender_name, &format!("REPLY:download-error {dest} {e}"), &inbox_clone),
                }
            });
            send_reply(sender, &format!("REPLY:download-progress {dest} 0"), inbox);
        }

        // uptime
        "uptime" => {
            let secs = BOOT_INSTANT.get().map(|i| i.elapsed().as_secs()).unwrap_or(0);
            let reply = supervisor::lifecycle::format_system_uptime(secs);
            send_reply(sender, &format!("REPLY:{reply}"), inbox);
            log_info!(Subsystem::Ipc, None, "uptime query from {sender}: {reply}");
        }

        // 023: loglevel <app> <level>
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

        // ping — reply with "pong <timestamp_ms>"
        "ping" => {
            let ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let reply = supervisor::ipc::format_pong_reply(ms);
            log_info!(Subsystem::Ipc, None, "ping from={sender} reply={reply}");
            send_reply(sender, &format!("REPLY:{reply}"), inbox);
        }

        // version
        "version" => {
            let v = supervisor::ipc::format_version(0, 19, 0);
            send_reply(sender, &format!("REPLY:{v}"), inbox);
            log_info!(Subsystem::Ipc, None, "version query from {sender}: {v}");
        }

        // T017 [US2]: screen-size — reply with current framebuffer dimensions
        "screen-size" => {
            const DEFAULT_SCREEN_W: u32 = 1440;
            const DEFAULT_SCREEN_H: u32 = 900;
            #[cfg(target_os = "linux")]
            let (w, h) = crate::display::screen_size().unwrap_or((DEFAULT_SCREEN_W, DEFAULT_SCREEN_H));
            #[cfg(not(target_os = "linux"))]
            let (w, h) = (DEFAULT_SCREEN_W, DEFAULT_SCREEN_H);
            let reply = format!("REPLY:{}x{}", w, h);
            log_info!(Subsystem::Display, None, "screen-size query from {sender}: {w}x{h}");
            send_reply(sender, &reply, inbox);
        }

        // P30: update-local <app> <wasm_path> — hot-swap WASM from local file
        "update-local" => {
            crate::ota_update::handle_update_local(parts, sender, inbox, focused, app_registry);
        }

        // Auto-update: check catalog and apply updates
        "check-updates" => {
            crate::auto_update::handle_check_updates(sender, inbox, app_registry);
        }
        "auto-update" => {
            crate::auto_update::handle_auto_update(parts, sender, inbox, focused, app_registry);
        }
        // Audio subsystem IPC commands
        "volume" | "volume-get" | "mute" | "unmute" => {
            return audio_ipc::handle_audio_ipc(verb, parts, sender, inbox);
        }

        // Workspace commands
        "workspace" | "workspace-move" | "workspace-get" => {
            return workspace_cmd::handle_workspace_command(
                verb, parts, sender, inbox, focused, app_registry,
            );
        }

        // Theme switching: theme <dark|light|auto>
        "theme" => {
            return theme_cmd::handle_theme_command(parts, sender, inbox, app_registry);
        }

        "lock" | "unlock" => {
            lock::handle_lock_command(verb, sender, inbox, focused, app_registry);
        }

        // Drag & drop between apps
        "drag-start" | "drag-cancel" => {
            return drag_drop_cmd::handle_drag_drop(verb, &parts, sender, inbox);
        }

        // P64: Share sheet between apps
        "share" | "share-accept" | "share-cancel" => {
            return share_cmd::handle_share(verb, parts, sender, inbox, app_registry);
        }

        // i18n: switch active locale
        "locale" => {
            let code = parts.get(1).unwrap_or(&"").trim();
            if code.is_empty() {
                let cur = crate::i18n::current_locale();
                let all = crate::i18n::supported_locales().join(", ");
                send_reply(sender, &format!("REPLY:locale {cur} (available: {all})"), inbox);
            } else if crate::i18n::set_locale(code) {
                log_info!(Subsystem::Ipc, None, "locale switched to {code} by {sender}");
                send_reply(sender, &format!("REPLY:locale set to {code}"), inbox);
                // Broadcast locale change to all running apps.
                let msg = format!("VYOMA_SYSTEM:locale:{code}");
                let reg = app_registry.lock().unwrap();
                let inb = inbox.lock().unwrap();
                for (name, state_arc) in reg.iter() {
                    let st = state_arc.lock().unwrap();
                    if matches!(st.status, crate::AppStatus::Running) {
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

        // Accessibility commands
        "a11y" => {
            return crate::accessibility::handle_a11y_command(
                parts, sender, inbox, app_registry,
            );
        }

        // Undo / redo / undo-history
        "undo" | "redo" | "undo-history" => {
            return undo_cmd::handle_undo_redo(verb, sender, inbox, focused, app_registry);
        }

        // P52: file associations
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
                            .map(|st| matches!(st.lock().unwrap().status, crate::AppStatus::Running))
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

        // P51: VFS commands
        "vfs-list" | "vfs-stat" | "vfs-mounts" => {
            return crate::vfs::handle_vfs_command(verb, parts, sender, inbox, app_registry);
        }

        // P87: encrypted storage
        "encrypt" | "decrypt" | "encrypted-write" => {
            return crate::encrypted_store::handle_encrypted_command(verb, parts, sender, inbox);
        }

        // P55: archive support (tar.gz extraction, archive listing)
        "extract" | "archive-list" => {
            return crate::archive_ipc::handle_archive_command(verb, parts, sender, inbox);
        }

        // P57: trash (recycle bin)
        "trash" | "trash-list" | "trash-restore" | "trash-empty" | "trash-size" => {
            return crate::trash::handle_trash_command(verb, parts, sender, inbox);
        }

        // P59: package registry
        "registry-search" | "registry-list" | "registry-add" => {
            return crate::pkg_registry::handle_registry_command(verb, parts, sender, inbox);
        }

        // P89: audit log commands
        "audit-list" | "audit-search" | "audit-clear" => {
            return crate::audit::handle_audit_command(verb, &parts, sender, inbox);
        }

        // P60: runtime capability requests
        "request-cap" => {
            crate::cap_request::handle_request_cap(parts, sender, inbox, app_registry);
        }
        "revoke-cap" => {
            crate::cap_request::handle_revoke_cap(parts, sender, inbox, app_registry);
        }
        "list-caps" => {
            crate::cap_request::handle_list_caps(parts, sender, inbox);
        }

        // P82: keyboard-only focus cycling via IPC (delegated to focus_cmd)
        "focus-next" | "focus-prev" => {
            return focus_cmd::handle_focus_command(verb, sender, inbox, focused, app_registry);
        }

        // P88: per-app firewall rules
        "firewall-list" | "firewall-add" | "firewall-remove" | "firewall-check" => {
            return crate::firewall::handle_firewall_command(verb, parts, sender, inbox);
        }

        // P94: memory pressure manager commands
        "memory-info" | "memory-apps" | "memory-pressure" => {
            return crate::memory::handle_memory_command(verb, sender, inbox, app_registry);
        }

        // P63: background service commands
        "bg-list" | "bg-start" | "bg-stop" => {
            return crate::bg_service::handle_bg_command(
                verb, parts, sender, inbox, focused, app_registry,
            );
        }

        // P85: user account commands
        "user-list" | "user-add" | "user-remove" | "user-login" | "user-logout"
        | "user-whoami" => {
            return crate::user::handle_user_command(verb, parts, sender, inbox);
        }

        // P86: per-user capability tokens
        "user-caps" | "user-set-quota" => {
            return crate::user_caps::handle_user_caps_command(verb, parts, sender, inbox);
        }
        _ => return false,
    }
    true
}
