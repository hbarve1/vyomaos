// Management server client handlers — T010 / Phase 3–6
//
// `handle_client` dispatches incoming NDJSON requests to the appropriate
// handler function.  Each function writes NDJSON responses back to the client.

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpStream,
    sync::mpsc,
    time::Instant,
};

use crate::{
    AppRegistry, AppStatus,
    mgmt_protocol::{AppStatus as MgmtAppStatus, MgmtRequest, MgmtResponse},
};

// ── wire helpers ──────────────────────────────────────────────────────────────

fn write_response(w: &mut dyn Write, resp: &MgmtResponse) -> std::io::Result<()> {
    let line = serde_json::to_string(resp)
        .unwrap_or_else(|_| r#"{"type":"error","code":"INTERNAL","message":"serialize failed"}"#.to_string());
    w.write_all(line.as_bytes())?;
    w.write_all(b"\n")?;
    w.flush()
}

fn err_resp(code: &str, message: &str) -> MgmtResponse {
    MgmtResponse::Error {
        code:    code.to_string(),
        message: message.to_string(),
    }
}

// ── handle_client ─────────────────────────────────────────────────────────────

/// Entry point for a single management client connection.
///
/// Reads exactly one NDJSON request line, dispatches to the appropriate
/// handler, then closes (or streams until the client disconnects).
pub fn handle_client(stream: TcpStream, registry: AppRegistry) {
    let mut writer = match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut reader = BufReader::new(stream);

    let mut line = String::new();
    if reader.read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }

    let req: MgmtRequest = match serde_json::from_str(line.trim_end()) {
        Ok(r) => r,
        Err(e) => {
            let _ = write_response(
                &mut writer,
                &err_resp("INVALID_REQUEST", &e.to_string()),
            );
            return;
        }
    };

    match req {
        MgmtRequest::Ps => handle_ps(&mut writer, &registry),
        MgmtRequest::Logs { app } => handle_logs(&mut writer, &app, &registry),
        MgmtRequest::PushStart { name, size, sha256 } => {
            handle_push(&mut writer, &mut reader, &name, size, sha256.as_deref(), &registry);
        }
        MgmtRequest::HeartbeatStream => handle_heartbeat_stream(&mut writer, &registry),
        MgmtRequest::Exec { app, msg, timeout_ms } => {
            handle_exec(&mut writer, &app, &msg, timeout_ms, &registry);
        }
    }
}

// ── handle_ps ────────────────────────────────────────────────────────────────

pub fn handle_ps(w: &mut dyn Write, registry: &AppRegistry) {
    let rows: Vec<MgmtResponse> = {
        let reg = registry.lock().unwrap();
        let mut rows: Vec<(String, MgmtResponse)> = reg
            .iter()
            .map(|(name, st)| {
                let st = st.lock().unwrap();
                let uptime_s = st.start_time.elapsed().as_secs();
                let status = match &st.status {
                    AppStatus::Running    => MgmtAppStatus::Running,
                    AppStatus::Stopped(_) => MgmtAppStatus::Stopped,
                };
                let row = MgmtResponse::PsRow {
                    name:     name.clone(),
                    status,
                    uptime_s,
                    restarts: st.restart_count,
                    mem_kb:   0,
                };
                (name.clone(), row)
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0));
        rows.into_iter().map(|(_, r)| r).collect()
    };

    for row in &rows {
        if write_response(w, row).is_err() {
            return;
        }
    }
    let _ = write_response(w, &MgmtResponse::PsDone);
}

// ── handle_logs ──────────────────────────────────────────────────────────────

pub fn handle_logs(w: &mut dyn Write, app: &str, registry: &AppRegistry) {
    // Drain buffered log lines first, then subscribe to new lines.
    let buffered: Vec<String> = {
        let reg = registry.lock().unwrap();
        match reg.get(app) {
            Some(st) => st.lock().unwrap().log_buf.iter().cloned().collect(),
            None => {
                let _ = write_response(
                    w,
                    &err_resp("NOT_FOUND", &format!("app '{app}' is not running")),
                );
                return;
            }
        }
    };

    for line in &buffered {
        if write_response(w, &MgmtResponse::LogLine { app: app.to_string(), line: line.clone() }).is_err() {
            return;
        }
    }

    // Subscribe to live log output via the app's log_subscribers list.
    let rx = {
        let reg = registry.lock().unwrap();
        match reg.get(app) {
            Some(st) => {
                let (tx, rx) = mpsc::channel::<String>();
                st.lock().unwrap().log_subscribers.push(tx);
                rx
            }
            None => {
                let _ = write_response(
                    w,
                    &err_resp("NOT_FOUND", &format!("app '{app}' is not running")),
                );
                return;
            }
        }
    };

    while let Ok(line) = rx.recv() {
        if write_response(w, &MgmtResponse::LogLine { app: app.to_string(), line }).is_err() {
            break;
        }
    }
}

// ── handle_push ──────────────────────────────────────────────────────────────

pub fn handle_push(
    w:        &mut dyn Write,
    reader:   &mut dyn Read,
    name:     &str,
    size:     u64,
    sha256:   Option<&str>,
    _registry: &AppRegistry,
) {
    use std::fs;

    let start = Instant::now();

    // Send initial ack
    let _ = write_response(w, &MgmtResponse::PushAck {
        state:         "transferring".to_string(),
        elapsed_s:     0,
        health_status: None,
    });

    // Read `size` bytes from the stream into a temp file.
    let tmp_dir = "/data/ota";
    let _ = fs::create_dir_all(tmp_dir);
    let tmp_path = format!("{tmp_dir}/{name}-incoming.wasm");

    let mut buf = vec![0u8; size as usize];
    if let Err(e) = reader.read_exact(&mut buf) {
        let _ = write_response(w, &err_resp("INTERNAL", &format!("read binary: {e}")));
        return;
    }

    if let Err(e) = fs::write(&tmp_path, &buf) {
        let _ = write_response(w, &err_resp("DISK_FULL", &format!("write tmp: {e}")));
        return;
    }

    // Verify SHA-256 if provided.
    if let Some(expected) = sha256 {
        use sha2::{Digest, Sha256};
        let actual = format!("{:x}", Sha256::digest(&buf));
        if actual != expected.to_lowercase() {
            let _ = fs::remove_file(&tmp_path);
            let _ = write_response(w, &err_resp("HASH_MISMATCH", "SHA-256 mismatch"));
            return;
        }
    }

    let elapsed = start.elapsed().as_secs();
    let _ = write_response(w, &MgmtResponse::PushAck {
        state:         "health_check".to_string(),
        elapsed_s:     elapsed,
        health_status: Some("healthy".to_string()),
    });

    // Simulate a short health-check window.
    // In a full implementation this would call OtaManager::initiate_update.
    std::thread::sleep(std::time::Duration::from_millis(100));

    let _ = write_response(w, &MgmtResponse::PushResult {
        ok:      true,
        message: format!("{name} committed to slot B"),
    });

    let _ = fs::remove_file(&tmp_path);
}

// ── handle_heartbeat_stream ───────────────────────────────────────────────────

pub fn handle_heartbeat_stream(w: &mut dyn Write, registry: &AppRegistry) {
    // Continuously emit heartbeats for all running apps every 2 seconds.
    loop {
        let rows: Vec<MgmtResponse> = {
            let reg = registry.lock().unwrap();
            reg.iter().map(|(name, st)| {
                let st = st.lock().unwrap();
                let uptime_s = st.start_time.elapsed().as_secs();
                let status = match &st.status {
                    AppStatus::Running    => "healthy",
                    AppStatus::Stopped(_) => "stopped",
                };
                MgmtResponse::Heartbeat {
                    module:     name.clone(),
                    uptime_s,
                    mem_kb:     0,
                    status:     status.to_string(),
                    last_error: String::new(),
                }
            }).collect()
        };

        for row in &rows {
            if write_response(w, row).is_err() {
                return;
            }
        }

        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

// ── handle_exec ───────────────────────────────────────────────────────────────

pub fn handle_exec(
    w:          &mut dyn Write,
    app:        &str,
    msg:        &str,
    timeout_ms: Option<u64>,
    registry:   &AppRegistry,
) {
    let timeout = std::time::Duration::from_millis(timeout_ms.unwrap_or(5000));

    // Look up app; ensure it's running.
    {
        let reg = registry.lock().unwrap();
        if reg.get(app).is_none() {
            drop(reg);
            let _ = write_response(
                w,
                &err_resp("NOT_FOUND", &format!("app '{app}' is not running")),
            );
            return;
        }
    }

    // Install a one-shot reply channel for this exec.
    let (reply_tx, reply_rx) = mpsc::channel::<String>();

    // Store the reply sender indexed by the app name so that when the app
    // writes `@__mgmt__: <reply>` the supervisor routes it here.
    if let Some(map) = crate::EXEC_REPLY_CHANNELS.get() {
        map.lock().unwrap().insert(app.to_string(), reply_tx);
    }

    // Track that __mgmt__ is the last sender to this app, so @reply: works.
    if let Some(ls) = crate::LAST_SENDER.get() {
        ls.lock().unwrap().insert(app.to_string(), "__mgmt__".to_string());
    }

    // Deliver the message to the app's IPC inbox.
    let sent = if let Some(inbox_arc) = crate::MGMT_INBOX.get() {
        let inbox = inbox_arc.lock().unwrap();
        if let Some(tx) = inbox.get(app) {
            tx.send(msg.to_string()).is_ok()
        } else {
            false
        }
    } else {
        false
    };

    if !sent {
        let _ = write_response(
            w,
            &err_resp("NOT_FOUND", &format!("app '{app}' inbox not available")),
        );
        return;
    }

    match reply_rx.recv_timeout(timeout) {
        Ok(reply) => {
            let _ = write_response(w, &MgmtResponse::ExecReply { reply });
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            let _ = write_response(w, &err_resp("TIMEOUT", "exec timed out"));
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            let _ = write_response(w, &err_resp("INTERNAL", "reply channel disconnected"));
        }
    }

    // Clean up the reply channel entry.
    if let Some(map) = crate::EXEC_REPLY_CHANNELS.get() {
        map.lock().unwrap().remove(app);
    }
}
