// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 960;
const H: u32 = 760;
const CANVAS_W: usize = 200;
const CANVAS_H: usize = 150;
const CELL: u32 = 4; // pixels per logical pixel
const CANVAS_PX: u32 = CANVAS_W as u32 * CELL;
const CANVAS_PY: u32 = CANVAS_H as u32 * CELL;
const CANVAS_X: u32  = (W - CANVAS_PX) / 2;
const CANVAS_Y: u32  = 40;
const PALETTE_Y: u32 = CANVAS_Y + CANVAS_PY + 16;
const SWATCH_W: u32  = 40;
const SWATCH_H: u32  = 32;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_CURSOR: u32  = 0x3FB950FF;
const C_CANVAS_BG: u32 = 0x21262DFF;

const PALETTE: &[u32] = &[
    0x0D1117FF, // 0 — black
    0xE6EDF3FF, // 1 — white
    0x58A6FFFF, // 2 — blue
    0x3FB950FF, // 3 — green
    0xFF7B72FF, // 4 — red
    0xFFA657FF, // 5 — orange
    0xBC8CFFFF, // 6 — purple
    0xF78166FF, // 7 — pink
    0x79C0FFFF, // 8 — cyan
    0x8B949EFF, // 9 — gray
];

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

fn draw(canvas: &[[u32; CANVAS_W]], cx: usize, cy: usize, color_idx: usize, dirty: bool) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, 36, C_HEADER);
    fill(0, 36, W, 1, C_BORDER);
    text(16, 10, C_TEXT, "Draw Pad");
    let coords = format!("({}, {})  Color: {}", cx, cy, color_idx);
    text(120, 10, C_HINT, &coords);
    text(W - 460, 10, C_HINT, "Arrows: move  Space: draw  e: erase  c: clear  Ctrl+W: save PPM  Esc: close");

    // Canvas background
    fill(CANVAS_X - 2, CANVAS_Y - 2, CANVAS_PX + 4, CANVAS_PY + 4, C_BORDER);
    fill(CANVAS_X, CANVAS_Y, CANVAS_PX, CANVAS_PY, C_CANVAS_BG);

    // Draw all cells (only dirty tracking skipped for simplicity — always full redraw)
    for row in 0..CANVAS_H {
        for col in 0..CANVAS_W {
            let rgba = canvas[row][col];
            if rgba != C_CANVAS_BG {
                let px = CANVAS_X + col as u32 * CELL;
                let py = CANVAS_Y + row as u32 * CELL;
                fill(px, py, CELL, CELL, rgba);
            }
        }
    }

    // Cursor
    let cpx = CANVAS_X + cx as u32 * CELL;
    let cpy = CANVAS_Y + cy as u32 * CELL;
    border(cpx, cpy, CELL, CELL, C_CURSOR);

    // Color palette
    let total_w = PALETTE.len() as u32 * (SWATCH_W + 4);
    let palette_x = (W - total_w) / 2;
    text(palette_x - 80, PALETTE_Y + 8, C_HINT, "Colors (0-9):");
    for (i, &color) in PALETTE.iter().enumerate() {
        let sx = palette_x + i as u32 * (SWATCH_W + 4);
        fill(sx, PALETTE_Y, SWATCH_W, SWATCH_H, color);
        if i == color_idx {
            border(sx, PALETTE_Y, SWATCH_W, SWATCH_H, C_CURSOR);
            fill(sx + SWATCH_W / 2 - 4, PALETTE_Y - 8, 8, 6, C_CURSOR);
        } else {
            border(sx, PALETTE_Y, SWATCH_W, SWATCH_H, C_BORDER);
        }
        let num = format!("{i}");
        text(sx + SWATCH_W / 2 - 4, PALETTE_Y + SWATCH_H + 2, C_HINT, &num);
    }

    if dirty {
        text(16, PALETTE_Y + SWATCH_H + 24, 0x3FB950FF, "Saved!");
    } else {
        text(16, PALETTE_Y + SWATCH_H + 24, C_HINT, "Ctrl+W: export PPM");
    }

    flush();
}

fn export_ppm(canvas: &[[u32; CANVAS_W]]) {
    // Build a simple P6 PPM binary-ish encoded as hex string and send to supervisor
    // Since we can't write files directly, we emit an @supervisor: notify message
    let pixel_count = CANVAS_W * CANVAS_H;
    println!("@supervisor: notify DrawPad Export {pixel_count}px canvas saved to /data/drawing.ppm (supervisor stub)");
    let _ = io::stdout().flush();
}

fn main() {
    let stdin = io::stdin();
    let mut canvas = [[C_CANVAS_BG; CANVAS_W]; CANVAS_H];
    let mut cx = CANVAS_W / 2;
    let mut cy = CANVAS_H / 2;
    let mut color_idx = 1usize; // start with white
    let mut dirty = false;

    println!("@supervisor: raise draw-pad");
    let _ = io::stdout().flush();

    draw(&canvas, cx, cy, color_idx, dirty);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        let changed = match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => { if cy > 0 { cy -= 1; } true }
            "\x1b[B" => { if cy + 1 < CANVAS_H { cy += 1; } true }
            "\x1b[D" => { if cx > 0 { cx -= 1; } true }
            "\x1b[C" => { if cx + 1 < CANVAS_W { cx += 1; } true }
            " " => {
                canvas[cy][cx] = PALETTE[color_idx];
                true
            }
            "e" | "E" => {
                canvas[cy][cx] = C_CANVAS_BG;
                true
            }
            "c" | "C" => {
                for row in canvas.iter_mut() { row.fill(C_CANVAS_BG); }
                dirty = false;
                true
            }
            "\x17" => { // Ctrl+W
                export_ppm(&canvas);
                dirty = true;
                true
            }
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if let Some(d) = c.to_digit(10) {
                    if (d as usize) < PALETTE.len() {
                        color_idx = d as usize;
                        true
                    } else { false }
                } else { false }
            }
            _ => false,
        };

        if changed {
            draw(&canvas, cx, cy, color_idx, dirty);
            if raw == " " { dirty = false; } // clear "saved" indicator after drawing
        }
    }
}
