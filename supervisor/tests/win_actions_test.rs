// Unit tests for supervisor/src/win_actions.rs — drag, snap, minimize logic.
//
// The actual functions in win_actions.rs are gated by #[cfg(target_os = "linux")]
// and depend on display/framebuffer.  We test the pure arithmetic and state
// machine logic that those functions rely on.

use supervisor::windows::{drag_delta, nearest_tiled_slot, compute_tiling};

// ── Drag delta computation ──────────────────────────────────────────────────

#[test]
fn test_drag_delta_right_down() {
    assert_eq!(drag_delta(100, 200, 150, 250), (50, 50));
}

#[test]
fn test_drag_delta_left_up() {
    assert_eq!(drag_delta(150, 250, 100, 200), (-50, -50));
}

#[test]
fn test_drag_delta_zero_movement() {
    assert_eq!(drag_delta(300, 400, 300, 400), (0, 0));
}

// ── Drag clamping arithmetic (mirrors apply_drag_update clamping) ───────────

#[test]
fn test_drag_clamp_left_boundary() {
    let (wx, _wy, ww, _wh): (u32, u32, u32, u32) = (10, 50, 400, 300);
    let (dx, _dy) = drag_delta(100, 100, 50, 100); // dx = -50
    let screen_w: i32 = 1440;
    let new_x = (wx as i32 + dx).clamp(0, (screen_w - ww as i32).max(0)) as u32;
    assert_eq!(new_x, 0); // clamped to 0
}

#[test]
fn test_drag_clamp_right_boundary() {
    let (wx, _wy, ww, _wh): (u32, u32, u32, u32) = (1400, 50, 400, 300);
    let (dx, _dy) = drag_delta(100, 100, 200, 100); // dx = 100
    let screen_w: i32 = 1440;
    let new_x = (wx as i32 + dx).clamp(0, (screen_w - ww as i32).max(0)) as u32;
    assert_eq!(new_x, 1040); // 1440 - 400 = 1040
}

#[test]
fn test_drag_clamp_top_boundary() {
    let (_wx, wy, _ww, wh): (u32, u32, u32, u32) = (100, 10, 400, 300);
    let (_dx, dy) = drag_delta(100, 100, 100, 50); // dy = -50
    let screen_h: i32 = 900;
    let menubar_h: i32 = 24;
    let new_y = (wy as i32 + dy).clamp(menubar_h, (screen_h - wh as i32).max(menubar_h)) as u32;
    assert_eq!(new_y, 24); // clamped to menubar
}

#[test]
fn test_drag_clamp_bottom_boundary() {
    let (_wx, wy, _ww, wh): (u32, u32, u32, u32) = (100, 800, 400, 300);
    let (_dx, dy) = drag_delta(100, 100, 100, 200); // dy = 100
    let screen_h: i32 = 900;
    let menubar_h: i32 = 24;
    let new_y = (wy as i32 + dy).clamp(menubar_h, (screen_h - wh as i32).max(menubar_h)) as u32;
    assert_eq!(new_y, 600); // 900 - 300 = 600
}

// ── Snap-back logic (nearest_tiled_slot) ────────────────────────────────────

#[test]
fn test_snap_within_40px() {
    let slot = nearest_tiled_slot((30, 54), 1, 1440, 900, 24);
    assert!(slot.is_some(), "window 30px from slot should snap");
}

#[test]
fn test_snap_beyond_40px() {
    let slot = nearest_tiled_slot((100, 100), 1, 1440, 900, 24);
    assert!(slot.is_none(), "window 100px from slot should not snap");
}

#[test]
fn test_snap_multi_app_closest() {
    // 4 apps on 1000x800, menubar=0:
    // Grid: 2 cols, 2 rows -> slots at (0,0), (500,0), (0,400), (500,400)
    let slot = nearest_tiled_slot((505, 5), 4, 1000, 800, 0);
    assert!(slot.is_some());
    let (sx, sy, _, _) = slot.unwrap();
    assert_eq!(sx, 500);
    assert_eq!(sy, 0);
}

// ── Minimize strip layout arithmetic ────────────────────────────────────────

fn compute_strip_pos(count: u32, screen_w: u32, screen_h: u32) -> (u32, u32) {
    let strip_x = (count * 240) % screen_w;
    let row = (count * 240) / screen_w;
    let strip_y = screen_h - 28 - row * 28;
    (strip_x, strip_y)
}

#[test]
fn test_minimize_strip_first() {
    assert_eq!(compute_strip_pos(0, 1440, 900), (0, 872));
}

#[test]
fn test_minimize_strip_second() {
    assert_eq!(compute_strip_pos(1, 1440, 900), (240, 872));
}

#[test]
fn test_minimize_strip_wrap_row() {
    // 6 * 240 = 1440, wraps: x=0, row=1
    assert_eq!(compute_strip_pos(6, 1440, 900), (0, 844));
}

#[test]
fn test_minimize_strip_third_row() {
    // 12 * 240 = 2880, row=2
    assert_eq!(compute_strip_pos(12, 1440, 900), (0, 816));
}

// ── DragState struct compile check ──────────────────────────────────────────
// DragState is private to the binary crate; verify we can at least use the
// pure functions it depends on.

#[test]
fn test_drag_state_pure_deps_compile() {
    let _ = drag_delta(0, 0, 10, 10);
    let _ = nearest_tiled_slot((0, 0), 1, 800, 600, 0);
    let _ = compute_tiling(2, 800, 600);
}
