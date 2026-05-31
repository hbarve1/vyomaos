// Integration tests for the undo/redo public types.

#[test]
fn undo_entry_fields() {
    let entry = supervisor::undo_public::UndoEntry {
        command: "kill foo".to_string(),
        undo_command: "run /apps/foo/vyoma.toml".to_string(),
        timestamp_ms: 1234567890,
    };
    assert_eq!(entry.command, "kill foo");
    assert_eq!(entry.undo_command, "run /apps/foo/vyoma.toml");
    assert_eq!(entry.timestamp_ms, 1234567890);
}

#[test]
fn undo_entry_clone() {
    let entry = supervisor::undo_public::UndoEntry {
        command: "mute".to_string(),
        undo_command: "unmute".to_string(),
        timestamp_ms: 0,
    };
    let cloned = entry.clone();
    assert_eq!(cloned.command, entry.command);
    assert_eq!(cloned.undo_command, entry.undo_command);
}
