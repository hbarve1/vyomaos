// Unit tests for supervisor/src/mgmt_protocol.rs — NDJSON protocol types.
//
// mgmt_protocol is a private module in the binary crate.  However, the types
// use serde Serialize/Deserialize, and the module is mirrored in cli/src/protocol.rs.
// We replicate the types here to test serialization/deserialization roundtrips.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum AppStatus {
    Running,
    Stopped,
    Degraded,
    Unhealthy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MgmtRequest {
    Ps,
    Logs { app: String },
    PushStart { name: String, size: u64, sha256: Option<String> },
    HeartbeatStream,
    Exec { app: String, msg: String, timeout_ms: Option<u64> },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
enum MgmtResponse {
    PsRow { name: String, status: AppStatus, uptime_s: u64, restarts: u32, mem_kb: u64 },
    PsDone,
    LogLine { app: String, line: String },
    PushAck { state: String, elapsed_s: u64, health_status: Option<String> },
    PushResult { ok: bool, message: String },
    Heartbeat { module: String, uptime_s: u64, mem_kb: u64, status: String, last_error: String },
    ExecReply { reply: String },
    Error { code: String, message: String },
}

// ── MgmtRequest serialization ───────────────────────────────────────────────

#[test]
fn test_ps_request_serialize() {
    let req = MgmtRequest::Ps;
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"type\":\"ps\""));
}

#[test]
fn test_ps_request_roundtrip() {
    let req = MgmtRequest::Ps;
    let json = serde_json::to_string(&req).unwrap();
    let req2: MgmtRequest = serde_json::from_str(&json).unwrap();
    assert!(matches!(req2, MgmtRequest::Ps));
}

#[test]
fn test_logs_request_serialize() {
    let req = MgmtRequest::Logs { app: "calc".into() };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"type\":\"logs\""));
    assert!(json.contains("\"app\":\"calc\""));
}

#[test]
fn test_push_start_serialize() {
    let req = MgmtRequest::PushStart {
        name: "calc".into(),
        size: 12345,
        sha256: Some("abc123".into()),
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"type\":\"push_start\""));
    assert!(json.contains("\"size\":12345"));
    assert!(json.contains("\"sha256\":\"abc123\""));
}

#[test]
fn test_push_start_without_sha256() {
    let req = MgmtRequest::PushStart {
        name: "calc".into(),
        size: 100,
        sha256: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    let req2: MgmtRequest = serde_json::from_str(&json).unwrap();
    match req2 {
        MgmtRequest::PushStart { sha256, .. } => assert!(sha256.is_none()),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn test_exec_request_serialize() {
    let req = MgmtRequest::Exec {
        app: "shell".into(),
        msg: "echo hello".into(),
        timeout_ms: Some(5000),
    };
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"type\":\"exec\""));
    assert!(json.contains("\"timeout_ms\":5000"));
}

#[test]
fn test_exec_request_without_timeout() {
    let req = MgmtRequest::Exec {
        app: "shell".into(),
        msg: "ls".into(),
        timeout_ms: None,
    };
    let json = serde_json::to_string(&req).unwrap();
    let req2: MgmtRequest = serde_json::from_str(&json).unwrap();
    match req2 {
        MgmtRequest::Exec { timeout_ms, .. } => assert!(timeout_ms.is_none()),
        _ => panic!("wrong variant"),
    }
}

#[test]
fn test_heartbeat_stream_serialize() {
    let req = MgmtRequest::HeartbeatStream;
    let json = serde_json::to_string(&req).unwrap();
    assert!(json.contains("\"type\":\"heartbeat_stream\""));
}

// ── MgmtResponse serialization ──────────────────────────────────────────────

#[test]
fn test_ps_row_roundtrip() {
    let resp = MgmtResponse::PsRow {
        name: "calc".into(),
        status: AppStatus::Running,
        uptime_s: 3600,
        restarts: 2,
        mem_kb: 1024,
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: MgmtResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, resp2);
}

#[test]
fn test_ps_done_roundtrip() {
    let resp = MgmtResponse::PsDone;
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: MgmtResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, resp2);
}

#[test]
fn test_error_response_roundtrip() {
    let resp = MgmtResponse::Error {
        code: "NOT_FOUND".into(),
        message: "app not found".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: MgmtResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, resp2);
}

#[test]
fn test_exec_reply_roundtrip() {
    let resp = MgmtResponse::ExecReply { reply: "REPLY:ok".into() };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: MgmtResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, resp2);
}

#[test]
fn test_heartbeat_roundtrip() {
    let resp = MgmtResponse::Heartbeat {
        module: "supervisor".into(),
        uptime_s: 120,
        mem_kb: 512,
        status: "ok".into(),
        last_error: "".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: MgmtResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, resp2);
}

#[test]
fn test_push_ack_roundtrip() {
    let resp = MgmtResponse::PushAck {
        state: "receiving".into(),
        elapsed_s: 5,
        health_status: Some("healthy".into()),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: MgmtResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, resp2);
}

#[test]
fn test_push_result_roundtrip() {
    let resp = MgmtResponse::PushResult { ok: true, message: "installed".into() };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: MgmtResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, resp2);
}

#[test]
fn test_log_line_roundtrip() {
    let resp = MgmtResponse::LogLine {
        app: "calc".into(),
        line: "hello world".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    let resp2: MgmtResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(resp, resp2);
}

// ── AppStatus serialization ─────────────────────────────────────────────────

#[test]
fn test_app_status_running() {
    let s = serde_json::to_string(&AppStatus::Running).unwrap();
    assert_eq!(s, "\"running\"");
}

#[test]
fn test_app_status_stopped() {
    let s = serde_json::to_string(&AppStatus::Stopped).unwrap();
    assert_eq!(s, "\"stopped\"");
}

#[test]
fn test_app_status_degraded() {
    let s = serde_json::to_string(&AppStatus::Degraded).unwrap();
    assert_eq!(s, "\"degraded\"");
}

#[test]
fn test_app_status_unhealthy() {
    let s = serde_json::to_string(&AppStatus::Unhealthy).unwrap();
    assert_eq!(s, "\"unhealthy\"");
}

// ── NDJSON multi-line parsing ───────────────────────────────────────────────

#[test]
fn test_ndjson_multi_response_parse() {
    let lines = vec![
        serde_json::to_string(&MgmtResponse::PsRow {
            name: "calc".into(), status: AppStatus::Running,
            uptime_s: 10, restarts: 0, mem_kb: 100,
        }).unwrap(),
        serde_json::to_string(&MgmtResponse::PsRow {
            name: "shell".into(), status: AppStatus::Running,
            uptime_s: 20, restarts: 1, mem_kb: 200,
        }).unwrap(),
        serde_json::to_string(&MgmtResponse::PsDone).unwrap(),
    ];

    let ndjson = lines.join("\n");
    let parsed: Vec<MgmtResponse> = ndjson.lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();

    assert_eq!(parsed.len(), 3);
    assert!(matches!(parsed[2], MgmtResponse::PsDone));
}
