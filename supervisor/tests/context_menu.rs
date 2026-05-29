// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Unit tests for the desktop right-click context menu (T063, T064).
//!
//! Uses a simulation struct that mirrors the ContextMenuState logic exactly,
//! following the same pattern as dropdown_keyboard.rs.  This keeps the tests
//! self-contained and avoids the "can't import binary crate" limitation.

// ── Minimal local context-menu simulation ────────────────────────────────────

const MENU_W: i32 = 160;
const ROW_H:  i32 = 28;
const PADDING: i32 = 8; // top/bottom padding inside panel

#[derive(Default)]
struct ContextMenuSim {
    open:  bool,
    x:     u32,
    y:     u32,
    items: Vec<(String, String)>, // (label, action)
}

impl ContextMenuSim {
    fn open(&mut self, x: u32, y: u32) {
        self.open = true;
        self.x = x;
        self.y = y;
        self.items = vec![
            ("New Note".to_string(),            "new".to_string()),
            ("Open Settings".to_string(), "spawn settings".to_string()),
        ];
    }

    fn close(&mut self) {
        self.open = false;
    }

    fn is_open(&self) -> bool { self.open }

    /// Returns Some((target_app, action)) if (cx, cy) hits an item row.
    fn hit_test(&self, cx: i32, cy: i32) -> Option<(String, String)> {
        if !self.open || self.items.is_empty() { return None; }
        let panel_h = ROW_H * self.items.len() as i32 + PADDING;
        let px = self.x as i32;
        let py = self.y as i32;
        // Outside bounding box?
        if cx < px || cx >= px + MENU_W || cy < py || cy >= py + panel_h {
            return None;
        }
        // Guard against clicks in the top PADDING/2 zone: negative offset maps to
        // row 0 via truncation-toward-zero, so we must check before dividing.
        let offset = cy - py - PADDING / 2;
        if offset < 0 { return None; }
        let row = offset / ROW_H;
        if row as usize >= self.items.len() { return None; }
        let (label, action) = &self.items[row as usize];
        // Map action to its target app
        let target_app = match action.as_str() {
            "new"            => "notes",
            "spawn settings" => "supervisor",
            _                => return None,
        };
        let _ = label;
        Some((target_app.to_string(), action.clone()))
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

// open / is_open / close ──────────────────────────────────────────────────────

#[test]
fn context_menu_starts_closed() {
    let m = ContextMenuSim::default();
    assert!(!m.is_open());
}

#[test]
fn open_sets_is_open() {
    let mut m = ContextMenuSim::default();
    m.open(100, 200);
    assert!(m.is_open());
}

#[test]
fn open_sets_position() {
    let mut m = ContextMenuSim::default();
    m.open(300, 400);
    assert_eq!(m.x, 300);
    assert_eq!(m.y, 400);
}

#[test]
fn open_populates_two_items() {
    let mut m = ContextMenuSim::default();
    m.open(0, 0);
    assert_eq!(m.items.len(), 2);
    assert_eq!(m.items[0].0, "New Note");
    assert_eq!(m.items[1].0, "Open Settings");
}

#[test]
fn close_resets_open_flag() {
    let mut m = ContextMenuSim::default();
    m.open(50, 50);
    assert!(m.is_open());
    m.close();
    assert!(!m.is_open());
}

// hit_test — inside ───────────────────────────────────────────────────────────

#[test]
fn hit_test_first_item_returns_notes_new() {
    let mut m = ContextMenuSim::default();
    m.open(100, 100);
    // First item row starts at y=100 + PADDING/2 (4) through 100+4+28=132
    let result = m.hit_test(120, 110); // well inside first row
    assert_eq!(result, Some(("notes".to_string(), "new".to_string())));
}

#[test]
fn hit_test_second_item_returns_supervisor_spawn_settings() {
    let mut m = ContextMenuSim::default();
    m.open(100, 100);
    // Second row: starts at y = 100 + 4 + 28 = 132 → 132+28=160
    let result = m.hit_test(120, 140); // inside second row
    assert_eq!(result, Some(("supervisor".to_string(), "spawn settings".to_string())));
}

// hit_test — outside ──────────────────────────────────────────────────────────

#[test]
fn hit_test_left_of_panel_returns_none() {
    let mut m = ContextMenuSim::default();
    m.open(100, 100);
    assert_eq!(m.hit_test(50, 110), None);
}

#[test]
fn hit_test_right_of_panel_returns_none() {
    let mut m = ContextMenuSim::default();
    m.open(100, 100);
    assert_eq!(m.hit_test(100 + 160 + 5, 110), None);
}

#[test]
fn hit_test_above_panel_returns_none() {
    let mut m = ContextMenuSim::default();
    m.open(100, 100);
    assert_eq!(m.hit_test(120, 99), None);
}

#[test]
fn hit_test_below_panel_returns_none() {
    let mut m = ContextMenuSim::default();
    m.open(100, 100);
    // panel_h = 28*2 + 8 = 64; bottom = 100+64=164
    assert_eq!(m.hit_test(120, 165), None);
}

#[test]
fn hit_test_when_closed_returns_none() {
    let m = ContextMenuSim::default(); // never opened
    assert_eq!(m.hit_test(0, 0), None);
}

// left-click dismiss behaviour ────────────────────────────────────────────────

#[test]
fn close_after_miss_dismisses_menu() {
    // Simulates: left-click outside menu → hit_test returns None → close
    let mut m = ContextMenuSim::default();
    m.open(100, 100);
    let result = m.hit_test(400, 400); // outside
    assert_eq!(result, None);
    m.close(); // caller closes on None
    assert!(!m.is_open());
}

#[test]
fn close_after_hit_dismisses_menu() {
    // Simulates: left-click on item → hit_test returns Some → dispatch + close
    let mut m = ContextMenuSim::default();
    m.open(100, 100);
    let result = m.hit_test(120, 110);
    assert!(result.is_some());
    m.close();
    assert!(!m.is_open());
}

// IPC message format contract ─────────────────────────────────────────────────

#[test]
fn ipc_message_format_new_note() {
    let mut m = ContextMenuSim::default();
    m.open(0, 0);
    let (app, action) = m.hit_test(10, 10).unwrap();
    let ipc = format!("@{app}: {action}");
    assert_eq!(ipc, "@notes: new");
}

#[test]
fn ipc_message_format_open_settings() {
    let mut m = ContextMenuSim::default();
    m.open(0, 0);
    // second row: y >= 0 + 4 + 28 = 32, use y=40
    let (app, action) = m.hit_test(10, 40).unwrap();
    let ipc = format!("@{app}: {action}");
    assert_eq!(ipc, "@supervisor: spawn settings");
}

// hit-test boundary conditions ────────────────────────────────────────────────

#[test]
fn hit_test_top_padding_returns_none() {
    // cy == menu_y — the very top edge of the panel is inside the PADDING/2 (4px)
    // guard zone and must return None, not hit item 0.
    let mut m = ContextMenuSim::default();
    m.open(100, 200);
    // offset = 200 - 200 - 4 = -4, which is < 0 → None
    assert_eq!(m.hit_test(120, 200), None);
}

#[test]
fn hit_test_bottom_edge_returns_second_item() {
    // cy == menu_y + PADDING/2 + ROW_H + ROW_H/2
    //     = menu_y + 4 + 28 + 14 = menu_y + 46
    // offset = 46 - 4 = 42; row = 42 / 28 = 1 → second item
    let mut m = ContextMenuSim::default();
    m.open(100, 200);
    let cy = 200 + PADDING / 2 + ROW_H + ROW_H / 2; // 200 + 4 + 28 + 14 = 246
    let result = m.hit_test(120, cy);
    assert_eq!(result, Some(("supervisor".to_string(), "spawn settings".to_string())));
}
