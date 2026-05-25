// vyoma ps — list all running apps with status/uptime/restarts.

use crate::{
    connection::VyomaConnection,
    protocol::{MgmtRequest, MgmtResponse},
};

// ── format helpers ─────────────────────────────────────────────────────────────

/// Format uptime seconds into a human-readable string (e.g. "2h 14m 32s").
pub fn format_uptime(s: u64) -> String {
    if s < 60 {
        format!("{s}s")
    } else if s < 3600 {
        format!("{}m {}s", s / 60, s % 60)
    } else {
        format!("{}h {}m {}s", s / 3600, (s % 3600) / 60, s % 60)
    }
}

// ── run ────────────────────────────────────────────────────────────────────────

/// Send a `ps` request and print a formatted table.
pub fn run(conn: &mut VyomaConnection) -> anyhow::Result<()> {
    conn.send(&MgmtRequest::Ps)?;

    let header = format!(
        "{:<20} {:<12} {:<15} {}",
        "NAME", "STATUS", "UPTIME", "RESTARTS"
    );
    println!("{header}");
    println!("{}", "-".repeat(60));

    loop {
        match conn.recv()? {
            MgmtResponse::PsRow {
                name,
                status,
                uptime_s,
                restarts,
                ..
            } => {
                println!(
                    "{:<20} {:<12} {:<15} {}",
                    name,
                    status.as_str(),
                    format_uptime(uptime_s),
                    restarts,
                );
            }
            MgmtResponse::PsDone => break,
            MgmtResponse::Error { code, message } => {
                eprintln!("error [{code}]: {message}");
                return Err(anyhow::anyhow!("ps failed: {code}"));
            }
            other => {
                eprintln!("unexpected response: {other:?}");
            }
        }
    }

    Ok(())
}
