// Management server protocol types — NDJSON wire format.
//
// Both supervisor (server) and CLI (client) use identical JSON serialization.
// The CLI has a mirrored copy in cli/src/protocol.rs.
//
// All messages use a `"type"` discriminator field (serde tagged enum).

use serde::{Deserialize, Serialize};

// ── AppStatus ─────────────────────────────────────────────────────────────────

/// Lifecycle state as reported to CLI clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppStatus {
    Running,
    Stopped,
    Degraded,
    Unhealthy,
}

// ── MgmtRequest ──────────────────────────────────────────────────────────────

/// Requests sent from CLI → supervisor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MgmtRequest {
    /// List all running apps.
    Ps,

    /// Stream log lines for the named app.
    Logs { app: String },

    /// Begin OTA push; `size` bytes of raw binary follow immediately.
    PushStart {
        name: String,
        size: u64,
        sha256: Option<String>,
    },

    /// Subscribe to heartbeat events (streams until client disconnects).
    HeartbeatStream,

    /// Send an IPC message to a running app and wait for a reply.
    Exec {
        app: String,
        msg: String,
        timeout_ms: Option<u64>,
    },
}

// ── MgmtResponse ─────────────────────────────────────────────────────────────

/// Responses sent from supervisor → CLI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MgmtResponse {
    /// One row per running app (ps command).
    PsRow {
        name:     String,
        status:   AppStatus,
        uptime_s: u64,
        restarts: u32,
        mem_kb:   u64,
    },

    /// Terminal response for ps — no more rows follow.
    PsDone,

    /// One log line from a streaming logs command.
    LogLine { app: String, line: String },

    /// Progress update during an OTA push.
    PushAck {
        state:         String,
        elapsed_s:     u64,
        health_status: Option<String>,
    },

    /// Terminal response for push_start.
    PushResult { ok: bool, message: String },

    /// One heartbeat event (heartbeat_stream command).
    Heartbeat {
        module:     String,
        uptime_s:   u64,
        mem_kb:     u64,
        status:     String,
        last_error: String,
    },

    /// Terminal response for exec.
    ExecReply { reply: String },

    /// Error response for any command (also terminal).
    Error { code: String, message: String },
}
