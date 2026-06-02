// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Multi-workspace (Spaces) support.
//!
//! Each workspace is a virtual desktop; apps are assigned to exactly one workspace.
//! System-layer apps (win_z >= Z_DOCK) are visible on all workspaces.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::chrome::Z_DOCK;
use crate::AppRegistry;

/// Default number of workspaces.
const DEFAULT_WORKSPACE_COUNT: usize = 4;

/// Workspace manager state.
pub struct WorkspaceManager {
    /// Currently active workspace (0-indexed).
    pub current: usize,
    /// Total number of workspaces.
    pub count: usize,
    /// Maps app name to its assigned workspace index.
    pub app_workspace: HashMap<String, usize>,
}

impl WorkspaceManager {
    fn new() -> Self {
        Self {
            current: 0,
            count: DEFAULT_WORKSPACE_COUNT,
            app_workspace: HashMap::new(),
        }
    }
}

/// Global workspace manager singleton.
static WORKSPACES: OnceLock<Mutex<WorkspaceManager>> = OnceLock::new();

/// Get or initialize the global workspace manager.
pub fn manager() -> &'static Mutex<WorkspaceManager> {
    WORKSPACES.get_or_init(|| Mutex::new(WorkspaceManager::new()))
}

/// Switch to the workspace at `idx` (0-indexed).  Clamps to `count - 1`.
/// Returns the actual workspace index switched to.
pub fn switch_to(idx: usize) -> usize {
    let mut mgr = manager().lock().unwrap();
    let target = idx.min(mgr.count.saturating_sub(1));
    mgr.current = target;
    target
}

/// Switch to the next workspace (wrapping around).
pub fn switch_next() -> usize {
    let mut mgr = manager().lock().unwrap();
    let next = (mgr.current + 1) % mgr.count;
    mgr.current = next;
    next
}

/// Switch to the previous workspace (wrapping around).
pub fn switch_prev() -> usize {
    let mut mgr = manager().lock().unwrap();
    let prev = if mgr.current == 0 { mgr.count - 1 } else { mgr.current - 1 };
    mgr.current = prev;
    prev
}

/// Return the current workspace index.
pub fn current() -> usize {
    manager().lock().unwrap().current
}

/// Return the total workspace count.
pub fn count() -> usize {
    manager().lock().unwrap().count
}

/// Check whether an app should be visible on the current workspace.
///
/// System apps (win_z >= Z_DOCK) are always visible regardless of workspace.
/// Regular apps are visible only when assigned to the current workspace.
pub fn is_visible(app_name: &str, registry: &AppRegistry) -> bool {
    // Check if system app (dock, overlay, desktop) — always visible.
    let is_system = {
        let reg = registry.lock().unwrap();
        reg.get(app_name)
            .map(|st| st.lock().unwrap().win_z >= Z_DOCK)
            .unwrap_or(false)
    };
    if is_system {
        return true;
    }

    let mgr = manager().lock().unwrap();
    let app_ws = mgr.app_workspace.get(app_name).copied().unwrap_or(0);
    app_ws == mgr.current
}

/// Move an app to a different workspace.  Clamps `workspace` to valid range.
pub fn move_app_to(app_name: &str, workspace: usize) {
    let mut mgr = manager().lock().unwrap();
    let target = workspace.min(mgr.count.saturating_sub(1));
    mgr.app_workspace.insert(app_name.to_string(), target);
}

/// Assign an app to workspace 0 if it has no assignment yet.
/// Called when a new app is spawned.
pub fn register_app(app_name: &str) {
    let mut mgr = manager().lock().unwrap();
    let current = mgr.current;
    mgr.app_workspace.entry(app_name.to_string()).or_insert(current);
}

/// Remove an app from workspace tracking (e.g. on exit).
#[allow(dead_code)]
pub fn unregister_app(app_name: &str) {
    let mut mgr = manager().lock().unwrap();
    mgr.app_workspace.remove(app_name);
}

/// Return a workspace indicator string for the menu bar.
/// Example: "● ○ ○ ○" when on workspace 0 of 4.
pub fn indicator_string() -> String {
    let mgr = manager().lock().unwrap();
    (0..mgr.count)
        .map(|i| if i == mgr.current { "\u{25CF}" } else { "\u{25CB}" })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Return the workspace index for a given app (defaults to 0).
pub fn app_workspace_index(app_name: &str) -> usize {
    manager().lock().unwrap().app_workspace.get(app_name).copied().unwrap_or(0)
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: reset the workspace manager to a clean state for isolated tests.
    /// Since OnceLock can only be set once, we manipulate the inner state instead.
    fn reset_manager() {
        let mut mgr = manager().lock().unwrap();
        mgr.current = 0;
        mgr.count = 4;
        mgr.app_workspace.clear();
    }

    #[test]
    fn test_switch_to_valid() {
        reset_manager();
        let idx = switch_to(2);
        assert_eq!(idx, 2);
        assert_eq!(current(), 2);
    }

    #[test]
    fn test_switch_to_clamps() {
        reset_manager();
        let idx = switch_to(99);
        assert_eq!(idx, 3); // clamped to count-1
    }

    #[test]
    fn test_switch_next_wraps() {
        reset_manager();
        switch_to(3);
        let idx = switch_next();
        assert_eq!(idx, 0); // wraps around
    }

    #[test]
    fn test_switch_prev_wraps() {
        reset_manager();
        switch_to(0);
        let idx = switch_prev();
        assert_eq!(idx, 3); // wraps to last
    }

    #[test]
    fn test_move_app_and_query() {
        reset_manager();
        move_app_to("calc", 2);
        assert_eq!(app_workspace_index("calc"), 2);
    }

    #[test]
    fn test_register_app_defaults_to_current() {
        reset_manager();
        switch_to(1);
        register_app("notes");
        assert_eq!(app_workspace_index("notes"), 1);
    }

    #[test]
    fn test_unregister_app() {
        reset_manager();
        register_app("temp");
        unregister_app("temp");
        // After unregister, defaults to 0
        assert_eq!(app_workspace_index("temp"), 0);
    }

    #[test]
    fn test_indicator_string() {
        reset_manager();
        switch_to(0);
        let s = indicator_string();
        assert!(s.starts_with('\u{25CF}')); // filled circle first
        assert!(s.contains('\u{25CB}'));    // open circles after
    }

    #[test]
    fn test_move_app_clamps() {
        reset_manager();
        move_app_to("app1", 100);
        assert_eq!(app_workspace_index("app1"), 3); // clamped to count-1
    }
}
