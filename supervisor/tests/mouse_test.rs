/// Returns true if screen point (mx, my) is inside window (wx, wy, ww, wh).
fn hit_test(mx: i32, my: i32, wx: u32, wy: u32, ww: u32, wh: u32) -> bool {
    mx >= wx as i32
        && my >= wy as i32
        && mx < (wx + ww) as i32
        && my < (wy + wh) as i32
}

/// Convert global screen coords to window-local coords.
fn to_local(mx: i32, my: i32, wx: u32, wy: u32) -> (i32, i32) {
    (mx - wx as i32, my - wy as i32)
}

// ── hit_test ──────────────────────────────────────────────────────────────────

#[test]
fn point_inside_gui_window() {
    assert!(hit_test(100, 200, 0, 0, 1440, 440));
}

#[test]
fn point_below_gui_window() {
    assert!(!hit_test(100, 441, 0, 0, 1440, 440));
}

#[test]
fn right_edge_excluded() {
    assert!(!hit_test(1440, 0, 0, 0, 1440, 440));
}

#[test]
fn bottom_edge_excluded() {
    assert!(!hit_test(0, 440, 0, 0, 1440, 440));
}

#[test]
fn top_left_corner_inside() {
    assert!(hit_test(0, 0, 0, 0, 1440, 440));
}

#[test]
fn point_in_shell_window() {
    assert!(hit_test(0, 440, 0, 440, 1440, 460));
    assert!(hit_test(100, 500, 0, 440, 1440, 460));
    assert!(!hit_test(0, 900, 0, 440, 1440, 460));
}

#[test]
fn point_one_above_shell_window() {
    assert!(!hit_test(0, 439, 0, 440, 1440, 460));
}

// ── to_local ──────────────────────────────────────────────────────────────────

#[test]
fn local_coords_gui_window_at_origin() {
    assert_eq!(to_local(100, 200, 0, 0), (100, 200));
}

#[test]
fn local_coords_shell_window_offset() {
    assert_eq!(to_local(100, 500, 0, 440), (100, 60));
}

#[test]
fn local_coords_mid_window() {
    assert_eq!(to_local(250, 350, 200, 300), (50, 50));
}
