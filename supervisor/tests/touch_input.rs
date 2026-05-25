// TM01: Failing tests for touch event parsing.
//
// parse_touch_event must handle:
//   VYOMA_INPUT:touch:tap:<x>,<y>
//   VYOMA_INPUT:touch:swipe:<dx>,<dy>
// and return None for unrelated lines.

use supervisor::touch_input::{parse_touch_event, TouchEvent};

// ── tap ───────────────────────────────────────────────────────────────────────

#[test]
fn tap_origin() {
    assert_eq!(
        parse_touch_event("VYOMA_INPUT:touch:tap:0,0"),
        Some(TouchEvent::Tap { x: 0, y: 0 })
    );
}

#[test]
fn tap_mid_screen() {
    assert_eq!(
        parse_touch_event("VYOMA_INPUT:touch:tap:540,1000"),
        Some(TouchEvent::Tap { x: 540, y: 1000 })
    );
}

#[test]
fn tap_bottom_right() {
    assert_eq!(
        parse_touch_event("VYOMA_INPUT:touch:tap:1079,2339"),
        Some(TouchEvent::Tap { x: 1079, y: 2339 })
    );
}

#[test]
fn tap_leading_whitespace_not_matched() {
    // Lines with leading whitespace must not parse.
    assert_eq!(parse_touch_event("  VYOMA_INPUT:touch:tap:10,20"), None);
}

// ── swipe ─────────────────────────────────────────────────────────────────────

#[test]
fn swipe_positive_delta() {
    assert_eq!(
        parse_touch_event("VYOMA_INPUT:touch:swipe:30,50"),
        Some(TouchEvent::Swipe { dx: 30, dy: 50 })
    );
}

#[test]
fn swipe_negative_dy() {
    assert_eq!(
        parse_touch_event("VYOMA_INPUT:touch:swipe:0,-100"),
        Some(TouchEvent::Swipe { dx: 0, dy: -100 })
    );
}

#[test]
fn swipe_negative_dx_positive_dy() {
    assert_eq!(
        parse_touch_event("VYOMA_INPUT:touch:swipe:-50,200"),
        Some(TouchEvent::Swipe { dx: -50, dy: 200 })
    );
}

#[test]
fn swipe_both_negative() {
    assert_eq!(
        parse_touch_event("VYOMA_INPUT:touch:swipe:-10,-20"),
        Some(TouchEvent::Swipe { dx: -10, dy: -20 })
    );
}

#[test]
fn swipe_zero_delta() {
    assert_eq!(
        parse_touch_event("VYOMA_INPUT:touch:swipe:0,0"),
        Some(TouchEvent::Swipe { dx: 0, dy: 0 })
    );
}

// ── unrelated lines ───────────────────────────────────────────────────────────

#[test]
fn mouse_move_not_matched() {
    assert_eq!(parse_touch_event("VYOMA_INPUT:mouse:move:10,20"), None);
}

#[test]
fn draw_command_not_matched() {
    assert_eq!(parse_touch_event("VYOMA_DRAW:fill_rect:0,0,100,100,4278190080"), None);
}

#[test]
fn empty_line_not_matched() {
    assert_eq!(parse_touch_event(""), None);
}

#[test]
fn ipc_message_not_matched() {
    assert_eq!(parse_touch_event("@supervisor: list"), None);
}

#[test]
fn touch_prefix_only_not_matched() {
    assert_eq!(parse_touch_event("VYOMA_INPUT:touch:"), None);
}

#[test]
fn tap_missing_y_not_matched() {
    assert_eq!(parse_touch_event("VYOMA_INPUT:touch:tap:50"), None);
}

#[test]
fn tap_non_numeric_not_matched() {
    assert_eq!(parse_touch_event("VYOMA_INPUT:touch:tap:abc,def"), None);
}

#[test]
fn swipe_non_numeric_not_matched() {
    assert_eq!(parse_touch_event("VYOMA_INPUT:touch:swipe:abc,def"), None);
}
