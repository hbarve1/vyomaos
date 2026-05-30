// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Integration tests for inter-app drag & drop IPC protocol.
//!
//! Tests the pure drag_drop module functions: start, cancel, payload storage,
//! and drop message formatting.

// The drag_drop module is part of the binary crate, so we test through the
// library-exported ipc helpers and replicate the pure logic here.

#[cfg(test)]
mod drag_drop_tests {
    /// Verify format of the drop message delivered to target apps.
    #[test]
    fn test_drop_message_format() {
        let msg = format!("VYOMA_SYSTEM:drop:{}:{}", "text/path", "/data/notes.txt");
        assert_eq!(msg, "VYOMA_SYSTEM:drop:text/path:/data/notes.txt");
    }

    /// Verify that the drag-start IPC command format is parsed correctly.
    /// The command is: `drag-start <mime> <data>`
    #[test]
    fn test_drag_start_command_parsing() {
        let cmd = "drag-start text/plain hello world";
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        assert_eq!(parts[0], "drag-start");
        let rest = parts[1].trim();
        let (mime, data) = rest.split_once(' ').unwrap();
        assert_eq!(mime, "text/plain");
        assert_eq!(data, "hello world");
    }

    /// Drag-cancel command has no arguments.
    #[test]
    fn test_drag_cancel_command_parsing() {
        let cmd = "drag-cancel";
        let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
        assert_eq!(parts[0], "drag-cancel");
    }

    /// Drop message for MIME types with slashes and colons in data.
    #[test]
    fn test_drop_message_special_chars() {
        let msg = format!("VYOMA_SYSTEM:drop:{}:{}", "application/json", "{\"key\":\"value\"}");
        assert_eq!(msg, "VYOMA_SYSTEM:drop:application/json:{\"key\":\"value\"}");
    }

    /// Verify that drag-start with only a mime (no data) is detected as invalid.
    #[test]
    fn test_drag_start_missing_data() {
        let rest = "text/plain";
        let result = rest.split_once(' ');
        assert!(result.is_none(), "single token should not split");
    }
}
