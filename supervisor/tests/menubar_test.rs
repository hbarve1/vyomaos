// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Unit tests for `menubar_hit_app` — menu bar click-to-focus hit detection.

use supervisor::windows::{menubar_hit_app, menubar_label_width, MENUBAR_APPS_START_X};

// ── Layout constants (mirrored from the production draw_menubar layout) ────────

/// Menu bar height used in production (MENUBAR_H = 32, scaled for 1080p).
const MENUBAR_H: u32 = 32;

// ── menubar_label_width ────────────────────────────────────────────────────────

#[test]
fn label_width_zero_len_is_padding_only() {
    // Even a zero-length name still has 8 + 8 = 16 px of padding.
    assert_eq!(menubar_label_width(0), 16);
}

#[test]
fn label_width_one_char() {
    // 1 * 8 + 16 = 24
    assert_eq!(menubar_label_width(1), 24);
}

#[test]
fn label_width_matches_expected_formula() {
    // "shell" has 5 chars → 5*8 + 16 = 56
    assert_eq!(menubar_label_width("shell".len()), 56);
}

// ── menubar_hit_app — cy outside menu bar ─────────────────────────────────────

#[test]
fn click_below_menubar_returns_none() {
    // cy == MENUBAR_H is just outside the bar.
    let apps = &["gui-demo", "shell"];
    assert_eq!(menubar_hit_app(MENUBAR_APPS_START_X, MENUBAR_H as i32, MENUBAR_H, apps), None);
}

#[test]
fn click_well_below_menubar_returns_none() {
    let apps = &["gui-demo"];
    assert_eq!(menubar_hit_app(100, 100, MENUBAR_H, apps), None);
}

#[test]
fn click_negative_cy_returns_none() {
    let apps = &["gui-demo"];
    assert_eq!(menubar_hit_app(MENUBAR_APPS_START_X, -1, MENUBAR_H, apps), None);
}

// ── menubar_hit_app — no apps ─────────────────────────────────────────────────

#[test]
fn no_apps_always_returns_none() {
    assert_eq!(menubar_hit_app(MENUBAR_APPS_START_X, 10, MENUBAR_H, &[]), None);
}

// ── menubar_hit_app — single app ──────────────────────────────────────────────

#[test]
fn single_app_left_edge_hit() {
    // First app starts at MENUBAR_APPS_START_X.
    let apps = &["gui"];
    // "gui" = 3 chars → label width = 3*8+16 = 40
    // label spans x in [88, 128)
    assert_eq!(
        menubar_hit_app(MENUBAR_APPS_START_X, 10, MENUBAR_H, apps),
        Some("gui")
    );
}

#[test]
fn single_app_right_edge_inclusive() {
    // "gui" label spans [88, 128); last pixel in label is x=127.
    let apps = &["gui"];
    let right_edge = MENUBAR_APPS_START_X + menubar_label_width("gui".len()) - 1;
    assert_eq!(
        menubar_hit_app(right_edge, 10, MENUBAR_H, apps),
        Some("gui")
    );
}

#[test]
fn single_app_one_past_right_edge_misses() {
    // x == MENUBAR_APPS_START_X + label_width is outside the label.
    let apps = &["gui"];
    let past_right = MENUBAR_APPS_START_X + menubar_label_width("gui".len());
    assert_eq!(menubar_hit_app(past_right, 10, MENUBAR_H, apps), None);
}

#[test]
fn click_before_apps_start_returns_none() {
    // cx < MENUBAR_APPS_START_X hits the "VyomaOS" brand area, not any app.
    let apps = &["shell"];
    assert_eq!(menubar_hit_app(MENUBAR_APPS_START_X - 1, 10, MENUBAR_H, apps), None);
}

// ── menubar_hit_app — multiple apps ───────────────────────────────────────────

#[test]
fn two_apps_hit_first() {
    let apps = &["ping", "pong"];
    // "ping" = 4 chars → label [88, 88+4*8+16) = [88, 136)
    assert_eq!(menubar_hit_app(88, 5, MENUBAR_H, apps), Some("ping"));
    assert_eq!(menubar_hit_app(135, 5, MENUBAR_H, apps), Some("ping"));
}

#[test]
fn two_apps_hit_second() {
    let apps = &["ping", "pong"];
    // "ping" label = [88, 136); "pong" label = [136, 184)
    let pong_start = MENUBAR_APPS_START_X + menubar_label_width("ping".len());
    assert_eq!(menubar_hit_app(pong_start, 5, MENUBAR_H, apps), Some("pong"));
    let pong_end = pong_start + menubar_label_width("pong".len()) - 1;
    assert_eq!(menubar_hit_app(pong_end, 5, MENUBAR_H, apps), Some("pong"));
}

#[test]
fn gap_between_labels_handled_correctly() {
    // Labels are contiguous (no gap between them) — the right edge of one app's
    // label is the left edge of the next.  Verify the boundary pixel.
    let apps = &["a", "bb"];
    // "a" label: [88, 88+1*8+16) = [88, 112)
    // "bb" label: [112, 112+2*8+16) = [112, 144)
    let boundary = MENUBAR_APPS_START_X + menubar_label_width(1); // = 112
    assert_eq!(menubar_hit_app(boundary - 1, 5, MENUBAR_H, apps), Some("a"));
    assert_eq!(menubar_hit_app(boundary, 5, MENUBAR_H, apps), Some("bb"));
}

#[test]
fn three_apps_last_one_hit() {
    let apps = &["a", "b", "shell"];
    // "a"     label: [88, 88+1*8+16) = [88, 112)
    // "b"     label: [112, 112+1*8+16) = [112, 136)
    // "shell" label: [136, 136+5*8+16) = [136, 192)
    let shell_start = MENUBAR_APPS_START_X
        + menubar_label_width(1)   // "a"
        + menubar_label_width(1);  // "b"
    assert_eq!(menubar_hit_app(shell_start, 12, MENUBAR_H, apps), Some("shell"));
    let shell_end = shell_start + menubar_label_width("shell".len()) - 1;
    assert_eq!(menubar_hit_app(shell_end, 12, MENUBAR_H, apps), Some("shell"));
    // One past the last label → None
    let past_all = shell_start + menubar_label_width("shell".len());
    assert_eq!(menubar_hit_app(past_all, 12, MENUBAR_H, apps), None);
}

// ── menubar_hit_app — cy boundary ─────────────────────────────────────────────

#[test]
fn cy_zero_is_in_menubar() {
    let apps = &["gui-demo"];
    assert!(menubar_hit_app(MENUBAR_APPS_START_X, 0, MENUBAR_H, apps).is_some());
}

#[test]
fn cy_one_below_menubar_top_is_in_bar() {
    let apps = &["gui-demo"];
    assert!(menubar_hit_app(MENUBAR_APPS_START_X, MENUBAR_H as i32 - 1, MENUBAR_H, apps).is_some());
}

#[test]
fn cy_exactly_menubar_h_is_outside() {
    let apps = &["gui-demo"];
    assert_eq!(
        menubar_hit_app(MENUBAR_APPS_START_X, MENUBAR_H as i32, MENUBAR_H, apps),
        None
    );
}
