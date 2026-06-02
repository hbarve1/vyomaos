// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use crate::{log_info, Inbox, TCP_CONNS, TCP_NEXT_ID};
use crate::send_reply;
use supervisor::logging::Subsystem;
use crate::lock_or_recover;

/// Handle TCP connection pool commands. Returns `true` if handled, `false` if unknown.
pub fn handle_tcp(verb: &str, parts: &[&str], sender: &str, inbox: &Inbox) -> bool {
    match verb {
        "tcp-connect" => {
            let addr = parts.get(1).unwrap_or(&"").trim().to_string();
            if addr.is_empty() {
                send_reply(sender, "REPLY:tcp-connect error no-addr", inbox);
                return true;
            }
            match std::net::TcpStream::connect(&addr) {
                Ok(stream) => {
                    let _ = stream.set_read_timeout(Some(std::time::Duration::from_millis(200)));
                    let id = TCP_NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    lock_or_recover(&TCP_CONNS.get().unwrap()).insert(id, stream);
                    log_info!(Subsystem::Ipc, None, "tcp-connect {addr} id={id}");
                    send_reply(sender, &format!("REPLY:tcp-connect {id}"), inbox);
                }
                Err(e) => send_reply(sender, &format!("REPLY:tcp-connect error {e}"), inbox),
            }
        }
        "tcp-send" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (id_str, data) = rest.split_once(' ').unwrap_or((rest, ""));
            let id: u32 = id_str.parse().unwrap_or(0);
            let map = lock_or_recover(&TCP_CONNS.get().unwrap());
            if let Some(stream) = map.get(&id) {
                use std::io::Write as IoWrite;
                let payload = format!("{data}\n");
                let _ = (&*stream as &std::net::TcpStream).write_all(payload.as_bytes());
                send_reply(sender, &format!("REPLY:tcp-send {id} ok"), inbox);
            } else {
                send_reply(sender, &format!("REPLY:tcp-send error not-found"), inbox);
            }
        }
        "tcp-recv" => {
            let id: u32 = parts.get(1).unwrap_or(&"0").trim().parse().unwrap_or(0);
            let map = lock_or_recover(&TCP_CONNS.get().unwrap());
            if let Some(stream) = map.get(&id) {
                use std::io::Read as IoRead;
                let mut buf = vec![0u8; 1024];
                match (&*stream as &std::net::TcpStream).read(&mut buf) {
                    Ok(n) if n > 0 => {
                        let data: String = buf[..n].iter()
                            .filter(|&&b| b >= 32 || b == b'\n' || b == b'\r')
                            .map(|&b| b as char)
                            .collect::<String>()
                            .replace('\r', "")
                            .replace('\n', "\\n");
                        send_reply(sender, &format!("REPLY:tcp-recv {id} {data}"), inbox);
                    }
                    _ => send_reply(sender, &format!("REPLY:tcp-recv {id} "), inbox),
                }
            } else {
                send_reply(sender, &format!("REPLY:tcp-recv error not-found"), inbox);
            }
        }
        "tcp-close" => {
            let id: u32 = parts.get(1).unwrap_or(&"0").trim().parse().unwrap_or(0);
            lock_or_recover(&TCP_CONNS.get().unwrap()).remove(&id);
            send_reply(sender, &format!("REPLY:tcp-close {id} ok"), inbox);
        }
        _ => return false,
    }
    true
}
