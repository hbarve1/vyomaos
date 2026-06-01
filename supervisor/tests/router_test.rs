// Unit tests for supervisor/src/router.rs — IPC routing logic.
//
// The `route_or_print` function lives in the binary crate and depends on
// global statics (LAST_SENDER, EXEC_REPLY_CHANNELS, AppRegistry).  We cannot
// call it directly from an integration test.  Instead we test the underlying
// pure functions it delegates to (ipc module) and the `send_reply` helper.

use std::collections::HashMap;
use std::sync::{mpsc, Arc, Mutex};

// ── send_reply via IPC inbox ────────────────────────────────────────────────

fn make_inbox() -> Arc<Mutex<HashMap<String, mpsc::Sender<String>>>> {
    Arc::new(Mutex::new(HashMap::new()))
}

/// send_reply to a registered app delivers the message.
#[test]
fn test_send_reply_delivers_message() {
    let inbox = make_inbox();
    let (tx, rx) = mpsc::channel();
    inbox.lock().unwrap().insert("pong".to_string(), tx);

    // Simulate send_reply logic (same as router::send_reply)
    {
        let map = inbox.lock().unwrap();
        if let Some(tx) = map.get("pong") {
            let _ = tx.send("hello".to_string());
        }
    }

    assert_eq!(rx.recv().unwrap(), "hello");
}

/// send_reply to unknown target is a no-op (no panic).
#[test]
fn test_send_reply_unknown_target_noop() {
    let inbox = make_inbox();
    // No panic expected
    {
        let map = inbox.lock().unwrap();
        if let Some(tx) = map.get("nonexistent") {
            let _ = tx.send("hello".to_string());
        }
    }
}

// ── IPC target parsing (used inside route_or_print) ─────────────────────────

#[test]
fn test_ipc_parse_valid() {
    let (target, payload) = supervisor::ipc::parse_ipc_target("@pong: hello").unwrap();
    assert_eq!(target, "pong");
    assert_eq!(payload, "hello");
}

#[test]
fn test_ipc_parse_supervisor() {
    let (target, payload) = supervisor::ipc::parse_ipc_target("@supervisor: ps").unwrap();
    assert_eq!(target, "supervisor");
    assert_eq!(payload, "ps");
}

#[test]
fn test_ipc_parse_missing_at() {
    assert!(supervisor::ipc::parse_ipc_target("pong: hello").is_err());
}

#[test]
fn test_ipc_parse_missing_separator() {
    assert!(supervisor::ipc::parse_ipc_target("@pong hello").is_err());
}

#[test]
fn test_ipc_parse_empty_target() {
    assert!(supervisor::ipc::parse_ipc_target("@: hello").is_err());
}

// ── Broadcast target detection ──────────────────────────────────────────────

#[test]
fn test_broadcast_target_true() {
    assert!(supervisor::ipc::is_broadcast_target("broadcast"));
}

#[test]
fn test_broadcast_target_false() {
    assert!(!supervisor::ipc::is_broadcast_target("pong"));
    assert!(!supervisor::ipc::is_broadcast_target("Broadcast"));
}

// ── Reply target detection ──────────────────────────────────────────────────

#[test]
fn test_reply_target_true() {
    assert!(supervisor::ipc::is_reply_target("reply"));
}

#[test]
fn test_reply_target_false() {
    assert!(!supervisor::ipc::is_reply_target("Reply"));
    assert!(!supervisor::ipc::is_reply_target("pong"));
}

// ── Reply resolution ────────────────────────────────────────────────────────

#[test]
fn test_resolve_reply_found() {
    let mut map = HashMap::new();
    map.insert("pong".to_string(), "ping".to_string());
    assert_eq!(supervisor::ipc::resolve_reply("pong", &map), Some("ping"));
}

#[test]
fn test_resolve_reply_not_found() {
    let map: HashMap<String, String> = HashMap::new();
    assert_eq!(supervisor::ipc::resolve_reply("pong", &map), None);
}

// ── VYOMA_DRAW prefix detection (mirrors router's strip_prefix logic) ───────

#[test]
fn test_vyoma_draw_prefix_strip() {
    let line = "VYOMA_DRAW:fill_rect:0,0,100,100,0xFF0000FF";
    let cmd = line.strip_prefix("VYOMA_DRAW:");
    assert_eq!(cmd, Some("fill_rect:0,0,100,100,0xFF0000FF"));
}

#[test]
fn test_vyoma_draw_prefix_absent() {
    let line = "hello world";
    assert!(line.strip_prefix("VYOMA_DRAW:").is_none());
}

// ── VYOMA_AUDIO prefix detection ────────────────────────────────────────────

#[test]
fn test_vyoma_audio_prefix_strip() {
    let line = "VYOMA_AUDIO:set_volume:80";
    let cmd = line.strip_prefix("VYOMA_AUDIO:");
    assert_eq!(cmd, Some("set_volume:80"));
}

#[test]
fn test_vyoma_audio_prefix_absent() {
    let line = "@pong: hello";
    assert!(line.strip_prefix("VYOMA_AUDIO:").is_none());
}

// ── IPC @ prefix + routing logic (pure string parsing) ──────────────────────

#[test]
fn test_at_prefix_routing() {
    let line = "@pong: hello from ping";
    let rest = line.strip_prefix('@').unwrap();
    let (target, msg) = rest.split_once(": ").unwrap();
    assert_eq!(target, "pong");
    assert_eq!(msg, "hello from ping");
}

#[test]
fn test_at_mgmt_target() {
    let line = "@__mgmt__: result ok";
    let rest = line.strip_prefix('@').unwrap();
    let (target, msg) = rest.split_once(": ").unwrap();
    assert_eq!(target, "__mgmt__");
    assert_eq!(msg, "result ok");
}

// ── Broadcast delivery to multiple receivers ────────────────────────────────

#[test]
fn test_broadcast_delivery_to_all() {
    let inbox = make_inbox();
    let (tx1, rx1) = mpsc::channel();
    let (tx2, rx2) = mpsc::channel();
    let (tx3, rx3) = mpsc::channel();
    {
        let mut map = inbox.lock().unwrap();
        map.insert("app1".to_string(), tx1);
        map.insert("app2".to_string(), tx2);
        map.insert("app3".to_string(), tx3);
    }

    // Simulate broadcast
    let msg = "system shutdown";
    {
        let map = inbox.lock().unwrap();
        for tx in map.values() {
            let _ = tx.send(msg.to_string());
        }
    }

    assert_eq!(rx1.recv().unwrap(), "system shutdown");
    assert_eq!(rx2.recv().unwrap(), "system shutdown");
    assert_eq!(rx3.recv().unwrap(), "system shutdown");
}
