// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Touch event dispatcher (TM09).
//!
//! Routes `VYOMA_INPUT:touch:` lines to the focused app's stdin, but only
//! if that app declared `touch = true` in its manifest.
//!
//! Touch events are demultiplexed from the same virtio-input pipe as mouse events.
//! The supervisor reads `VYOMA_INPUT:touch:` prefixed lines and calls
//! `dispatch_touch` to deliver them.

use supervisor::touch_input::parse_touch_event;
use crate::{log_info, AppRegistry, FocusedApp, Inbox};
use supervisor::logging::Subsystem;

// ── run_touch_input ───────────────────────────────────────────────────────────

/// Body of the touch-input thread.
///
/// Attempts to open a virtio-input touchscreen device from `/dev/input/eventN`.
/// On mobile/ARM64 targets the kernel exposes the touchscreen as an ABS input
/// device.  On headless or desktop builds no touchscreen is present so the
/// function logs and returns immediately.
///
/// In the current phase the raw evdev parsing is left to future work; instead
/// the thread reads pre-formatted `VYOMA_INPUT:touch:` lines from a named
/// pipe at `/dev/vyoma-touch` if present.  This keeps the supervisor portable
/// while still exercising the capability-check path in CI.
///
/// Only compiled on Linux.
#[cfg(target_os = "linux")]
pub fn run_touch_input(inbox: Inbox, focused: FocusedApp, registry: AppRegistry) {
    use std::io::BufRead;

    const TOUCH_PIPE: &str = "/dev/vyoma-touch";

    let f = match std::fs::File::open(TOUCH_PIPE) {
        Ok(f) => f,
        Err(_) => {
            log_info!(
                Subsystem::Input, None,
                "touch-input: no touchscreen device at {TOUCH_PIPE}, disabling"
            );
            return;
        }
    };

    log_info!(Subsystem::Input, None, "touch-input: reading from {TOUCH_PIPE}");

    for line in std::io::BufReader::new(f).lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.starts_with("VYOMA_INPUT:touch:") {
            dispatch_touch(&line, &inbox, &registry, &focused);
        }
    }
}

/// Dispatch a raw touch-event line to the focused app, if it is touch-capable.
///
/// `line` must be a `VYOMA_INPUT:touch:` prefixed string.
/// The line is delivered as-is to the focused app's inbox so the app can
/// parse it directly with the same format it would receive from stdin.
pub fn dispatch_touch(
    line:         &str,
    inbox:        &Inbox,
    app_registry: &AppRegistry,
    focused:      &FocusedApp,
) {
    // Only process lines that are valid touch events.
    if parse_touch_event(line).is_none() {
        return;
    }

    // Get the currently focused app name.
    let focused_name: Option<String> = focused.lock().unwrap().clone();

    let target = match focused_name {
        Some(name) => name,
        None => return, // no focused app — drop event
    };

    // Check if the focused app has the touch capability.
    let has_touch = {
        let reg = app_registry.lock().unwrap();
        reg.get(&target)
            .map(|st| st.lock().unwrap().has_touch)
            .unwrap_or(false)
    };

    if !has_touch {
        return; // capability isolation — app did not declare touch = true
    }

    // Deliver the raw event line to the app's inbox.
    if let Some(tx) = inbox.lock().unwrap().get(&target) {
        let _ = tx.send(line.to_string());
        log_info!(
            Subsystem::Input,
            Some(target.as_str()),
            "touch dispatch: {line}"
        );
    }
}

