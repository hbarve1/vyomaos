// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Desktop right-click context menu (T063, T064).
//!
//! Provides an 160-px-wide floating panel with hardcoded desktop actions.
//! Opening, hit-testing, and closing are all pure state mutations so that
//! tests can drive the logic without a framebuffer.

use std::sync::Mutex;

#[cfg(target_os = "linux")]
use crate::display;

// Panel visual constants
const MENU_W:   u32 = 160;
const ROW_H:    u32 = 28;
const PADDING:  u32 = 8; // total top+bottom padding inside panel

// ── State ─────────────────────────────────────────────────────────────────────

struct ContextMenuState {
    open:  bool,
    x:     u32,
    y:     u32,
    items: Vec<(String, String)>, // (label, action)
    #[allow(dead_code)]
    selected: usize,
}

static CTX_MENU: std::sync::OnceLock<Mutex<ContextMenuState>> = std::sync::OnceLock::new();

fn ctx_state() -> &'static Mutex<ContextMenuState> {
    CTX_MENU.get_or_init(|| Mutex::new(ContextMenuState {
        open: false,
        x: 0, y: 0,
        items: vec![],
        selected: 0,
    }))
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Open the context menu at screen position (x, y) with the hardcoded desktop items.
pub fn open_context_menu(x: u32, y: u32) {
    let mut s = ctx_state().lock().unwrap();
    s.open = true;
    s.x = x;
    s.y = y;
    s.selected = 0;
    s.items = vec![
        ("New Note".to_string(),            "new".to_string()),
        ("Open Settings".to_string(), "spawn settings".to_string()),
    ];
}

/// Close the context menu without dispatching any action.
pub fn close_context_menu() {
    ctx_state().lock().unwrap().open = false;
}

/// Return true when the context menu is currently visible.
pub fn context_menu_is_open() -> bool {
    ctx_state().lock().unwrap().open
}

/// Hit-test (cx, cy) against the open context menu.
///
/// Returns `Some((target_app, action))` when the click lands on an item row:
///   - item 0 ("New Note",      "new")            → target = "notes"
///   - item 1 ("Open Settings", "spawn settings") → target = "supervisor"
///
/// Returns `None` when the click is outside the panel, or when the menu is closed.
pub fn context_menu_hit_test(cx: i32, cy: i32) -> Option<(String, String)> {
    let s = ctx_state().lock().unwrap();
    if !s.open || s.items.is_empty() { return None; }
    let panel_h = ROW_H * s.items.len() as u32 + PADDING;
    let px = s.x as i32;
    let py = s.y as i32;
    // Outside bounding box?
    if cx < px || cx >= px + MENU_W as i32
        || cy < py || cy >= py + panel_h as i32
    {
        return None;
    }
    // Guard against clicks in the top PADDING/2 zone: negative offset maps to
    // row 0 via truncation-toward-zero, so we must check before dividing.
    let offset = cy - py - (PADDING / 2) as i32;
    if offset < 0 { return None; }
    let row = offset / ROW_H as i32;
    if row as usize >= s.items.len() { return None; }
    let (_label, action) = &s.items[row as usize];
    let target_app = match action.as_str() {
        "new"            => "notes",
        "spawn settings" => "supervisor",
        _                => return None,
    };
    Some((target_app.to_string(), action.clone()))
}

/// Draw the context menu panel onto the framebuffer back-buffer if open.
/// Called from the compositor flush path after chrome has been drawn.
#[cfg(target_os = "linux")]
pub fn render_context_menu_if_open(fb: &mut display::Framebuffer) {
    let s = ctx_state().lock().unwrap();
    if !s.open || s.items.is_empty() { return; }
    let (px, py, items) = (s.x, s.y, s.items.clone());
    drop(s);

    use crate::display::draw_rounded_rect;
    let (fs, fw, fh) = (fb.stride, fb.width, fb.height);
    let panel_h = ROW_H * items.len() as u32 + PADDING;

    // Panel background: dark, 85% alpha, radius=8
    draw_rounded_rect(&mut fb.back, px, py, MENU_W, panel_h, 0x1C1C1EE5_u32, 8, fs, fw, fh);

    // Item rows
    for (i, (label, _action)) in items.iter().enumerate() {
        let ry = py + PADDING / 2 + i as u32 * ROW_H;
        // Vertically center text in the row: baseline at mid of ROW_H
        let tx = (px + 12) as i32;
        let ty = (ry + ROW_H / 2) as i32;
        crate::chrome::draw_glyph_str_pub(fb, label, tx, ty, 0xFFFFFFFF, 13, false, false);
    }
}
