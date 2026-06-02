use crate::{send_reply, AppRegistry, FocusedApp, Inbox};
use crate::packages::{install_package, read_installed_apps, remove_package, PACKAGES};

pub fn handle(
    verb: &str, parts: &[&str], sender: &str,
    inbox: &Inbox, focused: &FocusedApp, app_registry: &AppRegistry,
) -> bool {
    match verb {
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
        _ => return false,
    }
    true
}
