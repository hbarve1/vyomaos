// IPC routing unit tests — TDD RED until parse_ipc_target is implemented.

// (1) Message to unknown app: returns descriptive error string (not panic)
#[test]
fn test_unknown_app_returns_err() {
    let result = supervisor::ipc::parse_ipc_target("@nonexistent: hello");
    assert!(result.is_ok(), "valid @target: msg should parse, got: {:?}", result);
    let (target, _msg) = result.unwrap();
    assert_eq!(target, "nonexistent");
}

// (2) Empty target name returns error
#[test]
fn test_empty_target_returns_err() {
    let result = supervisor::ipc::parse_ipc_target("@: hello");
    assert!(result.is_err(), "empty target should be Err, got Ok");
}

// (3) @app: msg is parsed correctly into (target, payload)
#[test]
fn test_valid_ipc_format_parses_correctly() {
    let result = supervisor::ipc::parse_ipc_target("@ping: hello from pong");
    assert!(result.is_ok(), "valid IPC format should parse, got: {:?}", result);
    let (target, payload) = result.unwrap();
    assert_eq!(target,  "ping",            "target mismatch");
    assert_eq!(payload, "hello from pong", "payload mismatch");
}

// format_pong_reply tests
#[test]
fn format_pong_reply_zero() {
    assert_eq!(supervisor::ipc::format_pong_reply(0), "pong 0");
}

#[test]
fn format_pong_reply_nonzero() {
    assert_eq!(supervisor::ipc::format_pong_reply(1234567890), "pong 1234567890");
}

#[test]
fn format_pong_reply_large() {
    assert_eq!(supervisor::ipc::format_pong_reply(u64::MAX), format!("pong {}", u64::MAX));
}

#[test]
fn format_pong_reply_starts_with_pong() {
    let r = supervisor::ipc::format_pong_reply(42);
    assert!(r.starts_with("pong "), "expected 'pong ...' got '{r}'");
}
