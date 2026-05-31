// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! IPC commands for WebSocket connections:
//!   @supervisor: ws-connect <url>
//!   @supervisor: ws-send <conn_id> <message>
//!   @supervisor: ws-close <conn_id>
//!
//! Incoming messages are delivered as `VYOMA_SYSTEM:ws:<conn_id>:<message>` to
//! the originating app via a background reader thread.

use std::sync::Arc;

use crate::{log_info, log_warn, Inbox, WS_CONNS, WS_NEXT_ID};
use crate::send_reply;
use crate::websocket;
use supervisor::logging::Subsystem;

/// Handle WebSocket IPC commands. Returns `true` if handled.
pub fn handle_ws(verb: &str, parts: &[&str], sender: &str, inbox: &Inbox) -> bool {
    match verb {
        "ws-connect" => {
            let url = parts.get(1).unwrap_or(&"").trim().to_string();
            if url.is_empty() {
                send_reply(sender, "REPLY:ws-connect error no-url", inbox);
                return true;
            }
            match websocket::ws_connect(&url) {
                Ok(conn) => {
                    let id = WS_NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    log_info!(Subsystem::Ipc, None, "ws-connect {url} id={id}");

                    // Clone the TcpStream for the background reader thread.
                    let reader_stream = match conn.stream.try_clone() {
                        Ok(s) => s,
                        Err(e) => {
                            send_reply(
                                sender,
                                &format!("REPLY:ws-connect error clone: {e}"),
                                inbox,
                            );
                            return true;
                        }
                    };

                    WS_CONNS.get().unwrap().lock().unwrap().insert(id, conn);
                    send_reply(sender, &format!("REPLY:ws-connect {id}"), inbox);

                    // Spawn a background thread to read incoming frames and
                    // deliver them to the sender app.
                    let sender_name = sender.to_string();
                    let inbox_clone = Arc::clone(inbox);
                    std::thread::spawn(move || {
                        ws_reader_loop(id, reader_stream, &sender_name, &inbox_clone);
                    });
                }
                Err(e) => {
                    send_reply(sender, &format!("REPLY:ws-connect error {e}"), inbox);
                }
            }
        }

        "ws-send" => {
            let rest = parts.get(1).unwrap_or(&"").trim();
            let (id_str, message) = rest.split_once(' ').unwrap_or((rest, ""));
            let id: u32 = match id_str.parse() {
                Ok(v) => v,
                Err(_) => {
                    send_reply(sender, "REPLY:ws-send error invalid-id", inbox);
                    return true;
                }
            };
            let mut map = WS_CONNS.get().unwrap().lock().unwrap();
            if let Some(conn) = map.get_mut(&id) {
                match websocket::ws_send(conn, message) {
                    Ok(()) => {
                        send_reply(sender, &format!("REPLY:ws-send {id} ok"), inbox);
                    }
                    Err(e) => {
                        send_reply(sender, &format!("REPLY:ws-send error {e}"), inbox);
                    }
                }
            } else {
                send_reply(sender, "REPLY:ws-send error not-found", inbox);
            }
        }

        "ws-close" => {
            let id: u32 = parts.get(1).unwrap_or(&"0").trim().parse().unwrap_or(0);
            let mut map = WS_CONNS.get().unwrap().lock().unwrap();
            if let Some(mut conn) = map.remove(&id) {
                let _ = websocket::ws_close(&mut conn);
                log_info!(Subsystem::Ipc, None, "ws-close id={id}");
                send_reply(sender, &format!("REPLY:ws-close {id} ok"), inbox);
            } else {
                send_reply(sender, "REPLY:ws-close error not-found", inbox);
            }
        }

        _ => return false,
    }
    true
}

/// Background reader loop: reads incoming WebSocket frames on a cloned TCP
/// stream and delivers text messages to the app via IPC.
fn ws_reader_loop(
    conn_id: u32,
    stream: std::net::TcpStream,
    sender: &str,
    inbox: &Inbox,
) {
    // Create a minimal WsConnection wrapper around the cloned stream for
    // the recv function.  This is read-only; we never send on this copy.
    let mut reader_conn = websocket::WsConnection {
        stream,
        host: String::new(),
        path: String::new(),
        closed: false,
    };

    loop {
        match websocket::ws_recv(&mut reader_conn) {
            Ok(Some(text)) => {
                // Deliver as VYOMA_SYSTEM event; escape newlines for IPC.
                let escaped = text.replace('\n', "\\n").replace('\r', "");
                let msg = format!("VYOMA_SYSTEM:ws:{conn_id}:{escaped}");
                send_reply(sender, &msg, inbox);
            }
            Ok(None) => {
                // No data within timeout, continue polling.
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                // Connection closed or error — notify app and exit.
                log_warn!(Subsystem::Ipc, None, "ws reader {conn_id}: {e}");
                let msg = format!("VYOMA_SYSTEM:ws:{conn_id}:closed");
                send_reply(sender, &msg, inbox);

                // Clean up the connection from the map if still present.
                if let Some(map) = WS_CONNS.get() {
                    map.lock().unwrap().remove(&conn_id);
                }
                break;
            }
        }

        // Check if the connection was removed (e.g. via ws-close command).
        let still_open = WS_CONNS.get()
            .map(|m| m.lock().unwrap().contains_key(&conn_id))
            .unwrap_or(false);
        if !still_open {
            break;
        }
    }
}
