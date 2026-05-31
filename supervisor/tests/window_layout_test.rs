// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use supervisor::windows::{clamp_tile_size, compute_tiling, compute_tiling_with_hints, compute_snap_layout, MIN_WIN_H, MIN_WIN_W};

const W: u32 = 1024;
const H: u32 = 768;

fn total_area(regions: &[(u32, u32, u32, u32)]) -> u32 {
    regions.iter().map(|&(_, _, w, h)| w * h).sum()
}

fn regions_overlap(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    ax < bx + bw && ax + aw > bx && ay < by + bh && ay + ah > by
}

#[test]
fn test_zero_returns_empty() {
    assert!(compute_tiling(0, W, H).is_empty());
}

#[test]
fn test_1_fullscreen() {
    let r = compute_tiling(1, W, H);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0], (0, 0, W, H));
}

#[test]
fn test_2_side_by_side() {
    let r = compute_tiling(2, W, H);
    assert_eq!(r.len(), 2);
    // Two equal halves horizontally
    assert_eq!(r[0], (0, 0, W / 2, H));
    assert_eq!(r[1].1, 0);    // y=0
    assert_eq!(r[1].3, H);    // h=full height
    // No overlap
    assert!(!regions_overlap(r[0], r[1]));
    // Full coverage
    assert_eq!(total_area(&r), W * H);
}

#[test]
fn test_4_apps_2x2() {
    let r = compute_tiling(4, W, H);
    assert_eq!(r.len(), 4);
    // No overlap
    for i in 0..4 {
        for j in (i + 1)..4 {
            assert!(!regions_overlap(r[i], r[j]), "regions {i} and {j} overlap");
        }
    }
    // Full coverage
    assert_eq!(total_area(&r), W * H);
    // 2×2 grid: top-left at (0,0)
    assert_eq!(r[0], (0, 0, W / 2, H / 2));
}

#[test]
fn test_3_apps_last_row_expands() {
    let r = compute_tiling(3, W, H);
    assert_eq!(r.len(), 3);
    // cols=2, rows=2; last row has 1 app → spans full width
    assert_eq!(r[2].0, 0);   // x=0
    assert_eq!(r[2].2, W);   // w=full width
    // No overlap
    for i in 0..3 {
        for j in (i + 1)..3 {
            assert!(!regions_overlap(r[i], r[j]), "regions {i} and {j} overlap");
        }
    }
    assert_eq!(total_area(&r), W * H);
}

#[test]
fn test_no_overlap_1_to_9() {
    for n in 1..=9 {
        let r = compute_tiling(n, W, H);
        assert_eq!(r.len(), n, "n={n}");
        for i in 0..n {
            for j in (i + 1)..n {
                assert!(
                    !regions_overlap(r[i], r[j]),
                    "regions {i} and {j} overlap for n={n}"
                );
            }
        }
    }
}

#[test]
fn test_full_coverage_1_to_9() {
    for n in 1..=9 {
        let r = compute_tiling(n, W, H);
        assert_eq!(
            total_area(&r),
            W * H,
            "coverage mismatch for n={n}: got {} expected {}",
            total_area(&r),
            W * H
        );
    }
}

#[test]
fn test_9_apps_3x3() {
    let r = compute_tiling(9, W, H);
    assert_eq!(r.len(), 9);
    assert_eq!(total_area(&r), W * H);
    // 3×3: first tile at (0,0)
    assert_eq!(r[0].0, 0);
    assert_eq!(r[0].1, 0);
}

#[test]
fn test_beyond_9_clamps() {
    let r = compute_tiling(12, W, H);
    assert_eq!(r.len(), 9);
}

#[test]
fn test_all_regions_within_screen() {
    for n in 1..=9 {
        let r = compute_tiling(n, W, H);
        for &(x, y, w, h) in &r {
            assert!(x + w <= W, "n={n}: region right edge {}", x + w);
            assert!(y + h <= H, "n={n}: region bottom edge {}", y + h);
        }
    }
}

// ── compute_tiling_with_hints ─────────────────────────────────────────────────

#[test]
fn test_hints_4_apps_produces_4_regions() {
    let hints = vec![(0u32, 0u32); 4];
    let r = compute_tiling_with_hints(4, W, H, &hints);
    assert_eq!(r.len(), 4);
}

#[test]
fn test_hints_zero_min_matches_no_hint() {
    // All-zero hints should give the same result as plain compute_tiling
    for n in 1..=9 {
        let hints = vec![(0u32, 0u32); n];
        let with_hints = compute_tiling_with_hints(n, W, H, &hints);
        let plain      = compute_tiling(n, W, H);
        assert_eq!(with_hints, plain, "n={n}: zero hints should match plain tiling");
    }
}

#[test]
fn test_hints_no_overlap_4_apps() {
    let hints = vec![(100u32, 80u32); 4];
    let r = compute_tiling_with_hints(4, W, H, &hints);
    for i in 0..r.len() {
        for j in (i + 1)..r.len() {
            assert!(!regions_overlap(r[i], r[j]), "regions {i} and {j} overlap");
        }
    }
}

#[test]
fn test_hints_all_within_screen_4_apps() {
    let hints = vec![(200u32, 150u32); 4];
    let r = compute_tiling_with_hints(4, W, H, &hints);
    for &(x, y, w, h) in &r {
        assert!(x + w <= W, "region right edge {} exceeds screen", x + w);
        assert!(y + h <= H, "region bottom edge {} exceeds screen", y + h);
    }
}

// ── clamp_tile_size unit tests ────────────────────────────────────────────────

#[test]
fn test_clamp_tile_size_already_large_passes_through() {
    let (w, h) = clamp_tile_size(800, 600, MIN_WIN_W, MIN_WIN_H);
    assert_eq!(w, 800);
    assert_eq!(h, 600);
}

#[test]
fn test_clamp_tile_size_below_min_width_clamped() {
    let (w, h) = clamp_tile_size(50, 300, MIN_WIN_W, MIN_WIN_H);
    assert_eq!(w, MIN_WIN_W);
    assert_eq!(h, 300);
}

#[test]
fn test_clamp_tile_size_below_min_height_clamped() {
    let (w, h) = clamp_tile_size(400, 20, MIN_WIN_W, MIN_WIN_H);
    assert_eq!(w, 400);
    assert_eq!(h, MIN_WIN_H);
}

#[test]
fn test_clamp_tile_size_both_below_min_clamped() {
    let (w, h) = clamp_tile_size(10, 5, MIN_WIN_W, MIN_WIN_H);
    assert_eq!(w, MIN_WIN_W);
    assert_eq!(h, MIN_WIN_H);
}

#[test]
fn test_clamp_tile_size_exactly_at_min_passes_through() {
    let (w, h) = clamp_tile_size(MIN_WIN_W, MIN_WIN_H, MIN_WIN_W, MIN_WIN_H);
    assert_eq!(w, MIN_WIN_W);
    assert_eq!(h, MIN_WIN_H);
}

#[test]
fn test_compute_tiling_respects_min_size_for_many_apps() {
    let r = compute_tiling(9, W, H);
    assert_eq!(r.len(), 9);
    for &(_, _, w, h) in &r {
        assert!(w >= MIN_WIN_W, "tile width {w} is below MIN_WIN_W={MIN_WIN_W}");
        assert!(h >= MIN_WIN_H, "tile height {h} is below MIN_WIN_H={MIN_WIN_H}");
    }
}

// ── compute_snap_layout ───────────────────────────────────────────────────────

const MENUBAR: u32 = 24;

#[test]
fn test_snap_single_app_full_screen() {
    let r = compute_snap_layout(W, H, MENUBAR, 0, 1);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0], (0, MENUBAR, W, H - MENUBAR));
}

#[test]
fn test_snap_two_apps_focused_first() {
    let r = compute_snap_layout(W, H, MENUBAR, 0, 2);
    assert_eq!(r.len(), 2);
    let left_w   = W * 2 / 3;
    let right_x  = left_w;
    let right_w  = W - left_w;
    let usable_h = H - MENUBAR;
    assert_eq!(r[0], (0, MENUBAR, left_w, usable_h), "focused tile mismatch");
    assert_eq!(r[1], (right_x, MENUBAR, right_w, usable_h), "other tile mismatch");
    assert_eq!(r[0].2 + r[1].2, W, "widths do not sum to screen width");
}

#[test]
fn test_snap_three_apps_focused_middle() {
    let r = compute_snap_layout(W, H, MENUBAR, 1, 3);
    assert_eq!(r.len(), 3);
    let left_w   = W * 2 / 3;
    let right_x  = left_w;
    let right_w  = W - left_w;
    let usable_h = H - MENUBAR;
    let slot_h   = usable_h / 2;
    assert_eq!(r[0].0, right_x, "r[0].x");
    assert_eq!(r[0].1, MENUBAR, "r[0].y");
    assert_eq!(r[0].2, right_w, "r[0].w");
    assert_eq!(r[0].3, slot_h,  "r[0].h");
    assert_eq!(r[1], (0, MENUBAR, left_w, usable_h), "focused tile mismatch");
    assert_eq!(r[2].0, right_x,           "r[2].x");
    assert_eq!(r[2].1, MENUBAR + slot_h,  "r[2].y");
    assert_eq!(r[2].2, right_w,           "r[2].w");
    assert_eq!(r[2].3, usable_h - slot_h, "r[2].h (remainder)");
    assert_eq!(r[0].3 + r[2].3, usable_h, "right-column heights must sum to usable_h");
}

// ── Dock-aware tiling tests ───────────────────────────────────────────────────

#[test]
fn test_dock_excluded_from_tile_pool() {
    // Pure math test: 1 tiled app should fill the height between menubar and dock strip.
    // The supervisor's apply_tiling_layout is tested via smoke test; this validates math.
    use supervisor::windows::compute_tiling_with_hints;
    const MENUBAR_H: u32 = 32;
    const DOCK_STRIP_H: u32 = 72;
    let sw = 1920u32;
    let sh = 1080u32;

    // Only 1 tiled app (desktop); dock excluded before compute_tiling call.
    let usable_h = sh - MENUBAR_H - DOCK_STRIP_H;
    let regions = compute_tiling_with_hints(1, sw, usable_h, &[]);
    assert_eq!(regions.len(), 1);
    let (_x, y, w, h) = regions[0];
    let y_mapped = y + MENUBAR_H;
    assert_eq!(y_mapped, MENUBAR_H);
    assert_eq!(w, sw);
    assert_eq!(h, usable_h, "desktop should fill height between menubar and dock");

    // Dock strip region (computed separately in apply_tiling_layout)
    let dock_region = (0u32, sh - DOCK_STRIP_H, sw, DOCK_STRIP_H);
    assert_eq!(dock_region.1, 1008, "dock top y should be sh - DOCK_STRIP_H");
    assert_eq!(dock_region.1 + dock_region.3, sh, "dock must reach screen bottom");
}

#[test]
fn test_two_regular_apps_with_dock() {
    use supervisor::windows::compute_tiling_with_hints;
    const MENUBAR_H: u32 = 32;
    const DOCK_STRIP_H: u32 = 72;
    let sw = 1920u32;
    let sh = 1080u32;
    let usable_h = sh - MENUBAR_H - DOCK_STRIP_H;

    // 2 tiled apps should not extend into the dock strip
    let regions = compute_tiling_with_hints(2, sw, usable_h, &[]);
    assert_eq!(regions.len(), 2);
    for (_, y, _, h) in &regions {
        let bottom = y + MENUBAR_H + h;
        assert!(
            bottom <= sh - DOCK_STRIP_H,
            "tiled app bottom {bottom} must not reach dock at {}", sh - DOCK_STRIP_H
        );
    }
}
