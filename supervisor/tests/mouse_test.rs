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

// ── dispatch event format ─────────────────────────────────────────────────────

fn format_mouse_event(lx: i32, ly: i32, btn: u8) -> String {
    if btn == 0 {
        format!("VYOMA_INPUT:mouse:move:{lx},{ly}")
    } else {
        let bname = match btn { 1 => "left", 2 => "right", 4 => "middle", _ => "left" };
        format!("VYOMA_INPUT:mouse:click:{lx},{ly}:{bname}")
    }
}

#[test]
fn move_event_format() {
    assert_eq!(format_mouse_event(120, 80, 0), "VYOMA_INPUT:mouse:move:120,80");
}

#[test]
fn left_click_format() {
    assert_eq!(format_mouse_event(10, 20, 1), "VYOMA_INPUT:mouse:click:10,20:left");
}

#[test]
fn right_click_format() {
    assert_eq!(format_mouse_event(0, 0, 2), "VYOMA_INPUT:mouse:click:0,0:right");
}

#[test]
fn middle_click_format() {
    assert_eq!(format_mouse_event(5, 5, 4), "VYOMA_INPUT:mouse:click:5,5:middle");
}

#[test]
fn unknown_btn_falls_back_to_left() {
    assert_eq!(format_mouse_event(1, 1, 7), "VYOMA_INPUT:mouse:click:1,1:left");
}

#[test]
fn move_event_zero_coords() {
    assert_eq!(format_mouse_event(0, 0, 0), "VYOMA_INPUT:mouse:move:0,0");
}
