// vyoma exec <app> "<message>" — send IPC message and print reply.
//
// Exit codes:
//   0 — reply received
//   3 — app not found
//   4 — timeout

use crate::{
    connection::VyomaConnection,
    protocol::{MgmtRequest, MgmtResponse},
};

/// Send an IPC message to `app` and print the reply.
pub fn run(
    conn:    &mut VyomaConnection,
    app:     &str,
    msg:     &str,
    timeout_secs: u64,
) -> anyhow::Result<()> {
    conn.send(&MgmtRequest::Exec {
        app:        app.to_string(),
        msg:        msg.to_string(),
        timeout_ms: Some(timeout_secs * 1000),
    })?;

    match conn.recv()? {
        MgmtResponse::ExecReply { reply } => {
            println!("{reply}");
            Ok(())
        }
        MgmtResponse::Error { code, message } => {
            eprintln!("error [{code}]: {message}");
            let exit_code = match code.as_str() {
                "NOT_FOUND" => 3,
                "TIMEOUT"   => 4,
                _           => 1,
            };
            std::process::exit(exit_code);
        }
        other => {
            eprintln!("unexpected response: {other:?}");
            Err(anyhow::anyhow!("unexpected exec response"))
        }
    }
}
