// CLI-side NDJSON protocol types — mirrors supervisor/src/mgmt_protocol.rs.
//
// These must produce byte-identical JSON to the supervisor types so that
// round-trips work correctly over the TCP/socket transport.

use serde::{Deserialize, Serialize};

// ── AppStatus ─────────────────────────────────────────────────────────────────

/// Lifecycle state as received from the supervisor.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    Running,
    Stopped,
    Degraded,
    Unhealthy,
}

impl AppStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AppStatus::Running   => "running",
            AppStatus::Stopped   => "stopped",
            AppStatus::Degraded  => "degraded",
            AppStatus::Unhealthy => "unhealthy",
        }
    }
}

// ── MgmtRequest ──────────────────────────────────────────────────────────────

/// Requests sent from CLI → supervisor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MgmtRequest {
    Ps,
    Logs { app: String },
    PushStart {
        name:   String,
        size:   u64,
        sha256: Option<String>,
    },
    HeartbeatStream,
    Exec {
        app:        String,
        msg:        String,
        timeout_ms: Option<u64>,
    },
}

// ── MgmtResponse ─────────────────────────────────────────────────────────────

/// Responses received from supervisor → CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MgmtResponse {
    PsRow {
        name:     String,
        status:   AppStatus,
        uptime_s: u64,
        restarts: u32,
        mem_kb:   u64,
    },
    PsDone,
    LogLine { app: String, line: String },
    PushAck {
        state:         String,
        elapsed_s:     u64,
        health_status: Option<String>,
    },
    PushResult { ok: bool, message: String },
    Heartbeat {
        module:     String,
        uptime_s:   u64,
        mem_kb:     u64,
        status:     String,
        last_error: String,
    },
    ExecReply { reply: String },
    Error { code: String, message: String },
}
