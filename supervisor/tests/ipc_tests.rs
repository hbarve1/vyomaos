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

// ── @reply: routing helpers ───────────────────────────────────────────────────

// is_reply_target("reply") returns true
#[test]
fn test_is_reply_target_reply() {
    assert!(supervisor::ipc::is_reply_target("reply"),
        "\"reply\" should be recognised as the reply pseudo-target");
}

// is_reply_target("shell") returns false
#[test]
fn test_is_reply_target_other_app() {
    assert!(!supervisor::ipc::is_reply_target("shell"),
        "\"shell\" should not be recognised as the reply pseudo-target");
}

// resolve_reply with a known sender returns the correct original sender
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

// resolve_reply with an unknown sender returns None
#[test]
fn test_resolve_reply_unknown_sender() {
    use std::collections::HashMap;
    let last_senders: HashMap<String, String> = HashMap::new();
    let result = supervisor::ipc::resolve_reply("unknown_app", &last_senders);
    assert_eq!(result, None,
        "resolve_reply should return None when no prior IPC message exists");
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

// ── format_app_list tests ─────────────────────────────────────────────────────

#[test]
fn format_app_list_empty() {
    assert_eq!(supervisor::ipc::format_app_list(&[]), "");
}

#[test]
fn format_app_list_one() {
    assert_eq!(supervisor::ipc::format_app_list(&["shell"]), "shell");
}

#[test]
fn format_app_list_multiple() {
    let r = supervisor::ipc::format_app_list(&["shell", "ticker", "gui-demo"]);
    assert_eq!(r, "shell\nticker\ngui-demo");
}

#[test]
fn format_app_list_no_trailing_newline() {
    let r = supervisor::ipc::format_app_list(&["a", "b"]);
    assert!(!r.ends_with('\n'), "should not end with newline");
}

// validate_kill_target tests

#[test]
fn validate_kill_target_found() {
    assert!(supervisor::ipc::validate_kill_target("shell", &["shell", "ticker"]));
}

#[test]
fn validate_kill_target_not_found() {
    assert!(!supervisor::ipc::validate_kill_target("missing", &["shell", "ticker"]));
}

#[test]
fn validate_kill_target_empty_list() {
    assert!(!supervisor::ipc::validate_kill_target("shell", &[]));
}

#[test]
fn validate_kill_target_exact_match() {
    // should not match substrings
    assert!(!supervisor::ipc::validate_kill_target("she", &["shell"]));
}
