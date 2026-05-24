// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 420;
const H: u32 = 560;
const HEADER_H: u32 = 40;
const FOOTER_H: u32 = 28;
const ROW_H: u32    = 36;
const MAX_ENTRIES: usize = 20;

const C_BG: u32     = 0x161B22FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_SEL_BG: u32 = 0x1F4068FF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;

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

fn draw(entries: &[String], cursor: usize, last_action: &str) {
    fill(0, 0, W, H, C_BG);
    border(0, 0, W, H, C_BORDER);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    text(16, 12, C_TEXT, "Clipboard History");
    let count = format!("{}/{}", entries.len(), MAX_ENTRIES);
    text(W - 60, 12, C_HINT, &count);

    if entries.is_empty() {
        text(20, H / 2 - 8, C_HINT, "Clipboard is empty");
        // Footer
        fill(0, H - FOOTER_H, W, FOOTER_H, C_HEADER);
        fill(0, H - FOOTER_H, W, 1, C_BORDER);
        text(16, H - FOOTER_H + 8, C_HINT, "Esc: close");
        flush();
        return;
    }

    let visible = ((H - HEADER_H - FOOTER_H) / ROW_H) as usize;

    for (i, entry) in entries.iter().take(visible).enumerate() {
        let ry = HEADER_H + i as u32 * ROW_H;
        let is_sel = i == cursor;

        let bg = if is_sel { C_SEL_BG } else if i % 2 == 0 { C_BG } else { 0x1C2128FF };
        fill(0, ry, W, ROW_H, bg);
        if is_sel { fill(0, ry, 3, ROW_H, C_SEL); }

        // Index number
        let idx = format!("{:2}", i + 1);
        text(8, ry + 10, C_HINT, &idx);

        // Entry preview
        let preview: String = entry.chars().take(44).collect();
        let tc = if is_sel { C_TEXT } else { C_DIM };
        text(36, ry + 10, tc, &preview);
        if entry.len() > 44 {
            text(W - 24, ry + 10, C_HINT, "…");
        }
    }

    // Action feedback
    if !last_action.is_empty() {
        let aw = last_action.len() as u32 * 8;
        fill(0, H - FOOTER_H - 20, W, 20, 0x21262DFF);
        text((W - aw) / 2, H - FOOTER_H - 14, C_GREEN, last_action);
    }

    // Footer
    fill(0, H - FOOTER_H, W, FOOTER_H, C_HEADER);
    fill(0, H - FOOTER_H, W, 1, C_BORDER);
    text(16, H - FOOTER_H + 8, C_HINT, "Enter: restore  c: paste  Del: remove  Esc: close");

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut entries: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    let mut last_action = String::new();

    // Query current clipboard
    println!("@supervisor: clipboard-get");
    println!("@supervisor: raise clipboard-history");
    let _ = io::stdout().flush();

    draw(&entries, cursor, &last_action);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:clipboard ") {
            let content = raw.trim_start_matches("REPLY:clipboard ").to_string();
            if !content.is_empty() && (entries.is_empty() || entries[0] != content) {
                entries.insert(0, content);
                if entries.len() > MAX_ENTRIES { entries.pop(); }
                if cursor >= entries.len() { cursor = entries.len().saturating_sub(1); }
            }
            draw(&entries, cursor, &last_action);
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        let n = entries.len();
        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if cursor > 0 { cursor -= 1; }
                last_action.clear();
                draw(&entries, cursor, &last_action);
            }
            "\x1b[B" => {
                if cursor + 1 < n { cursor += 1; }
                last_action.clear();
                draw(&entries, cursor, &last_action);
            }
            "" => {
                if let Some(entry) = entries.get(cursor) {
                    println!("@supervisor: clipboard-set {entry}");
                    let _ = io::stdout().flush();
                    last_action = "Restored to clipboard".to_string();
                }
                draw(&entries, cursor, &last_action);
            }
            "c" | "C" => {
                if let Some(entry) = entries.get(cursor) {
                    let e = entry.clone();
                    println!("@supervisor: input {e}");
                    let _ = io::stdout().flush();
                    last_action = "Pasted to focused app".to_string();
                }
                draw(&entries, cursor, &last_action);
            }
            "\x7f" => {
                if !entries.is_empty() {
                    entries.remove(cursor);
                    if cursor >= entries.len() && cursor > 0 { cursor -= 1; }
                    last_action = "Entry removed".to_string();
                }
                draw(&entries, cursor, &last_action);
            }
            _ => {}
        }
    }
}
