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

// ── @reply: routing helpers ───────────────────────────────────────────────────

// (4) is_reply_target("reply") returns true
#[test]
fn test_is_reply_target_reply() {
    assert!(supervisor::ipc::is_reply_target("reply"),
        "\"reply\" should be recognised as the reply pseudo-target");
}

// (5) is_reply_target("shell") returns false
#[test]
fn test_is_reply_target_other_app() {
    assert!(!supervisor::ipc::is_reply_target("shell"),
        "\"shell\" should not be recognised as the reply pseudo-target");
}

// (6) resolve_reply with a known sender returns the correct original sender
#[test]
fn test_resolve_reply_known_sender() {
    use std::collections::HashMap;
    let mut last_senders: HashMap<String, String> = HashMap::new();
    // app_b received a message from app_a, so LAST_SENDER["app_b"] = "app_a"
    last_senders.insert("app_b".to_string(), "app_a".to_string());
    let result = supervisor::ipc::resolve_reply("app_b", &last_senders);
    assert_eq!(result, Some("app_a"),
        "resolve_reply should return \"app_a\" as the last sender to app_b");
}

// (7) resolve_reply with an unknown sender returns None
#[test]
fn test_resolve_reply_unknown_sender() {
    use std::collections::HashMap;
    let last_senders: HashMap<String, String> = HashMap::new();
    let result = supervisor::ipc::resolve_reply("unknown_app", &last_senders);
    assert_eq!(result, None,
        "resolve_reply should return None when no prior IPC message exists");
}
