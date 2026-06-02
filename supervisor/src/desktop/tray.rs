// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! System tray / status icon helpers for the menu bar.
//!
//! Computes text labels for volume, lock, and network indicators so that
//! `chrome::draw_menubar` can render them without pulling in subsystem details.

use crate::lock_or_recover;

/// A single tray indicator with its display text and colour.
pub struct TrayItem {
    pub text: String,
    pub color: u32,
}

const RED:   u32 = 0xFF5F57FF; // muted / locked indicator
const LABEL: u32 = 0xEBEBEBFF; // primary label (near-white)
const DIM:   u32 = 0x8E8E93FF; // secondary / dim label

/// Build the volume tray indicator from current audio state.
pub fn volume_indicator() -> TrayItem {
    let state = lock_or_recover(&crate::audio::audio_state());
    if state.muted {
        return TrayItem { text: "M".to_string(), color: RED };
    }
    let label = match state.volume {
        0..=33  => "Vol:Lo",
        34..=66 => "Vol:Med",
        _       => "Vol:Hi",
    };
    TrayItem { text: label.to_string(), color: LABEL }
}

/// Build the lock tray indicator.  Returns `Some` only when the system is locked.
pub fn lock_indicator() -> Option<TrayItem> {
    if crate::is_locked() {
        Some(TrayItem { text: "Locked".to_string(), color: RED })
    } else {
        None
    }
}

/// Build the network tray indicator (static text for now).
pub fn network_indicator() -> TrayItem {
    TrayItem { text: "Net".to_string(), color: DIM }
}

/// Collect all active tray items in display order (left-to-right before the clock).
pub fn collect_items() -> Vec<TrayItem> {
    let mut items = Vec::with_capacity(4);
    items.push(crate::battery::battery_indicator());
    items.push(volume_indicator());
    if let Some(lock) = lock_indicator() {
        items.push(lock);
    }
    items.push(network_indicator());
    items
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_volume_indicator_default() {
        let item = volume_indicator();
        assert!(
            item.text == "Vol:Lo" || item.text == "Vol:Med" || item.text == "Vol:Hi",
            "unexpected volume text: {}", item.text
        );
        assert_eq!(item.color, LABEL);
    }

    #[test]
    fn test_lock_indicator_unlocked() {
        // Default lock state is false
        assert!(lock_indicator().is_none());
    }

    #[test]
    fn test_network_indicator() {
        let item = network_indicator();
        assert_eq!(item.text, "Net");
    }

    #[test]
    fn test_collect_items_count() {
        let items = collect_items();
        // At minimum: battery + volume + network = 3 items (lock only when locked)
        assert!(items.len() >= 3);
    }
}
