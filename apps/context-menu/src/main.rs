// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 220;
const C_BG: u32     = 0x1C2128FF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x1F4068FF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_ACCENT: u32 = 0x58A6FFFF;

const ITEM_H: u32   = 28;
const PADDING: u32  = 6;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn parse_menu(line: &str) -> (u32, u32, Vec<String>) {
    // Format: MENU:<x>,<y>:<item1>|<item2>|...
    let payload = line.trim_start_matches("MENU:");
    let mut parts = payload.splitn(2, ':');
    let coords = parts.next().unwrap_or("0,0");
    let items_str = parts.next().unwrap_or("");

    let mut cp = coords.splitn(2, ',');
    let x: u32 = cp.next().unwrap_or("0").parse().unwrap_or(0);
    let y: u32 = cp.next().unwrap_or("0").parse().unwrap_or(0);

    let items: Vec<String> = items_str
        .split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    (x, y, items)
}

fn draw(items: &[String], cursor: usize) {
    let h = PADDING * 2 + items.len() as u32 * ITEM_H;
    fill(0, 0, W, h, C_BG);
    border(0, 0, W, h, C_BORDER);

    for (i, item) in items.iter().enumerate() {
        let iy = PADDING + i as u32 * ITEM_H;
        if i == cursor {
            fill(1, iy, W - 2, ITEM_H, C_SEL);
            fill(1, iy, 3, ITEM_H, C_ACCENT);
        }
        let tc = if item.starts_with("---") {
            // Separator
            fill(8, iy + ITEM_H / 2, W - 16, 1, C_BORDER);
            continue;
        } else if i == cursor { C_TEXT } else { C_DIM };

        let label: String = item.chars().take(24).collect();
        text(12, iy + 8, tc, &label);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut items: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    let mut positioned = false;
    let mut menu_received = false;

    // Default items shown if no MENU: spec arrives
    let default_items = vec![
        "Open".to_string(),
        "Copy".to_string(),
        "Paste".to_string(),
        "---".to_string(),
        "Delete".to_string(),
        "---".to_string(),
        "Properties".to_string(),
    ];

    println!("@supervisor: raise context-menu");
    let _ = io::stdout().flush();

    // Draw empty initially
    draw(&default_items, cursor);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("MENU:") {
            let (x, y, parsed_items) = parse_menu(&raw);
            items = parsed_items;
            if items.is_empty() { items = default_items.clone(); }
            cursor = 0;
            menu_received = true;

            if !positioned {
                println!("@supervisor: reposition context-menu {x} {y}");
                let _ = io::stdout().flush();
                positioned = true;
            }

            draw(&items, cursor);
            continue;
        }

        if raw.starts_with("REPLY:") { continue; }

        // Use defaults if no MENU: ever arrived
        if !menu_received { items = default_items.clone(); menu_received = true; }

        let n = items.len();
        let selectable: Vec<usize> = (0..n).filter(|&i| !items[i].starts_with("---")).collect();

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, 300, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                // Move cursor to previous non-separator
                if let Some(pos) = selectable.iter().position(|&i| i == cursor) {
                    if pos > 0 { cursor = selectable[pos - 1]; }
                } else if !selectable.is_empty() {
                    cursor = *selectable.last().unwrap();
                }
                draw(&items, cursor);
            }
            "\x1b[B" => {
                if let Some(pos) = selectable.iter().position(|&i| i == cursor) {
                    if pos + 1 < selectable.len() { cursor = selectable[pos + 1]; }
                } else if !selectable.is_empty() {
                    cursor = selectable[0];
                }
                draw(&items, cursor);
            }
            "" => {
                if !items.is_empty() && !items[cursor].starts_with("---") {
                    let selected = &items[cursor];
                    println!("@supervisor: context-reply {selected}");
                    let _ = io::stdout().flush();
                }
                fill(0, 0, W, 300, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            _ => {}
        }
    }
}
