//! Dropdown and context menu state + rendering (T059–T064).

use std::sync::Mutex;
use crate::lock_or_recover;

#[cfg(target_os = "linux")]
use crate::display;

// ── Dropdown menu ────────────────────────────────────────────────────────────

struct DropdownState {
    open: bool, selected: usize, items: Vec<(String, String)>,
    anchor_x: u32, anchor_y: u32, app_name: String,
}
static DROPDOWN_STATE: std::sync::OnceLock<Mutex<DropdownState>> = std::sync::OnceLock::new();
fn dropdown_state() -> &'static Mutex<DropdownState> {
    DROPDOWN_STATE.get_or_init(|| Mutex::new(DropdownState {
        open: false, selected: 0, items: vec![],
        anchor_x: 0, anchor_y: 0, app_name: String::new(),
    }))
}

/// Query whether the dropdown is currently open.
pub fn is_dropdown_open() -> bool { lock_or_recover(&dropdown_state()).open }

/// Open the app-name dropdown at anchor position with menu items.
pub fn open_dropdown(app: &str, items: Vec<(String, String)>, ax: u32, ay: u32) {
    let mut s = lock_or_recover(&dropdown_state());
    s.open = true; s.selected = 0; s.items = items;
    s.anchor_x = ax; s.anchor_y = ay; s.app_name = app.to_string();
}

/// Close the dropdown.
pub fn close_dropdown() { lock_or_recover(&dropdown_state()).open = false; }

/// Handle keyboard navigation within the dropdown.
/// Returns `Some((app_name, action))` when Enter is pressed on a selected item.
pub fn handle_dropdown_key_action(key: &str) -> Option<(String, String)> {
    let mut s = match dropdown_state().try_lock() { Ok(s) => s, Err(_) => return None };
    if !s.open { return None; }
    match key {
        // P82: Tab also moves to next item (keyboard-only navigation)
        "\x1b[B" | "j" | "\t" => { if s.selected + 1 < s.items.len() { s.selected += 1; } None }
        "\x1b[A" | "k" => { if s.selected > 0 { s.selected -= 1; } None }
        "\r" | "\n" | "" => {
            let r = s.items.get(s.selected).map(|(_, a)| (s.app_name.clone(), a.clone()));
            s.open = false; r
        }
        "\x1b" => { s.open = false; None }
        _ => None,
    }
}

/// Render the dropdown menu if open.
#[cfg(target_os = "linux")]
pub fn render_dropdown_if_open(fb: &mut display::Framebuffer) {
    let s = lock_or_recover(&dropdown_state());
    if !s.open || s.items.is_empty() { return; }
    let (ax, ay, items, sel) = (s.anchor_x, s.anchor_y, s.items.clone(), s.selected);
    drop(s);
    render_panel(fb, &items, ax, ay, sel);
}

// ── Context menu (right-click on desktop) ────────────────────────────────────

struct ContextMenuState {
    open: bool, selected: usize, items: Vec<(String, String)>, anchor_x: u32, anchor_y: u32,
}
static CONTEXT_MENU_STATE: std::sync::OnceLock<Mutex<ContextMenuState>> = std::sync::OnceLock::new();
fn context_menu_state() -> &'static Mutex<ContextMenuState> {
    CONTEXT_MENU_STATE.get_or_init(|| Mutex::new(ContextMenuState {
        open: false, selected: 0, items: vec![], anchor_x: 0, anchor_y: 0,
    }))
}

/// Open a desktop context menu at cursor position.
pub fn open_context_menu(cx: u32, cy: u32) {
    let mut s = lock_or_recover(&context_menu_state());
    s.open = true; s.selected = 0; s.anchor_x = cx; s.anchor_y = cy;
    s.items = vec![
        ("New Note".to_string(), "new-note".to_string()),
        ("Open Settings".to_string(), "open-settings".to_string()),
    ];
}

pub fn close_context_menu() { lock_or_recover(&context_menu_state()).open = false; }
pub fn is_context_menu_open() -> bool { lock_or_recover(&context_menu_state()).open }

/// Dismiss context menu if click is outside its bounds. Returns true if dismissed.
pub fn dismiss_context_menu_if_outside(cx: i32, cy: i32) -> bool {
    let s = lock_or_recover(&context_menu_state());
    if !s.open { return false; }
    let (pw, row_h) = (200u32, 24u32);
    let panel_h = row_h * s.items.len() as u32 + 8;
    let inside = cx >= s.anchor_x as i32 && cx < (s.anchor_x + pw) as i32
        && cy >= s.anchor_y as i32 && cy < (s.anchor_y + panel_h) as i32;
    drop(s);
    if !inside { close_context_menu(); }
    !inside
}

/// Render context menu if open (same panel style as dropdown).
#[cfg(target_os = "linux")]
pub fn render_context_menu_if_open(fb: &mut display::Framebuffer) {
    let s = lock_or_recover(&context_menu_state());
    if !s.open || s.items.is_empty() { return; }
    let (ax, ay, items, sel) = (s.anchor_x, s.anchor_y, s.items.clone(), s.selected);
    drop(s);
    render_panel(fb, &items, ax, ay, sel);
}

// ── Shared panel renderer ────────────────────────────────────────────────────

/// Dark translucent bg (95% alpha, radius 8), 13pt glyphs, accent highlight.
#[cfg(target_os = "linux")]
fn render_panel(fb: &mut display::Framebuffer, items: &[(String, String)], ax: u32, ay: u32, sel: usize) {
    use crate::display::draw_rounded_rect;
    let (row_h, pw) = (24u32, 200u32);
    let (fs, fw, fh) = (fb.stride, fb.width, fb.height);
    draw_rounded_rect(&mut fb.back, ax, ay, pw, row_h * items.len() as u32 + 8, 0x1C1C1EF2, 8, fs, fw, fh);
    for (i, (label, _)) in items.iter().enumerate() {
        let ry = ay + 4 + i as u32 * row_h;
        if i == sel { draw_rounded_rect(&mut fb.back, ax + 4, ry, pw - 8, row_h - 2, 0x0A84FFFF, 4, fs, fw, fh); }
        crate::chrome::draw_glyph_str_pub(fb, label, (ax + 12) as i32, (ry + row_h / 2) as i32,
            if i == sel { 0xFFFFFFFF } else { 0xE0E0E0FF }, 13, false, false);
    }
}
