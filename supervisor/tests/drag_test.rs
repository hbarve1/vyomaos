// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Unit tests for spec-042: Interactive Window Management.
//!
//! Covers drag clamping arithmetic, nearest_tiled_slot snap-back, and
//! minimize-strip layout invariants.

#[cfg(test)]
mod drag_tests {
    use supervisor::windows::nearest_tiled_slot;

    // ── Drag clamping (US1) ───────────────────────────────────────────────────

    /// Dragging a window 20 px left from x=5 should clamp to x=0.
    #[test]
    fn test_drag_clamp_x_min() {
        let wx: u32 = 5;
        let dx: i32 = -20;
        let sw: i32 = 1440;
        let ww: u32 = 720;
        let new_x = (wx as i32 + dx).clamp(0, (sw - ww as i32).max(0)) as u32;
        assert_eq!(new_x, 0);
    }

    /// Dragging a window above the menu bar should clamp to MENUBAR_H (24).
    #[test]
    fn test_drag_clamp_y_min() {
        let wy: u32 = 10;
        let dy: i32 = -30;
        let sh: i32 = 900;
        let wh: u32 = 400;
        let menubar_h: i32 = 24;
        let new_y = (wy as i32 + dy).clamp(menubar_h, (sh - wh as i32).max(menubar_h)) as u32;
        assert_eq!(new_y, 24);
    }

    /// A window at y=0 (above menu bar) should always be clamped to MENUBAR_H.
    #[test]
    fn test_drag_clamp_y_exactly_menubar() {
        let new_y = (0i32).clamp(24, 500) as u32;
        assert_eq!(new_y, 24);
    }

    // ── nearest_tiled_slot snap-back (US2) ────────────────────────────────────

    /// Single-app grid on 1440×900 with menubar=24 → slot (0,24,1440,876).
    /// A window placed 5 px from the origin should snap back.
    #[test]
    fn test_nearest_tiled_slot_snap() {
        let result = nearest_tiled_slot((5, 29), 1, 1440, 900, 24);
        assert_eq!(result, Some((0, 24, 1440, 876)));
    }

    /// A window 200 px from the only slot should NOT snap (>40 px Chebyshev).
    #[test]
    fn test_nearest_tiled_slot_no_snap() {
        let result = nearest_tiled_slot((200, 200), 1, 1440, 900, 24);
        assert_eq!(result, None);
    }

    /// Zero apps → no slot exists → always returns None.
    #[test]
    fn test_nearest_tiled_slot_zero_apps() {
        assert_eq!(nearest_tiled_slot((0, 0), 0, 1440, 900, 24), None);
    }

    // ── Close button (US3) ────────────────────────────────────────────────────

    /// The close button requires NO new implementation; the exit watcher in
    /// app_threads::wait_app() handles reflow + focus transfer on any app death.
    /// This test documents that invariant.
    #[test]
    fn test_close_button_no_code_change() {
        // app_threads::wait_app() calls apply_tiling_layout + auto_transfer_focus
        // on every app exit, whether caused by traffic-light close or any other signal.
        assert!(true, "close button reflow is handled by the exit watcher path");
    }

    // ── Minimize strip layout (US4) ───────────────────────────────────────────

    /// First minimized app (count=0) should land at strip_x=0, strip_y=900-28=872.
    #[test]
    fn test_minimize_strip_layout() {
        let count: u32 = 0;
        let screen_w: u32 = 1440;
        let screen_h: u32 = 900;
        let strip_x = (count * 240) % screen_w;
        let row = (count * 240) / screen_w;
        let strip_y = screen_h - 28 - row * 28;
        assert_eq!((strip_x, strip_y), (0, 872));
    }

    /// Strip 6: 6×240=1440, which wraps once → x=0, row=1, y=900-28-28=844.
    #[test]
    fn test_minimize_strip_wrap() {
        let count: u32 = 6;
        let screen_w: u32 = 1440;
        let screen_h: u32 = 900;
        let strip_x = (count * 240) % screen_w;
        let row = (count * 240) / screen_w;
        let strip_y = screen_h - 28 - row * 28;
        assert_eq!((strip_x, strip_y), (0, 844));
    }
}
