// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Public undo types re-exported from the library crate for integration tests.

/// A single reversible action on the undo/redo stack.
#[derive(Debug, Clone)]
pub struct UndoEntry {
    /// The IPC command that was originally executed.
    pub command: String,
    /// The IPC command that reverses `command`.
    pub undo_command: String,
    /// Timestamp (ms since UNIX epoch) when the action was recorded.
    pub timestamp_ms: u64,
}
