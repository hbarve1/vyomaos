// T015: Unit tests for heartbeat emission.
//
// Tests verify:
// - JSON-line format matches data-model.md §6
// - Interval enforcement
// - Module metadata fields

use supervisor::observability::{Heartbeat, ModuleStatus};
use supervisor::observability::heartbeat::HeartbeatEmitter;

// ── T015-1: Heartbeat JSON contains required fields ───────────────────────────

#[test]
fn test_heartbeat_json_contains_type_field() {
    let hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Healthy);
    let json = hb.to_json_line();
    assert!(json.contains(r#""type":"heartbeat""#), "JSON should contain type field: {json}");
}

#[test]
fn test_heartbeat_json_contains_module_field() {
    let hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Healthy);
    let json = hb.to_json_line();
    assert!(json.contains(r#""module":"sensor""#), "JSON should contain module field: {json}");
}

#[test]
fn test_heartbeat_json_contains_uptime_field() {
    let hb = Heartbeat::new("sensor", 42, 64, ModuleStatus::Healthy);
    let json = hb.to_json_line();
    assert!(json.contains(r#""uptime_s":42"#), "JSON should contain uptime_s field: {json}");
}

#[test]
fn test_heartbeat_json_contains_mem_kb_field() {
    let hb = Heartbeat::new("sensor", 10, 256, ModuleStatus::Healthy);
    let json = hb.to_json_line();
    assert!(json.contains(r#""mem_kb":256"#), "JSON should contain mem_kb field: {json}");
}

#[test]
fn test_heartbeat_json_contains_status_healthy() {
    let hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Healthy);
    let json = hb.to_json_line();
    assert!(json.contains(r#""status":"healthy""#), "JSON should contain healthy status: {json}");
}

#[test]
fn test_heartbeat_json_contains_status_degraded() {
    let hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Degraded);
    let json = hb.to_json_line();
    assert!(json.contains(r#""status":"degraded""#), "JSON should contain degraded status: {json}");
}

#[test]
fn test_heartbeat_json_contains_status_unhealthy() {
    let hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Unhealthy);
    let json = hb.to_json_line();
    assert!(json.contains(r#""status":"unhealthy""#), "JSON should contain unhealthy status: {json}");
}

// ── T015-2: Heartbeat JSON has last_error null when no error ─────────────────

#[test]
fn test_heartbeat_json_last_error_null_when_none() {
    let hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Healthy);
    let json = hb.to_json_line();
    assert!(json.contains(r#""last_error":null"#), "last_error should be null when None: {json}");
}

// ── T015-3: Heartbeat JSON has last_error string when set ────────────────────

#[test]
fn test_heartbeat_json_last_error_string_when_set() {
    let mut hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Degraded);
    hb.last_error = Some("connection refused".to_string());
    let json = hb.to_json_line();
    assert!(
        json.contains(r#""last_error":"connection refused""#),
        "last_error should be a string when set: {json}"
    );
}

// ── T015-4: Heartbeat JSON last_error escapes special characters ─────────────

#[test]
fn test_heartbeat_json_last_error_escapes_quotes() {
    let mut hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Degraded);
    hb.last_error = Some(r#"error: "timeout""#.to_string());
    let json = hb.to_json_line();
    // The quote inside the error string should be escaped.
    assert!(
        !json.contains(r#"error: "timeout""#),
        "unescaped quote inside JSON string should not appear: {json}"
    );
    assert!(json.contains("error:"), "error text should still appear in JSON: {json}");
}

// ── T015-5: HeartbeatEmitter is_due() returns true initially ─────────────────

#[test]
fn test_emitter_is_due_initially() {
    let emitter = HeartbeatEmitter::new("motor", 30);
    assert!(emitter.is_due(), "emitter should be due initially");
}

// ── T015-6: HeartbeatEmitter is_due() returns false immediately after emit ───

#[test]
fn test_emitter_not_due_immediately_after_emit() {
    let mut emitter = HeartbeatEmitter::new("motor", 30);
    let _ = emitter.emit(128, ModuleStatus::Healthy);
    assert!(!emitter.is_due(), "emitter should not be due immediately after emit");
}

// ── T015-7: emit() returns Heartbeat with correct module name ────────────────

#[test]
fn test_emitter_emit_returns_correct_module_name() {
    let mut emitter = HeartbeatEmitter::new("camera", 30);
    let hb = emitter.emit(64, ModuleStatus::Healthy);
    assert_eq!(hb.module, "camera");
}

// ── T015-8: emit() returns Heartbeat with correct mem_kb ─────────────────────

#[test]
fn test_emitter_emit_returns_correct_mem_kb() {
    let mut emitter = HeartbeatEmitter::new("lidar", 30);
    let hb = emitter.emit(512, ModuleStatus::Healthy);
    assert_eq!(hb.mem_kb, 512);
}

// ── T015-9: Heartbeat JSON is a single line (no newlines) ────────────────────

#[test]
fn test_heartbeat_json_is_single_line() {
    let hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Healthy);
    let json = hb.to_json_line();
    assert!(!json.contains('\n'), "JSON should be a single line: {json}");
}

// ── T015-10: Heartbeat JSON is valid JSON structure (braces) ─────────────────

#[test]
fn test_heartbeat_json_has_outer_braces() {
    let hb = Heartbeat::new("sensor", 10, 128, ModuleStatus::Healthy);
    let json = hb.to_json_line();
    assert!(json.starts_with('{'), "JSON should start with {{: {json}");
    assert!(json.ends_with('}'), "JSON should end with }}: {json}");
}
