// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 400;
const H: u32 = 600;
const C_BG: u32     = 0x161B22F0;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_TITLE: u32  = 0x58A6FFFF;
const C_SEP: u32    = 0x21262DFF;

const HEADER_H: u32 = 36;
const ITEM_H: u32   = 72;
const MAX_NOTES: usize = 10;

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

fn draw(notes: &[(String, String)]) {
    fill(0, 0, W, H, C_BG);
    border(0, 0, W, H, C_BORDER);

    // Header
    fill(0, 0, W, HEADER_H, C_SEP);
    text(16, 10, C_TEXT, "Notification Center");
    text(W - 130, 10, C_HINT, "c: clear  Esc: close");
    fill(0, HEADER_H, W, 1, C_BORDER);

    if notes.is_empty() {
        text(W / 2 - 60, H / 2 - 8, C_HINT, "No notifications");
        flush();
        return;
    }

    let visible = notes.len().min((H - HEADER_H - 4) as usize / ITEM_H as usize);
    for (i, (title, msg)) in notes.iter().rev().take(visible).enumerate() {
        let y = HEADER_H + 4 + i as u32 * ITEM_H;
        fill(8, y, W - 16, ITEM_H - 4, 0x1C2128FF);
        border(8, y, W - 16, ITEM_H - 4, C_BORDER);

        // Title with colored dot
        fill(18, y + 14, 6, 6, C_TITLE);
        let t: String = title.chars().take(38).collect();
        text(32, y + 10, C_TITLE, &t);

        // Message
        let m: String = msg.chars().take(46).collect();
        text(18, y + 32, C_DIM, &m);
        if msg.len() > 46 {
            let m2: String = msg.chars().skip(46).take(46).collect();
            text(18, y + 50, C_DIM, &m2);
        }
    }

    // Count
    let count = format!("{} notification{}", notes.len(), if notes.len() == 1 { "" } else { "s" });
    fill(0, H - 24, W, 24, C_SEP);
    fill(0, H - 24, W, 1, C_BORDER);
    text(16, H - 16, C_HINT, &count);

    flush();
}

fn parse_notify(line: &str) -> Option<(String, String)> {
    let payload = line.trim_start_matches("NOTIFY:");
    let mut parts = payload.splitn(2, '|');
    let title = parts.next()?.trim().to_string();
    let msg   = parts.next().unwrap_or("").trim().to_string();
    if title.is_empty() { return None; }
    Some((title, msg))
}

fn main() {
    let stdin = io::stdin();
    let mut notes: Vec<(String, String)> = Vec::new();

    println!("@supervisor: raise notification-center");
    let _ = io::stdout().flush();

    draw(&notes);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") { continue; }

        if raw.starts_with("NOTIFY:") {
            if let Some(note) = parse_notify(&raw) {
                notes.push(note);
                if notes.len() > MAX_NOTES { notes.remove(0); }
                draw(&notes);
                continue;
            }
        }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "c" | "C" => {
                notes.clear();
                draw(&notes);
            }
            _ => {}
        }
    }
}
