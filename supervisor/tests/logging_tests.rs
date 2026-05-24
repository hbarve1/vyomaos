// Logging unit tests — TDD RED until logging module is implemented.

use supervisor::logging::{format_log, Level, Subsystem};

// (1) format_log(Info, Manifest, None, "msg") produces correct prefix format
#[test]
fn test_format_log_info_manifest_no_app() {
    let line = format_log(Level::Info, Subsystem::Manifest, None, "parsed OK");
    // Expected: "[YYYY-MM-DDTHH:MM:SS.mmmZ] [INFO ] [manifest] parsed OK"
    assert!(line.contains("[INFO ]"),     "missing level: {line}");
    assert!(line.contains("[manifest]"),  "missing subsystem: {line}");
    assert!(line.contains("parsed OK"),   "missing message: {line}");
    assert!(!line.contains("app="),       "should have no app= field when None: {line}");
}

// (2) Timestamp field is non-empty and contains 'T' (ISO 8601 format)
#[test]
fn test_timestamp_is_iso8601() {
    let line = format_log(Level::Info, Subsystem::Lifecycle, None, "boot");
    assert!(line.contains('T'), "timestamp missing 'T' separator: {line}");
    // Should start with '[' (opening bracket of timestamp)
    assert!(line.starts_with('['), "log line should start with '[': {line}");
}

// (3) Level::Warn produces "[WARN ]" (5 chars with trailing space)
#[test]
fn test_warn_level_has_trailing_space() {
    let line = format_log(Level::Warn, Subsystem::Ipc, None, "retrying");
    assert!(line.contains("[WARN ]"), "WARN level should be 5 chars '[WARN ]': {line}");
}

// (4) format_log with app = Some("hello-world") includes app=hello-world in output
#[test]
fn test_format_log_with_app_includes_app_field() {
    let line = format_log(Level::Info, Subsystem::Lifecycle, Some("hello-world"), "spawned");
    assert!(line.contains("app=hello-world"), "missing app= field: {line}");
    assert!(line.contains("spawned"),         "missing message: {line}");
}
