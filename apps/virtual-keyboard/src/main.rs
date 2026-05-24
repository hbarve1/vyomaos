// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1000;
const H: u32 = 320;
const C_BG: u32         = 0x0D1117FF;
const C_KEY_BG: u32     = 0x21262DFF;
const C_KEY_BORDER: u32 = 0x30363DFF;
const C_KEY_TEXT: u32   = 0xFFFFFFFF;
const C_ACCENT: u32     = 0x58A6FFFF;
const C_SEL: u32        = 0x1F4068FF;
const C_DIM: u32        = 0x8B949EFF;

const KEY_W: u32 = 80;
const KEY_H: u32 = 50;
const ROW_START_X: u32 = 20;

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

// Key layout: (label, char_to_send, x, y, width)
// Special keys use non-printable labels; width overrides KEY_W
fn build_keys() -> Vec<(&'static str, &'static str, u32, u32, u32)> {
    let mut keys = Vec::new();
    // Row 0: Q–P
    let row0 = ["Q","W","E","R","T","Y","U","I","O","P"];
    for (i, k) in row0.iter().enumerate() {
        keys.push((*k, *k, ROW_START_X + i as u32 * (KEY_W + 4), 60, KEY_W));
    }
    // Row 1: A–L
    let row1 = ["A","S","D","F","G","H","J","K","L"];
    for (i, k) in row1.iter().enumerate() {
        keys.push((*k, *k, ROW_START_X + 40 + i as u32 * (KEY_W + 4), 120, KEY_W));
    }
    // Row 2: Z–M
    let row2 = ["Z","X","C","V","B","N","M"];
    for (i, k) in row2.iter().enumerate() {
        keys.push((*k, *k, ROW_START_X + 80 + i as u32 * (KEY_W + 4), 180, KEY_W));
    }
    // Row 3: Space, Backspace, Enter
    keys.push(("SPACE", " ",    ROW_START_X,       240, 400));
    keys.push(("⌫",    "\x7f", ROW_START_X + 408,  240, KEY_W + 40));
    keys.push(("↵",    "",     ROW_START_X + 576,  240, KEY_W + 40));
    keys
}

fn draw(keys: &[(&str, &str, u32, u32, u32)], pressed: Option<usize>) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_DIM, "Virtual Keyboard  —  Ctrl+C to close");
    fill(0, 44, W, 1, C_KEY_BORDER);

    for (i, (label, _, kx, ky, kw)) in keys.iter().enumerate() {
        let is_pressed = pressed == Some(i);
        let bg = if is_pressed { C_SEL } else { C_KEY_BG };
        let bc = if is_pressed { C_ACCENT } else { C_KEY_BORDER };
        fill(*kx, *ky, *kw, KEY_H, bg);
        border(*kx, *ky, *kw, KEY_H, bc);
        let lx = kx + kw / 2 - (label.len() as u32 * 4).min(kw / 2 - 4);
        text(lx, ky + 17, C_KEY_TEXT, label);
    }
    flush();
}

fn hit_test(keys: &[(&str, &str, u32, u32, u32)], mx: u32, my: u32) -> Option<usize> {
    for (i, (_, _, kx, ky, kw)) in keys.iter().enumerate() {
        if mx >= *kx && mx < kx + kw && my >= *ky && my < ky + KEY_H {
            return Some(i);
        }
    }
    None
}

fn main() {
    let stdin = io::stdin();
    let keys = build_keys();
    draw(&keys, None);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        // Mouse click
        if let Some(rest) = raw.strip_prefix("VYOMA_INPUT:mouse:down:") {
            let parts: Vec<&str> = rest.splitn(2, ',').collect();
            if parts.len() == 2 {
                let mx: u32 = parts[0].parse().unwrap_or(0);
                let my: u32 = parts[1].parse().unwrap_or(0);
                if let Some(idx) = hit_test(&keys, mx, my) {
                    let ch = keys[idx].1;
                    draw(&keys, Some(idx));
                    println!("@supervisor: input {ch}");
                    let _ = io::stdout().flush();
                    // Brief highlight then redraw normal
                    draw(&keys, None);
                }
            }
            continue;
        }

        if raw == "\x03" {
            fill(0, 0, W, H, 0x0D1117FF);
            flush();
            std::process::exit(0);
        }
    }
}
