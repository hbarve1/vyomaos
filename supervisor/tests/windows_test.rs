// Unit tests for supervisor/src/windows.rs — window layout, hit detection, snap layout.

use supervisor::windows::{
    clamp_tile_size, compute_tiling, compute_tiling_with_hints,
    compute_snap_layout, drag_delta, nearest_tiled_slot,
    menubar_hit_app, menubar_label_width,
    MIN_WIN_W, MIN_WIN_H,
};

// ── clamp_tile_size ─────────────────────────────────────────────────────────

#[test]
fn test_clamp_tile_size_no_change() {
    assert_eq!(clamp_tile_size(400, 300, 200, 100), (400, 300));
}

#[test]
fn test_clamp_tile_size_both_clamped() {
    assert_eq!(clamp_tile_size(50, 30, 200, 100), (200, 100));
}

#[test]
fn test_clamp_tile_size_only_width() {
    assert_eq!(clamp_tile_size(100, 300, 200, 100), (200, 300));
}

#[test]
fn test_clamp_tile_size_only_height() {
    assert_eq!(clamp_tile_size(400, 50, 200, 100), (400, 100));
}

// ── compute_tiling edge cases ───────────────────────────────────────────────

#[test]
fn test_tiling_zero_screen_w() {
    assert!(compute_tiling(1, 0, 768).is_empty());
}

#[test]
fn test_tiling_zero_screen_h() {
    assert!(compute_tiling(1, 1024, 0).is_empty());
}

#[test]
fn test_tiling_capped_at_9() {
    let r = compute_tiling(20, 1024, 768);
    assert_eq!(r.len(), 9);
}

#[test]
fn test_tiling_3_apps() {
    let r = compute_tiling(3, 1024, 768);
    assert_eq!(r.len(), 3);
    // 3 apps: cols=2, rows=2, top row has 2 tiles, bottom row has 1 (expanded)
    // All y+h should not exceed screen height
    for &(x, y, w, h) in &r {
        assert!(x + w <= 1024 || w == MIN_WIN_W, "x+w exceeds screen: {}+{}", x, w);
        assert!(y + h <= 768 || h == MIN_WIN_H, "y+h exceeds screen: {}+{}", y, h);
    }
}

#[test]
fn test_tiling_4_apps_grid() {
    let r = compute_tiling(4, 1024, 768);
    assert_eq!(r.len(), 4);
    // 4 apps: 2x2 grid
    assert_eq!(r[0], (0, 0, 512, 384));
    assert_eq!(r[1], (512, 0, 512, 384));
    assert_eq!(r[2], (0, 384, 512, 384));
    assert_eq!(r[3], (512, 384, 512, 384));
}

// ── compute_tiling_with_hints ───────────────────────────────────────────────

#[test]
fn test_tiling_with_hints_empty_hints() {
    let a = compute_tiling(2, 800, 600);
    let b = compute_tiling_with_hints(2, 800, 600, &[]);
    assert_eq!(a, b);
}

#[test]
fn test_tiling_with_hints_full_row_request() {
    // With 3 apps: cols=2, rows=2. Top row has 2 apps, bottom has 1.
    // First app wants full row (min_w=900 > base_cell_w=400).
    // But only non-last-row apps can get full row; top row is not the last row.
    let r = compute_tiling_with_hints(3, 800, 600, &[(900, 100), (200, 100), (200, 100)]);
    assert_eq!(r.len(), 3);
    // First app (row 0, not last row) should get full row width
    assert_eq!(r[0].2, 800); // w = screen_w
}

// ── drag_delta ──────────────────────────────────────────────────────────────

#[test]
fn test_drag_delta_positive() {
    assert_eq!(drag_delta(10, 20, 50, 80), (40, 60));
}

#[test]
fn test_drag_delta_negative() {
    assert_eq!(drag_delta(100, 200, 50, 80), (-50, -120));
}

#[test]
fn test_drag_delta_zero() {
    assert_eq!(drag_delta(50, 50, 50, 50), (0, 0));
}

// ── nearest_tiled_slot ──────────────────────────────────────────────────────

#[test]
fn test_nearest_slot_within_threshold() {
    // Single app on 1440x900 with menubar=24 -> slot at (0, 24)
    let result = nearest_tiled_slot((10, 30), 1, 1440, 900, 24);
    assert!(result.is_some());
}

#[test]
fn test_nearest_slot_outside_threshold() {
    let result = nearest_tiled_slot((500, 500), 1, 1440, 900, 24);
    assert!(result.is_none());
}

#[test]
fn test_nearest_slot_zero_apps() {
    assert_eq!(nearest_tiled_slot((0, 0), 0, 1440, 900, 24), None);
}

#[test]
fn test_nearest_slot_exact_match() {
    // 1 app: slot at (0, 24, 1440, 876)
    let result = nearest_tiled_slot((0, 24), 1, 1440, 900, 24);
    assert_eq!(result, Some((0, 24, 1440, 876)));
}

// ── menubar_label_width ─────────────────────────────────────────────────────

#[test]
fn test_menubar_label_width_short() {
    // "ab" = 2 chars -> 2*8 + 16 = 32
    assert_eq!(menubar_label_width(2), 32);
}

#[test]
fn test_menubar_label_width_typical() {
    // "calculator" = 10 chars -> 10*8 + 16 = 96
    assert_eq!(menubar_label_width(10), 96);
}

// ── menubar_hit_app ─────────────────────────────────────────────────────────

#[test]
fn test_menubar_hit_below_bar() {
    let apps = &["calc", "shell"];
    // Click below menu bar (cy >= menubar_h)
    assert_eq!(menubar_hit_app(100, 24, 24, apps), None);
}

#[test]
fn test_menubar_hit_above_bar() {
    let apps = &["calc"];
    // Negative y
    assert_eq!(menubar_hit_app(100, -1, 24, apps), None);
}

#[test]
fn test_menubar_hit_first_app() {
    let apps = &["calc", "shell"];
    // "calc" starts at x=88, width = 4*8+16 = 48, ends at 136
    let result = menubar_hit_app(90, 10, 24, apps);
    assert_eq!(result, Some("calc"));
}

#[test]
fn test_menubar_hit_second_app() {
    let apps = &["calc", "shell"];
    // "calc" ends at 88+48=136. "shell" starts at 136, width=5*8+16=56, ends at 192
    let result = menubar_hit_app(140, 10, 24, apps);
    assert_eq!(result, Some("shell"));
}

#[test]
fn test_menubar_hit_miss_right() {
    let apps = &["calc"];
    // "calc" ends at 88+48=136. Click at 200 is past it.
    let result = menubar_hit_app(200, 10, 24, apps);
    assert_eq!(result, None);
}

#[test]
fn test_menubar_hit_empty_apps() {
    let apps: &[&str] = &[];
    assert_eq!(menubar_hit_app(90, 10, 24, apps), None);
}

// ── compute_snap_layout ─────────────────────────────────────────────────────

#[test]
fn test_snap_layout_zero_apps() {
    assert!(compute_snap_layout(1440, 900, 24, 0, 0).is_empty());
}

#[test]
fn test_snap_layout_single_app() {
    let r = compute_snap_layout(1440, 900, 24, 0, 1);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0], (0, 24, 1440, 876));
}

#[test]
fn test_snap_layout_focused_gets_two_thirds() {
    let r = compute_snap_layout(1440, 900, 24, 0, 3);
    assert_eq!(r.len(), 3);
    // Focused (idx=0) gets 2/3 of screen width on the left
    let left_w = 1440 * 2 / 3;
    assert_eq!(r[0], (0, 24, left_w, 876));
    // Others stacked on the right
    assert_eq!(r[1].0, left_w);
    assert_eq!(r[2].0, left_w);
}

#[test]
fn test_snap_layout_focused_middle() {
    let r = compute_snap_layout(1440, 900, 24, 1, 3);
    assert_eq!(r.len(), 3);
    // Index 1 is focused -> gets left 2/3
    let left_w = 1440 * 2 / 3;
    assert_eq!(r[1], (0, 24, left_w, 876));
    // Index 0 and 2 on the right
    assert_eq!(r[0].0, left_w);
    assert_eq!(r[2].0, left_w);
}
