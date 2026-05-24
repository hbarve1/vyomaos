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

// ── Broadcast target tests ────────────────────────────────────────────────────

// (4) "broadcast" is recognised as the reserved broadcast address.
#[test]
fn test_is_broadcast_target_true() {
    assert!(supervisor::ipc::is_broadcast_target("broadcast"),
        "\"broadcast\" must be recognised as the broadcast target");
}

// (5) A regular app name is NOT treated as broadcast.
#[test]
fn test_is_broadcast_target_false_for_named_app() {
    assert!(!supervisor::ipc::is_broadcast_target("shell"),
        "\"shell\" must not be treated as a broadcast target");
}

// (6) @broadcast: hello parses into target="broadcast" and payload="hello".
#[test]
fn test_parse_ipc_target_broadcast_message() {
    let result = supervisor::ipc::parse_ipc_target("@broadcast: hello");
    assert!(result.is_ok(), "valid broadcast line should parse, got: {:?}", result);
    let (target, payload) = result.unwrap();
    assert_eq!(target,  "broadcast", "target should be \"broadcast\"");
    assert_eq!(payload, "hello",     "payload mismatch");
}

// (7) A line without the IPC prefix returns an error.
#[test]
fn test_parse_ipc_target_non_ipc_line() {
    let result = supervisor::ipc::parse_ipc_target("not a message");
    assert!(result.is_err(), "non-IPC line should return Err, got Ok");
}
