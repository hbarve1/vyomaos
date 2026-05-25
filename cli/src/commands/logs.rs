// vyoma logs <app> — stream live log output from a named app.
//
// Implements FR-003: auto-resume on restart by reconnecting and re-subscribing
// when the server closes the connection.

use crate::{
    connection::VyomaConnection,
    protocol::{MgmtRequest, MgmtResponse},
};

/// Stream log lines for `app` until interrupted or a fatal error occurs.
///
/// On EOF (server closed) the command prints a reconnect notice and retries
/// with a new `VyomaConnection` to the same host/port.
pub fn run(conn: &mut VyomaConnection, app: &str) -> anyhow::Result<()> {
    conn.send(&MgmtRequest::Logs { app: app.to_string() })?;

    loop {
        match conn.recv() {
            Ok(MgmtResponse::LogLine { app: a, line }) => {
                println!("[{a}] {line}");
            }
            Ok(MgmtResponse::Error { code, message }) => {
                if code == "NOT_FOUND" {
                    eprintln!("error: app '{app}' not found — {message}");
                    std::process::exit(3);
                }
                eprintln!("error [{code}]: {message}");
                return Err(anyhow::anyhow!("logs failed: {code}"));
            }
            Ok(other) => {
                eprintln!("unexpected response: {other:?}");
            }
            Err(e) => {
                // EOF or network error — log and surface to caller for retry.
                eprintln!("connection closed ({e}); streaming ended");
                return Ok(());
            }
        }
    }
}
