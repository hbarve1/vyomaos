use std::path::Path;
use std::sync::Arc;
use std::{fs, thread};

use crate::{log_info, send_reply, Inbox};
use crate::net::{dns_resolve_a, http_get};
use supervisor::logging::Subsystem;

pub fn handle(
    verb: &str, parts: &[&str], sender: &str, inbox: &Inbox,
) -> bool {
    match verb {
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
        _ => return false,
    }
    true
}
