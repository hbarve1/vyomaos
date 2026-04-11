/// Apply window origin offset + clip a fill_rect to window bounds.
/// Returns the global (ax, ay, aw, ah) rectangle, or None if fully outside.
/// - (wx, wy): window top-left in global screen coords
/// - (ww, wh): window dimensions
/// - (lx, ly, w, h): rectangle in LOCAL window coords
fn clip_fill(
    wx: u32, wy: u32, ww: u32, wh: u32,
    lx: u32, ly: u32, w: u32, h: u32,
) -> Option<(u32, u32, u32, u32)> {
    let ax = wx + lx;
    let ay = wy + ly;
    let win_right  = wx + ww;
    let win_bottom = wy + wh;
    if ax >= win_right || ay >= win_bottom { return None; }
    let aw = w.min(win_right  - ax);
    let ah = h.min(win_bottom - ay);
    if aw == 0 || ah == 0 { return None; }
    Some((ax, ay, aw, ah))
}

/// Apply window origin offset to a draw_text position.
/// Returns None if the text origin is outside the window bounds.
fn offset_text(
    wx: u32, wy: u32, ww: u32, wh: u32,
    lx: u32, ly: u32,
) -> Option<(u32, u32)> {
    let ax = wx + lx;
    let ay = wy + ly;
    if ax >= wx + ww || ay >= wy + wh { return None; }
    Some((ax, ay))
}

// ── clip_fill tests ───────────────────────────────────────────────────────────

#[test]
fn fill_no_offset_passthrough() {
    // window at origin: local == global
    assert_eq!(clip_fill(0, 0, 1440, 440, 0, 0, 1440, 440),
               Some((0, 0, 1440, 440)));
}

#[test]
fn fill_window_y_offset_applied() {
    // shell window at y=440; local y=10 → global y=450
    assert_eq!(clip_fill(0, 440, 1440, 460, 0, 10, 100, 20),
               Some((0, 450, 100, 20)));
}

#[test]
fn fill_fully_outside_window_dropped() {
    // local y=500, window height=460 → global y=940 > 440+460=900: outside
    assert_eq!(clip_fill(0, 440, 1440, 460, 0, 500, 100, 20), None);
}

#[test]
fn fill_clipped_at_bottom_edge() {
    // rect extending past window bottom gets clamped in height
    // window: y=440, h=460 → bottom=900; local y=450, h=20 → global y=890, h clamped to 10
    assert_eq!(clip_fill(0, 440, 1440, 460, 0, 450, 100, 20),
               Some((0, 890, 100, 10)));
}

#[test]
fn fill_clipped_at_right_edge() {
    // rect extending past window right edge gets clamped in width
    // window: x=0, w=1440; local x=1430, w=20 → clamped to 10
    assert_eq!(clip_fill(0, 0, 1440, 440, 1430, 0, 20, 10),
               Some((1430, 0, 10, 10)));
}

#[test]
fn fill_zero_width_after_clip_dropped() {
    // rect exactly at the right edge produces zero width — should be dropped
    assert_eq!(clip_fill(0, 0, 100, 100, 100, 0, 10, 10), None);
}

// ── offset_text tests ─────────────────────────────────────────────────────────

#[test]
fn text_offset_applied() {
    // shell window at y=440; text at local (24, 10) → global (24, 450)
    assert_eq!(offset_text(0, 440, 1440, 460, 24, 10), Some((24, 450)));
}

#[test]
fn text_outside_window_dropped() {
    // text origin below window bottom
    assert_eq!(offset_text(0, 440, 1440, 460, 0, 461), None);
}
