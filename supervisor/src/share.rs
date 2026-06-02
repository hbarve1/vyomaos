// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P64: App Inter-Op Share Sheet
//!
//! Protocol:
//!   1. Source app sends `@supervisor: share <mime> <data>`
//!      e.g. `@supervisor: share text/plain Hello world`
//!   2. Supervisor stores a `SharePayload` and broadcasts
//!      `VYOMA_SYSTEM:share-available:<mime>` to all running display apps.
//!   3. Target app sends `@supervisor: share-accept` to receive
//!      `VYOMA_SYSTEM:share-data:<mime>:<data>`.
//!   4. Payload auto-clears after accept, explicit cancel, or 30s timeout.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use crate::lock_or_recover;

/// Timeout in seconds after which a pending share is automatically cleared.
pub const SHARE_TIMEOUT_SECS: u64 = 30;

/// Payload held while a share operation is pending.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharePayload {
    /// MIME type of the shared content (e.g. `text/plain`, `image/png`).
    pub mime: String,
    /// Opaque data string carried by the share.
    pub data: String,
    /// Name of the app that initiated the share.
    pub source_app: String,
}

/// Internal state: payload + timestamp for timeout enforcement.
#[derive(Debug, Clone)]
struct ShareState {
    payload: SharePayload,
    created: Instant,
}

/// Global share state: `Some(state)` while a share is pending, `None` otherwise.
static SHARE_STATE: OnceLock<Mutex<Option<ShareState>>> = OnceLock::new();

/// Access the global share state mutex.
fn share_state() -> &'static Mutex<Option<ShareState>> {
    SHARE_STATE.get_or_init(|| Mutex::new(None))
}

/// Begin a share operation. Stores the payload; any previous share is replaced.
///
/// Returns `Ok(())` if the payload was stored, or `Err(msg)` on validation failure.
pub fn share_start(source_app: &str, mime: &str, data: &str) -> Result<(), String> {
    if mime.is_empty() {
        return Err("share: mime type must not be empty".to_string());
    }
    if data.is_empty() {
        return Err("share: data must not be empty".to_string());
    }
    let state = ShareState {
        payload: SharePayload {
            mime: mime.to_string(),
            data: data.to_string(),
            source_app: source_app.to_string(),
        },
        created: Instant::now(),
    };
    *lock_or_recover(&share_state()) = Some(state);
    Ok(())
}

/// Accept the current share, returning the payload if one is pending and not expired.
/// Clears the share state on success.
pub fn share_accept() -> Result<SharePayload, String> {
    let mut guard = lock_or_recover(&share_state());
    match guard.take() {
        Some(state) => {
            if state.created.elapsed().as_secs() >= SHARE_TIMEOUT_SECS {
                Err("share: expired (30s timeout)".to_string())
            } else {
                Ok(state.payload)
            }
        }
        None => Err("share: no pending share".to_string()),
    }
}

/// Cancel the current share operation.
/// Returns `true` if there was an active share, `false` if already idle.
pub fn share_cancel() -> bool {
    lock_or_recover(&share_state()).take().is_some()
}

/// Check whether a share is currently pending (and not expired).
pub fn is_share_pending() -> bool {
    let guard = lock_or_recover(&share_state());
    match guard.as_ref() {
        Some(state) => state.created.elapsed().as_secs() < SHARE_TIMEOUT_SECS,
        None => false,
    }
}

/// Clear expired shares. Called periodically (e.g. from a watchdog or timer).
/// Returns `true` if an expired share was cleared.
pub fn clear_expired() -> bool {
    let mut guard = lock_or_recover(&share_state());
    if let Some(ref state) = *guard {
        if state.created.elapsed().as_secs() >= SHARE_TIMEOUT_SECS {
            *guard = None;
            return true;
        }
    }
    false
}

/// Format the broadcast notification sent to all display apps.
///
/// Format: `VYOMA_SYSTEM:share-available:<mime>`
pub fn format_share_available(mime: &str) -> String {
    format!("VYOMA_SYSTEM:share-available:{mime}")
}

/// Format the data delivery message sent to the accepting app.
///
/// Format: `VYOMA_SYSTEM:share-data:<mime>:<data>`
pub fn format_share_data(payload: &SharePayload) -> String {
    format!("VYOMA_SYSTEM:share-data:{}:{}", payload.mime, payload.data)
}

// ── For testing: allow resetting internal state ──────────────────────────────

#[cfg(test)]
pub fn reset_state() {
    *share_state().lock().unwrap() = None;
}

#[cfg(test)]
pub fn set_created_instant(instant: Instant) {
    let mut guard = share_state().lock().unwrap();
    if let Some(ref mut state) = *guard {
        state.created = instant;
    }
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_share_start_stores_payload() {
        reset_state();
        let result = share_start("notes-app", "text/plain", "Hello world");
        assert!(result.is_ok());
        assert!(is_share_pending());
    }

    #[test]
    fn test_share_start_rejects_empty_mime() {
        reset_state();
        let result = share_start("app", "", "data");
        assert!(result.is_err());
        assert!(!is_share_pending());
    }

    #[test]
    fn test_share_start_rejects_empty_data() {
        reset_state();
        let result = share_start("app", "text/plain", "");
        assert!(result.is_err());
        assert!(!is_share_pending());
    }

    #[test]
    fn test_share_accept_returns_payload() {
        reset_state();
        share_start("notes-app", "text/plain", "Hello world").unwrap();
        let payload = share_accept().unwrap();
        assert_eq!(payload.source_app, "notes-app");
        assert_eq!(payload.mime, "text/plain");
        assert_eq!(payload.data, "Hello world");
        // State is cleared after accept
        assert!(!is_share_pending());
    }

    #[test]
    fn test_share_accept_no_pending() {
        reset_state();
        let result = share_accept();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("no pending share"));
    }

    #[test]
    fn test_share_accept_expired() {
        reset_state();
        share_start("app", "text/plain", "data").unwrap();
        // Force the share to look expired by backdating the created instant
        let expired = Instant::now() - Duration::from_secs(SHARE_TIMEOUT_SECS + 1);
        set_created_instant(expired);
        let result = share_accept();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_share_cancel_clears_active() {
        reset_state();
        share_start("app", "text/plain", "data").unwrap();
        assert!(share_cancel());
        assert!(!is_share_pending());
    }

    #[test]
    fn test_share_cancel_returns_false_when_idle() {
        reset_state();
        assert!(!share_cancel());
    }

    #[test]
    fn test_share_start_replaces_previous() {
        reset_state();
        share_start("app1", "text/plain", "first").unwrap();
        share_start("app2", "image/png", "second").unwrap();
        let payload = share_accept().unwrap();
        assert_eq!(payload.source_app, "app2");
        assert_eq!(payload.mime, "image/png");
        assert_eq!(payload.data, "second");
    }

    #[test]
    fn test_is_share_pending_false_when_expired() {
        reset_state();
        share_start("app", "text/plain", "data").unwrap();
        let expired = Instant::now() - Duration::from_secs(SHARE_TIMEOUT_SECS + 1);
        set_created_instant(expired);
        assert!(!is_share_pending());
    }

    #[test]
    fn test_clear_expired_removes_stale() {
        reset_state();
        share_start("app", "text/plain", "data").unwrap();
        let expired = Instant::now() - Duration::from_secs(SHARE_TIMEOUT_SECS + 1);
        set_created_instant(expired);
        assert!(clear_expired());
        assert!(!is_share_pending());
    }

    #[test]
    fn test_clear_expired_noop_when_fresh() {
        reset_state();
        share_start("app", "text/plain", "data").unwrap();
        // A freshly created share should not be expired.
        assert!(!clear_expired());
    }

    #[test]
    fn test_clear_expired_noop_when_empty() {
        reset_state();
        assert!(!clear_expired());
    }

    #[test]
    fn test_format_share_available() {
        assert_eq!(
            format_share_available("text/plain"),
            "VYOMA_SYSTEM:share-available:text/plain"
        );
    }

    #[test]
    fn test_format_share_data() {
        let payload = SharePayload {
            mime: "text/plain".to_string(),
            data: "Hello world".to_string(),
            source_app: "notes".to_string(),
        };
        assert_eq!(
            format_share_data(&payload),
            "VYOMA_SYSTEM:share-data:text/plain:Hello world"
        );
    }
}
