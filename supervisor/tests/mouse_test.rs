// ── traffic_light_hit ─────────────────────────────────────────────────────────

/// Mirror of the TrafficLight enum from main.rs for unit testing purposes.
#[derive(Debug, PartialEq)]
enum TrafficLight {
    Close,
    Minimize,
    Maximize,
}

/// Mirror of `traffic_light_hit` from mouse_input.rs.
///
/// Layout: tl_y = wy + 8, dot size = 12×12
///   close    at wx+14 .. wx+26
///   minimize at wx+34 .. wx+46
///   maximize at wx+54 .. wx+66
fn traffic_light_hit(cx: i32, cy: i32, wx: u32, wy: u32) -> Option<TrafficLight> {
    let tl_y = wy as i32 + 8;
    if cy < tl_y || cy >= tl_y + 12 {
        return None;
    }
    let wx = wx as i32;
    if cx >= wx + 14 && cx < wx + 26 { return Some(TrafficLight::Close);    }
    if cx >= wx + 34 && cx < wx + 46 { return Some(TrafficLight::Minimize); }
    if cx >= wx + 54 && cx < wx + 66 { return Some(TrafficLight::Maximize); }
    None
}

#[test]
fn test_traffic_light_hit_detection() {
    // Window at wx=100, wy=50 -> tl_y = 58
    let (wx, wy): (u32, u32) = (100, 50);

    // Close dot: x in [114,126), y in [58,70)
    assert_eq!(traffic_light_hit(114, 58, wx, wy), Some(TrafficLight::Close));
    assert_eq!(traffic_light_hit(125, 69, wx, wy), Some(TrafficLight::Close));
    assert_eq!(traffic_light_hit(120, 64, wx, wy), Some(TrafficLight::Close)); // center

    // Minimize dot: x in [134,146), y in [58,70)
    assert_eq!(traffic_light_hit(134, 58, wx, wy), Some(TrafficLight::Minimize));
    assert_eq!(traffic_light_hit(145, 69, wx, wy), Some(TrafficLight::Minimize));
    assert_eq!(traffic_light_hit(140, 64, wx, wy), Some(TrafficLight::Minimize)); // center

    // Maximize dot: x in [154,166), y in [58,70)
    assert_eq!(traffic_light_hit(154, 58, wx, wy), Some(TrafficLight::Maximize));
    assert_eq!(traffic_light_hit(165, 69, wx, wy), Some(TrafficLight::Maximize));
    assert_eq!(traffic_light_hit(160, 64, wx, wy), Some(TrafficLight::Maximize)); // center

    // Title bar but between / outside dots -- not a traffic-light hit
    assert_eq!(traffic_light_hit(100, 64, wx, wy), None); // left of close
    assert_eq!(traffic_light_hit(126, 64, wx, wy), None); // gap between close and minimize
    assert_eq!(traffic_light_hit(166, 64, wx, wy), None); // right of maximize

    // Above the dot row (y < tl_y)
    assert_eq!(traffic_light_hit(120, 57, wx, wy), None);

    // Below the dot row (y >= tl_y + 12)
    assert_eq!(traffic_light_hit(120, 70, wx, wy), None);

    // Well below the title bar (content area)
    assert_eq!(traffic_light_hit(120, 100, wx, wy), None);
}

#[test]
fn traffic_light_close_right_edge_excluded() {
    // x=126 is the first pixel NOT in the close dot (wx=100, dot at 114..126)
    assert_eq!(traffic_light_hit(126, 64, 100, 50), None);
}

#[test]
fn traffic_light_minimize_left_edge_inclusive() {
    assert_eq!(traffic_light_hit(134, 64, 100, 50), Some(TrafficLight::Minimize));
}

#[test]
fn traffic_light_maximize_left_edge_inclusive() {
    assert_eq!(traffic_light_hit(154, 64, 100, 50), Some(TrafficLight::Maximize));
}

#[test]
fn traffic_light_y_row_boundaries() {
    let (wx, wy): (u32, u32) = (0, 0);
    // tl_y = 8; dot spans y in [8, 20)
    assert_eq!(traffic_light_hit(14, 7,  wx, wy), None);  // one above
    assert_eq!(traffic_light_hit(14, 8,  wx, wy), Some(TrafficLight::Close)); // first row
    assert_eq!(traffic_light_hit(14, 19, wx, wy), Some(TrafficLight::Close)); // last row
    assert_eq!(traffic_light_hit(14, 20, wx, wy), None);  // one below
}

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

// ── titlebar_color_for_state ──────────────────────────────────────────────────

/// Mirror of the title-bar colour constants from main.rs.
const MAC_TITLE_ACT:   u32 = 0x323232FF; // active / focused
const MAC_TITLE_INACT: u32 = 0x282828FF; // inactive / unfocused
const MAC_TITLE_HOVER: u32 = 0x383838FF; // hovered (slightly lighter than active)

/// Mirror of `titlebar_color_for_state` from main.rs.
///
/// Priority: hovered > focused > inactive.
fn titlebar_color_for_state(focused: bool, hovered: bool) -> u32 {
    if hovered      { MAC_TITLE_HOVER }
    else if focused { MAC_TITLE_ACT   }
    else            { MAC_TITLE_INACT }
}

#[test]
fn titlebar_color_hovered_returns_hover_color() {
    // A hovered title bar (regardless of focus) must return the hover colour.
    assert_eq!(titlebar_color_for_state(false, true),  MAC_TITLE_HOVER);
}

#[test]
fn titlebar_color_focused_not_hovered_returns_active_color() {
    // Focused but not hovered → active (bright) colour.
    assert_eq!(titlebar_color_for_state(true, false), MAC_TITLE_ACT);
}

#[test]
fn titlebar_color_unfocused_not_hovered_returns_inactive_color() {
    // Neither focused nor hovered → inactive (dim) colour.
    assert_eq!(titlebar_color_for_state(false, false), MAC_TITLE_INACT);
}

#[test]
fn titlebar_color_hover_wins_over_focus() {
    // When both focused AND hovered, hover wins (hover highlight takes priority).
    assert_eq!(titlebar_color_for_state(true, true), MAC_TITLE_HOVER);
}

#[test]
fn titlebar_color_hover_is_lighter_than_inactive() {
    // Hover colour RGB luminance should be greater than inactive (visual affordance).
    let hover_r = (MAC_TITLE_HOVER >> 24) & 0xFF;
    let inact_r = (MAC_TITLE_INACT >> 24) & 0xFF;
    assert!(hover_r > inact_r, "hover R ({hover_r}) should exceed inactive R ({inact_r})");
}

// ── drag_delta ────────────────────────────────────────────────────────────────

#[test]
fn drag_delta_zero() {
    assert_eq!(supervisor::windows::drag_delta(10, 20, 10, 20), (0, 0));
}

#[test]
fn drag_delta_positive() {
    assert_eq!(supervisor::windows::drag_delta(0, 0, 5, 3), (5, 3));
}

#[test]
fn drag_delta_negative() {
    assert_eq!(supervisor::windows::drag_delta(10, 10, 3, 2), (-7, -8));
}

#[test]
fn drag_delta_mixed() {
    assert_eq!(supervisor::windows::drag_delta(5, 0, 2, 7), (-3, 7));
}
