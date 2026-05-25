// vyoma push <path.wasm> — transfer a WASM binary to the running VM via OTA.
//
// Protocol:
//   1. Send MgmtRequest::PushStart with name, size, sha256.
//   2. Stream `size` bytes of raw binary data.
//   3. Loop reading MgmtResponse until PushResult or Error.

use std::{fs, path::Path};

use crate::{
    connection::VyomaConnection,
    protocol::{MgmtRequest, MgmtResponse},
};

/// Push a WASM binary to the supervisor and stream progress.
///
/// Returns `Ok(())` on commit (exit 0) or `Err(...)` on rollback/error.
pub fn run(
    conn:   &mut VyomaConnection,
    path:   &Path,
    name:   &str,
    sha256: Option<&str>,
) -> anyhow::Result<()> {
    let bytes = fs::read(path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let size = bytes.len() as u64;

    let display_size = if size < 1024 {
        format!("{size} B")
    } else {
        format!("{:.1} KB", size as f64 / 1024.0)
    };

    println!("[0s] Transferring {name}.wasm ({display_size})...");

    // Send push_start request.
    conn.send(&MgmtRequest::PushStart {
        name:   name.to_string(),
        size,
        sha256: sha256.map(|s| s.to_string()),
    })?;

    // Stream binary data immediately after the request line.
    conn.write_raw(&bytes)?;

    // Read progress responses until PushResult or Error.
    loop {
        match conn.recv()? {
            MgmtResponse::PushAck { state, elapsed_s, health_status } => {
                let status_str = health_status
                    .map(|s| format!(" status: {s}"))
                    .unwrap_or_default();
                println!("[{elapsed_s}s] {state}{status_str}...");
            }
            MgmtResponse::PushResult { ok, message } => {
                if ok {
                    println!("SUCCESS: {message}");
                    return Ok(());
                } else {
                    eprintln!("ROLLBACK: {message}");
                    return Err(anyhow::anyhow!("push rolled back: {message}"));
                }
            }
            MgmtResponse::Error { code, message } => {
                eprintln!("error [{code}]: {message}");
                return Err(anyhow::anyhow!("push failed: {code}: {message}"));
            }
            other => {
                eprintln!("unexpected response: {other:?}");
            }
        }
    }
}
