// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Keyboard shortcut overlay app.
//!
//! Displays a full-screen semi-transparent overlay listing every keyboard
//! shortcut recognised by the VyomaOS supervisor, grouped by category.
//! Dismiss with Escape or Ctrl+/.

use std::io::{self, BufRead, Write};

// --- Screen and layout constants -------------------------------------------

const W: u32 = 1440;
const H: u32 = 900;

// Two-column layout
const COL_LEFT_X: u32 = 80;
const COL_RIGHT_X: u32 = 740;
const HEADER_Y: u32 = 60;
const SECTION_GAP: u32 = 44;
const ROW_H: u32 = 28;

// --- Colour palette (packed RGBA) ------------------------------------------

const C_BG: u32 = 0x0A0A0ADD;        // dark translucent background
const C_TITLE: u32 = 0xFFFFFFFF;     // white
const C_ACCENT: u32 = 0x58A6FFFF;    // blue accent for category headers
const C_KEY: u32 = 0xE6EDF3FF;       // bright white for key combos
const C_DESC: u32 = 0x8B949EFF;      // muted grey for descriptions
const C_KEY_BG: u32 = 0x21262DFF;    // dark chip background for key badges
const C_BORDER: u32 = 0x30363DFF;    // subtle border
const C_DISMISS: u32 = 0x6E7681FF;   // dim hint text

// --- Draw helpers ----------------------------------------------------------

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}

fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}

fn text_small(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},s,{s}");
}

fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}

fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

// --- Shortcut data ---------------------------------------------------------

struct Shortcut {
    key: &'static str,
    desc: &'static str,
}

struct Category {
    name: &'static str,
    shortcuts: &'static [Shortcut],
}

const SYSTEM: &[Shortcut] = &[
    Shortcut { key: "Ctrl+L",     desc: "Lock Screen" },
    Shortcut { key: "Ctrl+Q",     desc: "Quit App" },
    Shortcut { key: "Alt+?",      desc: "Show Shortcut Help Toast" },
    Shortcut { key: "Escape",     desc: "Close Overlay / Cancel" },
];

const WINDOW: &[Shortcut] = &[
    Shortcut { key: "Alt+Tab",       desc: "Next Window" },
    Shortcut { key: "Alt+Shift+Tab", desc: "Previous Window" },
    Shortcut { key: "Alt+W",         desc: "Close Window" },
    Shortcut { key: "Alt+F",         desc: "Snap / Maximize Window" },
];

const NAVIGATION: &[Shortcut] = &[
    Shortcut { key: "Ctrl+Left",  desc: "Switch to Previous Workspace" },
    Shortcut { key: "Ctrl+Right", desc: "Switch to Next Workspace" },
    Shortcut { key: "Arrow Up",   desc: "Navigate Up" },
    Shortcut { key: "Arrow Down", desc: "Navigate Down" },
    Shortcut { key: "Enter",      desc: "Select / Activate" },
];

const EDITING: &[Shortcut] = &[
    Shortcut { key: "Ctrl+C", desc: "Copy" },
    Shortcut { key: "Ctrl+V", desc: "Paste" },
    Shortcut { key: "Ctrl+S", desc: "Save" },
];

const CATEGORIES: &[Category] = &[
    Category { name: "System",     shortcuts: SYSTEM },
    Category { name: "Window",     shortcuts: WINDOW },
    Category { name: "Navigation", shortcuts: NAVIGATION },
    Category { name: "Editing",    shortcuts: EDITING },
];

// --- Key badge helper ------------------------------------------------------

/// Draw a rounded-ish key badge: dark chip with the key name inside.
fn draw_key_badge(x: u32, y: u32, key: &str) -> u32 {
    let char_w: u32 = 8; // medium font character width
    let pad: u32 = 8;
    let badge_w = (key.len() as u32) * char_w + pad * 2;
    let badge_h: u32 = 20;
    let badge_y = y.wrapping_sub(2);
    fill(x, badge_y, badge_w, badge_h, C_KEY_BG);
    border(x, badge_y, badge_w, badge_h, C_BORDER);
    text(x + pad, y, C_KEY, key);
    badge_w + 16 // return total width consumed (badge + gap)
}

// --- Main draw function ----------------------------------------------------

fn draw_overlay() {
    // Full-screen translucent background
    fill(0, 0, W, H, C_BG);

    // Outer border
    border(40, 30, W - 80, H - 60, C_BORDER);

    // Title centred at top
    let title = "Keyboard Shortcuts";
    let title_x = W / 2 - (title.len() as u32 * 8) / 2;
    text(title_x, HEADER_Y, C_TITLE, title);

    // Divider line below title
    fill(80, HEADER_Y + 26, W - 160, 1, C_BORDER);

    // Lay out categories in two columns
    let mut left_y = HEADER_Y + 50;
    let mut right_y = HEADER_Y + 50;

    for (i, cat) in CATEGORIES.iter().enumerate() {
        let (col_x, y) = if i < 2 {
            (COL_LEFT_X, &mut left_y)
        } else {
            (COL_RIGHT_X, &mut right_y)
        };

        // Category header
        text(col_x, *y, C_ACCENT, cat.name);
        fill(col_x, *y + 20, 200, 1, C_ACCENT);
        *y += SECTION_GAP;

        // Shortcut rows
        for sc in cat.shortcuts {
            let badge_end = draw_key_badge(col_x, *y, sc.key);
            text(col_x + badge_end, *y, C_DESC, sc.desc);
            *y += ROW_H;
        }

        // Gap before next category
        *y += 16;
    }

    // Dismiss hint at bottom
    let hint = "Press Escape or Ctrl+/ to close";
    let hint_x = W / 2 - (hint.len() as u32 * 4) / 2; // small font is ~4px wide
    text_small(hint_x, H - 50, C_DISMISS, hint);

    flush();
}

// --- Clear and exit --------------------------------------------------------

fn clear_and_exit() {
    fill(0, 0, W, H, 0x0D1117FF);
    flush();
    std::process::exit(0);
}

// --- Main loop -------------------------------------------------------------

fn main() {
    draw_overlay();

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Escape (plain or with bracket prefix)
        if raw == "\x1b" || raw == "\x1b[" {
            clear_and_exit();
        }

        // Ctrl+/ sends 0x1F in raw terminal mode
        if raw == "\x1f" {
            clear_and_exit();
        }

        // Ctrl+C — also dismiss
        if raw == "\x03" {
            clear_and_exit();
        }

        // Ignore everything else — overlay is read-only
    }
}
