use crate::{log_info, send_reply, AppRegistry, Inbox, FocusedApp};
use crate::chrome::{z_order_push_back, z_order_push_front};
use supervisor::logging::Subsystem;
use crate::lock_or_recover;

pub fn handle(
    verb: &str, parts: &[&str], sender: &str,
    inbox: &Inbox, focused: &FocusedApp, app_registry: &AppRegistry,
) -> bool {
    match verb {
        "wallpaper" => {
            let arg = parts.get(1).unwrap_or(&"").trim();
            match crate::wallpaper::parse_arg(arg) {
                Some(wp @ crate::wallpaper::Wallpaper::SolidColor(rgba)) => {
                    crate::wallpaper::set(wp);
                    #[cfg(target_os = "linux")]
                    if let Some(fb_lock) = crate::display::get() {
                        let mut fb = lock_or_recover(&fb_lock);
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
                    let rgba = 0x0D1117FFu32;
                    crate::wallpaper::set(crate::wallpaper::Wallpaper::SolidColor(rgba));
                    #[cfg(target_os = "linux")]
                    if let Some(fb_lock) = crate::display::get() {
                        let mut fb = lock_or_recover(&fb_lock);
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
            *lock_or_recover(&focused) = Some(app_name.clone());
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
                        let (x, y) = lock_or_recover(&app_registry).get(&app_name)
                            .and_then(|st| lock_or_recover(&st).win_region.map(|(x, y, _, _)| (x, y)))
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
            let ok = lock_or_recover(&app_registry).get(&app_name)
                .map(|sa| { lock_or_recover(&sa).win_region = Some((nx, ny, nw, nh)); }).is_some();
            if !ok { send_reply(sender, &format!("REPLY:error: app {app_name} not found"), inbox); return true; }
            send_reply(&app_name, &format!("VYOMA_SYSTEM:resize:{nw},{nh}"), inbox);
            log_info!(Subsystem::Display, Some(app_name.as_str()), "resize {app_name} → ({nx},{ny},{nw},{nh})");
            send_reply(sender, &format!("REPLY:resized {app_name} to {nx},{ny},{nw}x{nh}"), inbox);
        }
        _ => return false,
    }
    true
}
