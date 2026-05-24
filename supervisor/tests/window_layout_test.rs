// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use supervisor::windows::compute_tiling;

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
