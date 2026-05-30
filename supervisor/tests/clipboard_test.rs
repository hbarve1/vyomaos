// Unit tests for clipboard IPC helpers and protocol formatting.

#[test]
fn format_paste_message_basic() {
    let msg = supervisor::ipc::format_paste_message("hello world");
    assert_eq!(msg, "VYOMA_SYSTEM:paste:hello world");
}

#[test]
fn format_paste_message_empty() {
    let msg = supervisor::ipc::format_paste_message("");
    assert_eq!(msg, "VYOMA_SYSTEM:paste:");
}

#[test]
fn format_copy_signal() {
    let msg = supervisor::ipc::format_copy_signal();
    assert_eq!(msg, "VYOMA_SYSTEM:copy");
}

#[test]
fn format_clipboard_set_reply() {
    let reply = supervisor::ipc::format_clipboard_set_reply();
    assert_eq!(reply, "REPLY:clipboard-set ok");
}

#[test]
fn format_clipboard_get_reply_with_text() {
    let reply = supervisor::ipc::format_clipboard_get_reply("some text");
    assert_eq!(reply, "REPLY:clipboard:some text");
}

#[test]
fn format_clipboard_get_reply_empty() {
    let reply = supervisor::ipc::format_clipboard_get_reply("");
    assert_eq!(reply, "REPLY:clipboard:");
}

#[test]
fn paste_message_preserves_special_chars() {
    let msg = supervisor::ipc::format_paste_message("line1\tline2");
    assert!(msg.starts_with("VYOMA_SYSTEM:paste:"));
    assert!(msg.contains("line1\tline2"));
}
