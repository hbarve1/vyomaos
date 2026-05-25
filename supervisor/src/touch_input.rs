// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Touch input event parser and dispatcher (TM04).
//!
//! Touch events mirror the mouse event protocol but use the `touch` prefix:
//!   `VYOMA_INPUT:touch:tap:<x>,<y>`
//!   `VYOMA_INPUT:touch:swipe:<dx>,<dy>`
//!
//! The supervisor demultiplexes by prefix:
//!   `VYOMA_INPUT:mouse:` → mouse-capable apps
//!   `VYOMA_INPUT:touch:` → touch-capable apps (touch = true in manifest)

// ── TouchEvent ────────────────────────────────────────────────────────────────

/// A parsed touch event from the virtio-touchscreen input device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TouchEvent {
    /// A single-finger tap at screen coordinates (x, y).
    Tap { x: u32, y: u32 },
    /// A swipe gesture with deltas in screen pixels.
    /// Positive dx = swipe right, positive dy = swipe down.
    Swipe { dx: i32, dy: i32 },
}

// ── parse_touch_event ─────────────────────────────────────────────────────────

/// Parse a single line of supervisor stdin and return a `TouchEvent` if the
/// line matches the `VYOMA_INPUT:touch:` prefix and has a valid payload.
///
/// Returns `None` for any line that is not a recognised touch event.
///
/// # Line formats
///
/// ```text
/// VYOMA_INPUT:touch:tap:<x>,<y>
/// VYOMA_INPUT:touch:swipe:<dx>,<dy>
/// ```
///
/// - `<x>`, `<y>` are non-negative decimal integers (u32).
/// - `<dx>`, `<dy>` are signed decimal integers (i32, may be negative).
pub fn parse_touch_event(line: &str) -> Option<TouchEvent> {
    let rest = line.strip_prefix("VYOMA_INPUT:touch:")?;

    if let Some(coords) = rest.strip_prefix("tap:") {
        parse_tap(coords)
    } else if let Some(deltas) = rest.strip_prefix("swipe:") {
        parse_swipe(deltas)
    } else {
        None
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn parse_tap(coords: &str) -> Option<TouchEvent> {
    let (xs, ys) = coords.split_once(',')?;
    let x = xs.parse::<u32>().ok()?;
    let y = ys.parse::<u32>().ok()?;
    Some(TouchEvent::Tap { x, y })
}

fn parse_swipe(deltas: &str) -> Option<TouchEvent> {
    let (dxs, dys) = deltas.split_once(',')?;
    let dx = dxs.parse::<i32>().ok()?;
    let dy = dys.parse::<i32>().ok()?;
    Some(TouchEvent::Swipe { dx, dy })
}
