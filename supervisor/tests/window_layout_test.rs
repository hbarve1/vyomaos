// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use supervisor::windows::{compute_tiling, compute_tiling_with_hints, compute_snap_layout};

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

// ── compute_snap_layout ───────────────────────────────────────────────────────

const MENUBAR: u32 = 24;

/// 1 app total → full screen minus menu bar.
#[test]
fn test_snap_single_app_full_screen() {
    let r = compute_snap_layout(W, H, MENUBAR, 0, 1);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0], (0, MENUBAR, W, H - MENUBAR));
}

/// 2 apps, focused=0 → focused gets left 2/3, other gets right 1/3.
#[test]
fn test_snap_two_apps_focused_first() {
    let r = compute_snap_layout(W, H, MENUBAR, 0, 2);
    assert_eq!(r.len(), 2);
    let left_w   = W * 2 / 3;
    let right_x  = left_w;
    let right_w  = W - left_w;
    let usable_h = H - MENUBAR;
    // Focused (index 0): left column, full usable height.
    assert_eq!(r[0], (0, MENUBAR, left_w, usable_h), "focused tile mismatch");
    // Other (index 1): right column.
    assert_eq!(r[1], (right_x, MENUBAR, right_w, usable_h), "other tile mismatch");
    // No gaps at right edge.
    assert_eq!(r[0].2 + r[1].2, W, "widths do not sum to screen width");
}

/// 3 apps, focused=1 → focused gets left 2/3, other two split right 1/3 vertically.
#[test]
fn test_snap_three_apps_focused_middle() {
    let r = compute_snap_layout(W, H, MENUBAR, 1, 3);
    assert_eq!(r.len(), 3);
    let left_w   = W * 2 / 3;
    let right_x  = left_w;
    let right_w  = W - left_w;
    let usable_h = H - MENUBAR;
    let slot_h   = usable_h / 2;

    // App 0 (other): first right-column slot.
    assert_eq!(r[0].0, right_x, "r[0].x");
    assert_eq!(r[0].1, MENUBAR, "r[0].y");
    assert_eq!(r[0].2, right_w, "r[0].w");
    assert_eq!(r[0].3, slot_h,  "r[0].h");

    // App 1 (focused): full left 2/3.
    assert_eq!(r[1], (0, MENUBAR, left_w, usable_h), "focused tile mismatch");

    // App 2 (other): second right-column slot, absorbs remainder.
    assert_eq!(r[2].0, right_x,           "r[2].x");
    assert_eq!(r[2].1, MENUBAR + slot_h,  "r[2].y");
    assert_eq!(r[2].2, right_w,           "r[2].w");
    assert_eq!(r[2].3, usable_h - slot_h, "r[2].h (remainder)");

    // Combined height of the two right slots equals usable_h.
    assert_eq!(r[0].3 + r[2].3, usable_h, "right-column heights must sum to usable_h");
}
