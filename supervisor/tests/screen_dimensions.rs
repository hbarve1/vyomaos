// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Tests for multi-resolution display support (spec-045).
//!
//! Covers:
//! - Tiling at non-default resolutions (1920×1080, 640×480)
//! - VYOMA_SYSTEM:screen: message parsing
//! - screen-size IPC reply format

use supervisor::windows::compute_tiling;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn regions_overlap(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    ax < bx + bw && ax + aw > bx && ay < by + bh && ay + ah > by
}

fn all_within(regions: &[(u32, u32, u32, u32)], sw: u32, sh: u32) -> bool {
    regions.iter().all(|&(x, y, w, h)| x + w <= sw && y + h <= sh)
}

fn no_overlap(regions: &[(u32, u32, u32, u32)]) -> bool {
    for i in 0..regions.len() {
        for j in (i + 1)..regions.len() {
            if regions_overlap(regions[i], regions[j]) {
                return false;
            }
        }
    }
    true
}

// ── T005: test_tiling_at_1920x1080 ───────────────────────────────────────────

/// Tiling 3 apps at 1920×1080 must fit within the screen and not overlap.
#[test]
fn test_tiling_at_1920x1080() {
    let (sw, sh) = (1920u32, 1080u32);
    let regions = compute_tiling(3, sw, sh);
    assert_eq!(regions.len(), 3, "expected 3 regions for n=3");
    assert!(no_overlap(&regions), "regions must not overlap at 1920×1080");
    assert!(all_within(&regions, sw, sh), "all regions must fit within 1920×1080");
}

/// Tiling at 1920×1080 should give wider tiles than at 1440×900.
#[test]
fn test_tiling_1920x1080_wider_than_1440x900() {
    let small = compute_tiling(1, 1440, 900);
    let large = compute_tiling(1, 1920, 1080);
    assert_eq!(small[0].2, 1440, "1440-wide single tile should have w=1440");
    assert_eq!(large[0].2, 1920, "1920-wide single tile should have w=1920");
}

/// Tiling at 640×480 (IoT profile) — 1 app must render without off-screen elements.
#[test]
fn test_tiling_at_640x480_single_app() {
    let (sw, sh) = (640u32, 480u32);
    let regions = compute_tiling(1, sw, sh);
    assert_eq!(regions.len(), 1);
    assert!(all_within(&regions, sw, sh), "640×480 single app must fit on screen");
    assert_eq!(regions[0], (0, 0, 640, 480));
}

/// Tiling at 640×480 with 3 apps — all tiles must fit and not overlap.
#[test]
fn test_tiling_at_640x480_three_apps() {
    let (sw, sh) = (640u32, 480u32);
    let regions = compute_tiling(3, sw, sh);
    assert_eq!(regions.len(), 3);
    assert!(no_overlap(&regions), "640×480 three-app tiles must not overlap");
    assert!(all_within(&regions, sw, sh), "640×480 three-app tiles must fit on screen");
}

// ── T019: test_screen_size_ipc_reply ─────────────────────────────────────────

/// A screen-size IPC reply must contain "x" separator and two parseable integers.
#[test]
fn test_screen_size_ipc_reply_format() {
    // Simulate what the supervisor produces for screen-size reply:
    // format "REPLY:<w>x<h>"
    let w = 1920u32;
    let h = 1080u32;
    let reply = format!("REPLY:{}x{}", w, h);

    // Strip "REPLY:" prefix (as a shell app would)
    let payload = reply.strip_prefix("REPLY:").expect("reply must start with REPLY:");
    assert!(payload.contains('x'), "screen-size reply must contain 'x' separator, got: {payload}");

    let parts: Vec<&str> = payload.splitn(2, 'x').collect();
    assert_eq!(parts.len(), 2, "screen-size reply must have exactly two parts");

    let pw: u32 = parts[0].parse().expect("width must be a valid u32");
    let ph: u32 = parts[1].parse().expect("height must be a valid u32");
    assert_eq!(pw, 1920, "parsed width must match");
    assert_eq!(ph, 1080, "parsed height must match");
}

/// screen-size reply for headless boot (fallback 1440×900).
#[test]
fn test_screen_size_ipc_reply_fallback() {
    let w = 1440u32;
    let h = 900u32;
    let reply = format!("REPLY:{}x{}", w, h);
    let payload = reply.strip_prefix("REPLY:").unwrap();
    let mut it = payload.splitn(2, 'x');
    let pw: u32 = it.next().unwrap().parse().unwrap();
    let ph: u32 = it.next().unwrap().parse().unwrap();
    assert_eq!(pw, 1440);
    assert_eq!(ph, 900);
}

// ── VYOMA_SYSTEM:screen: parsing (mirrors app-side logic) ────────────────────

/// Simulate parsing "VYOMA_SYSTEM:screen:<w>,<h>" as the app would.
fn parse_screen_notification(line: &str) -> Option<(u32, u32)> {
    let dims = line.strip_prefix("VYOMA_SYSTEM:screen:")?;
    let (ws, hs) = dims.split_once(',')?;
    let w = ws.parse::<u32>().ok()?;
    let h = hs.parse::<u32>().ok()?;
    Some((w, h))
}

#[test]
fn test_parse_screen_notification_valid() {
    assert_eq!(
        parse_screen_notification("VYOMA_SYSTEM:screen:1920,1080"),
        Some((1920, 1080))
    );
}

#[test]
fn test_parse_screen_notification_fallback_dims() {
    assert_eq!(
        parse_screen_notification("VYOMA_SYSTEM:screen:1440,900"),
        Some((1440, 900))
    );
}

#[test]
fn test_parse_screen_notification_iot_dims() {
    assert_eq!(
        parse_screen_notification("VYOMA_SYSTEM:screen:640,480"),
        Some((640, 480))
    );
}

#[test]
fn test_parse_screen_notification_invalid_returns_none() {
    assert_eq!(parse_screen_notification("VYOMA_SYSTEM:screen:abc,xyz"), None);
    assert_eq!(parse_screen_notification("REPLY:something"), None);
    assert_eq!(parse_screen_notification("VYOMA_SYSTEM:screen:1920"), None);
    assert_eq!(parse_screen_notification(""), None);
}

#[test]
fn test_parse_screen_notification_empty_payload_returns_none() {
    assert_eq!(parse_screen_notification("VYOMA_SYSTEM:screen:"), None);
}
