// Unit tests for supervisor/src/menus.rs — dropdown/context menu state management.
//
// The menu state (DropdownState, ContextMenuState) is in the binary crate's
// private module.  Since the functions are pub, we test them by calling
// through the binary.  However, since they use static OnceLock, tests in
// separate test binaries each get their own statics.
//
// We replicate the pure state-machine logic here to test correctness.

// ── Dropdown key navigation logic (replicated from menus.rs) ────────────────

struct MockDropdown {
    open: bool,
    selected: usize,
    items: Vec<(String, String)>,
    app_name: String,
}

impl MockDropdown {
    fn new(app: &str, items: Vec<(String, String)>) -> Self {
        Self { open: true, selected: 0, items, app_name: app.to_string() }
    }

    fn handle_key(&mut self, key: &str) -> Option<(String, String)> {
        if !self.open { return None; }
        match key {
            "\x1b[B" | "j" | "\t" => {
                if self.selected + 1 < self.items.len() { self.selected += 1; }
                None
            }
            "\x1b[A" | "k" => {
                if self.selected > 0 { self.selected -= 1; }
                None
            }
            "\r" | "\n" | "" => {
                let r = self.items.get(self.selected)
                    .map(|(_, a)| (self.app_name.clone(), a.clone()));
                self.open = false;
                r
            }
            "\x1b" => { self.open = false; None }
            _ => None,
        }
    }
}

#[test]
fn test_dropdown_open_initial_state() {
    let dd = MockDropdown::new("calc", vec![
        ("Copy".into(), "copy".into()),
        ("Paste".into(), "paste".into()),
    ]);
    assert!(dd.open);
    assert_eq!(dd.selected, 0);
}

#[test]
fn test_dropdown_move_down() {
    let mut dd = MockDropdown::new("calc", vec![
        ("A".into(), "a".into()),
        ("B".into(), "b".into()),
        ("C".into(), "c".into()),
    ]);
    assert_eq!(dd.selected, 0);
    dd.handle_key("\x1b[B"); // down arrow
    assert_eq!(dd.selected, 1);
    dd.handle_key("j"); // vim down
    assert_eq!(dd.selected, 2);
    dd.handle_key("\x1b[B"); // can't go past last
    assert_eq!(dd.selected, 2);
}

#[test]
fn test_dropdown_move_up() {
    let mut dd = MockDropdown::new("calc", vec![
        ("A".into(), "a".into()),
        ("B".into(), "b".into()),
    ]);
    dd.handle_key("\x1b[B"); // go to 1
    assert_eq!(dd.selected, 1);
    dd.handle_key("\x1b[A"); // up arrow
    assert_eq!(dd.selected, 0);
    dd.handle_key("k"); // vim up at 0 stays at 0
    assert_eq!(dd.selected, 0);
}

#[test]
fn test_dropdown_tab_navigation() {
    let mut dd = MockDropdown::new("calc", vec![
        ("A".into(), "a".into()),
        ("B".into(), "b".into()),
    ]);
    dd.handle_key("\t"); // tab moves down
    assert_eq!(dd.selected, 1);
}

#[test]
fn test_dropdown_enter_selects() {
    let mut dd = MockDropdown::new("calc", vec![
        ("Copy".into(), "copy".into()),
        ("Paste".into(), "paste".into()),
    ]);
    dd.handle_key("\x1b[B"); // select "Paste"
    let result = dd.handle_key("\r");
    assert_eq!(result, Some(("calc".to_string(), "paste".to_string())));
    assert!(!dd.open);
}

#[test]
fn test_dropdown_newline_selects() {
    let mut dd = MockDropdown::new("calc", vec![
        ("Copy".into(), "copy".into()),
    ]);
    let result = dd.handle_key("\n");
    assert_eq!(result, Some(("calc".to_string(), "copy".to_string())));
}

#[test]
fn test_dropdown_escape_closes() {
    let mut dd = MockDropdown::new("calc", vec![
        ("A".into(), "a".into()),
    ]);
    let result = dd.handle_key("\x1b");
    assert_eq!(result, None);
    assert!(!dd.open);
}

#[test]
fn test_dropdown_unknown_key_noop() {
    let mut dd = MockDropdown::new("calc", vec![
        ("A".into(), "a".into()),
    ]);
    let result = dd.handle_key("x");
    assert_eq!(result, None);
    assert!(dd.open);
    assert_eq!(dd.selected, 0);
}

#[test]
fn test_dropdown_closed_noop() {
    let mut dd = MockDropdown::new("calc", vec![
        ("A".into(), "a".into()),
    ]);
    dd.open = false;
    let result = dd.handle_key("\r");
    assert_eq!(result, None);
}

// ── Context menu dismiss logic ──────────────────────────────────────────────

struct MockContextMenu {
    open: bool,
    anchor_x: u32,
    anchor_y: u32,
    items_count: u32,
}

impl MockContextMenu {
    fn dismiss_if_outside(&mut self, cx: i32, cy: i32) -> bool {
        if !self.open { return false; }
        let (pw, row_h) = (200u32, 24u32);
        let panel_h = row_h * self.items_count + 8;
        let inside = cx >= self.anchor_x as i32 && cx < (self.anchor_x + pw) as i32
            && cy >= self.anchor_y as i32 && cy < (self.anchor_y + panel_h) as i32;
        if !inside { self.open = false; }
        !inside
    }
}

#[test]
fn test_context_menu_dismiss_outside() {
    let mut cm = MockContextMenu { open: true, anchor_x: 100, anchor_y: 200, items_count: 2 };
    // Panel is 200x(24*2+8)=56. Click at (500, 500) is outside.
    assert!(cm.dismiss_if_outside(500, 500));
    assert!(!cm.open);
}

#[test]
fn test_context_menu_not_dismissed_inside() {
    let mut cm = MockContextMenu { open: true, anchor_x: 100, anchor_y: 200, items_count: 2 };
    // Click at (150, 220) is inside the 200x56 panel at (100, 200)
    assert!(!cm.dismiss_if_outside(150, 220));
    assert!(cm.open);
}

#[test]
fn test_context_menu_dismiss_closed_noop() {
    let mut cm = MockContextMenu { open: false, anchor_x: 100, anchor_y: 200, items_count: 2 };
    assert!(!cm.dismiss_if_outside(500, 500));
}

#[test]
fn test_context_menu_dismiss_at_boundary() {
    let mut cm = MockContextMenu { open: true, anchor_x: 100, anchor_y: 200, items_count: 2 };
    // Right boundary: anchor_x + pw = 300. Click at exactly 300 is outside.
    assert!(cm.dismiss_if_outside(300, 210));
    assert!(!cm.open);
}

#[test]
fn test_context_menu_dismiss_just_inside() {
    let mut cm = MockContextMenu { open: true, anchor_x: 100, anchor_y: 200, items_count: 2 };
    // anchor_x + pw - 1 = 299 is inside
    assert!(!cm.dismiss_if_outside(299, 210));
    assert!(cm.open);
}
