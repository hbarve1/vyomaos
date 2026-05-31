// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Inter-app drag & drop via IPC.
//!
//! Protocol:
//!   1. Source app sends `@supervisor: drag-start <mime> <data>`
//!      e.g. `@supervisor: drag-start text/path /data/notes.txt`
//!   2. Supervisor stores a `DragPayload` with source app, MIME type, and data.
//!   3. On mouse-up over a target app window, the supervisor delivers
//!      `VYOMA_SYSTEM:drop:<mime>:<data>` to the target app's stdin.
//!   4. Source app (or supervisor) can cancel with `@supervisor: drag-cancel`.

use std::sync::{Mutex, OnceLock};

/// Payload held while a drag operation is in progress.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DragPayload {
    /// Name of the app that initiated the drag.
    pub source_app: String,
    /// MIME type of the dragged content (e.g. `text/plain`, `text/path`).
    pub mime: String,
    /// Opaque data string carried by the drag.
    pub data: String,
}

/// Global drag-and-drop state: `Some(payload)` while a drag is active, `None` otherwise.
static DRAG_DROP_STATE: OnceLock<Mutex<Option<DragPayload>>> = OnceLock::new();

/// Access the global drag-drop state mutex.
pub fn drag_drop_state() -> &'static Mutex<Option<DragPayload>> {
    DRAG_DROP_STATE.get_or_init(|| Mutex::new(None))
}

/// Begin a drag operation. Stores the payload; any previous drag is replaced.
///
/// Returns `Ok(())` if the payload was stored, or `Err(msg)` on validation failure.
pub fn drag_start(source_app: &str, mime: &str, data: &str) -> Result<(), String> {
    if mime.is_empty() {
        return Err("drag-start: mime type must not be empty".to_string());
    }
    if data.is_empty() {
        return Err("drag-start: data must not be empty".to_string());
    }
    let payload = DragPayload {
        source_app: source_app.to_string(),
        mime: mime.to_string(),
        data: data.to_string(),
    };
    *drag_drop_state().lock().unwrap() = Some(payload);
    Ok(())
}

/// Cancel the current drag operation, clearing any stored payload.
/// Returns `true` if there was an active drag, `false` if already idle.
pub fn drag_cancel() -> bool {
    drag_drop_state().lock().unwrap().take().is_some()
}

/// Take the current drag payload (if any), clearing the state.
/// Used on mouse-up to deliver the drop and reset.
pub fn take_payload() -> Option<DragPayload> {
    drag_drop_state().lock().unwrap().take()
}

/// Check whether a drag operation is currently active.
pub fn is_drag_active() -> bool {
    drag_drop_state().lock().unwrap().is_some()
}

/// Format the drop message delivered to the target app's stdin.
///
/// Format: `VYOMA_SYSTEM:drop:<mime>:<data>`
pub fn format_drop_message(payload: &DragPayload) -> String {
    format!("VYOMA_SYSTEM:drop:{}:{}", payload.mime, payload.data)
}

/// Deliver a drag-and-drop payload to the topmost window under the cursor.
/// Called on left-button release; no-ops if no drag is active.
#[cfg(target_os = "linux")]
pub fn deliver_drop_if_active(
    cx: i32,
    cy: i32,
    inbox: &crate::Inbox,
    registry: &crate::AppRegistry,
) {
    use crate::{log_info, send_reply};
    use supervisor::logging::Subsystem;

    if !is_drag_active() {
        return;
    }

    // Find topmost window under cursor (z-order aware).
    let z_snap: Vec<String> = crate::Z_ORDER.get()
        .map(|m| m.lock().unwrap().clone()).unwrap_or_default();

    let target_app: Option<String> = {
        let reg = registry.lock().unwrap();
        let mut found = None;
        for name in &z_snap {
            let Some(state_arc) = reg.get(name) else { continue };
            let st = state_arc.lock().unwrap();
            let Some((wx, wy, ww, wh)) = st.win_region else { continue };
            if cx >= wx as i32 && cy >= wy as i32
                && cx < (wx + ww) as i32 && cy < (wy + wh) as i32
            {
                found = Some(name.clone());
                break;
            }
        }
        found
    };

    if let Some(target) = target_app {
        if let Some(payload) = take_payload() {
            // Do not deliver drop to the source app itself.
            if target != payload.source_app {
                let msg = format_drop_message(&payload);
                send_reply(&target, &msg, inbox);
                log_info!(Subsystem::Ipc, Some(target.as_str()),
                    "drop delivered: mime={} data={} from={}",
                    payload.mime, payload.data, payload.source_app);
            } else {
                log_info!(Subsystem::Ipc, Some(target.as_str()),
                    "drag cancelled: dropped on source app");
            }
        }
    } else {
        // Mouse released outside any window — cancel the drag.
        drag_cancel();
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: ensure drag state is clean before each test.
    fn reset_state() {
        *drag_drop_state().lock().unwrap() = None;
    }

    #[test]
    fn test_drag_start_stores_payload() {
        reset_state();
        let result = drag_start("file-manager", "text/path", "/data/notes.txt");
        assert!(result.is_ok());
        let state = drag_drop_state().lock().unwrap();
        let payload = state.as_ref().unwrap();
        assert_eq!(payload.source_app, "file-manager");
        assert_eq!(payload.mime, "text/path");
        assert_eq!(payload.data, "/data/notes.txt");
    }

    #[test]
    fn test_drag_start_rejects_empty_mime() {
        reset_state();
        let result = drag_start("app", "", "data");
        assert!(result.is_err());
        assert!(drag_drop_state().lock().unwrap().is_none());
    }

    #[test]
    fn test_drag_start_rejects_empty_data() {
        reset_state();
        let result = drag_start("app", "text/plain", "");
        assert!(result.is_err());
        assert!(drag_drop_state().lock().unwrap().is_none());
    }

    #[test]
    fn test_drag_cancel_clears_active() {
        reset_state();
        drag_start("app", "text/plain", "hello").unwrap();
        assert!(drag_cancel());
        assert!(drag_drop_state().lock().unwrap().is_none());
    }

    #[test]
    fn test_drag_cancel_returns_false_when_idle() {
        reset_state();
        assert!(!drag_cancel());
    }

    #[test]
    fn test_take_payload_clears_state() {
        reset_state();
        drag_start("src", "text/uri", "http://example.com").unwrap();
        let payload = take_payload();
        assert!(payload.is_some());
        let p = payload.unwrap();
        assert_eq!(p.source_app, "src");
        assert_eq!(p.mime, "text/uri");
        assert_eq!(p.data, "http://example.com");
        // State is now cleared
        assert!(!is_drag_active());
    }

    #[test]
    fn test_is_drag_active() {
        reset_state();
        assert!(!is_drag_active());
        drag_start("a", "text/plain", "x").unwrap();
        assert!(is_drag_active());
        drag_cancel();
        assert!(!is_drag_active());
    }

    #[test]
    fn test_format_drop_message() {
        let payload = DragPayload {
            source_app: "file-manager".to_string(),
            mime: "text/path".to_string(),
            data: "/data/notes.txt".to_string(),
        };
        assert_eq!(
            format_drop_message(&payload),
            "VYOMA_SYSTEM:drop:text/path:/data/notes.txt"
        );
    }

    #[test]
    fn test_drag_start_replaces_previous() {
        reset_state();
        drag_start("app1", "text/plain", "first").unwrap();
        drag_start("app2", "image/png", "second").unwrap();
        let state = drag_drop_state().lock().unwrap();
        let p = state.as_ref().unwrap();
        assert_eq!(p.source_app, "app2");
        assert_eq!(p.mime, "image/png");
        assert_eq!(p.data, "second");
    }
}
