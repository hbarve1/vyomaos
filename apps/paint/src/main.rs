// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 960;
const H: u32 = 760;
const HEADER_H: u32 = 48;

const COLS: usize = 160;
const ROWS: usize = 120;
const CELL: u32 = 5;
const CANVAS_X: u32 = 80;
const CANVAS_Y: u32 = 64;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;

const PALETTE: [u32; 10] = [
    0x000000FF, // black
    0xFFFFFFFF, // white
    0xFF3B30FF, // red
    0x30D158FF, // green
    0x0A84FFFF, // blue
    0xFFD60AFF, // yellow
    0x32D2FFFF, // cyan
    0xFF375FFF, // magenta
    0xFF9F0AFF, // orange
    0x8E8E93FF, // grey
];

const PALETTE_NAMES: [&str; 10] = [
    "Black", "White", "Red", "Green", "Blue",
    "Yellow", "Cyan", "Magenta", "Orange", "Grey",
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

struct Game {
    canvas:  Box<[[u32; COLS]; ROWS]>,
    cx:      usize,
    cy:      usize,
    color:   usize, // palette index
    drawing: bool,  // space held = draw mode
    dirty:   bool,
}

impl Game {
    fn new() -> Self {
        Game {
            canvas:  Box::new([[0x1A1A1AFF; COLS]; ROWS]),
            cx: COLS / 2,
            cy: ROWS / 2,
            color: 1, // start with white
            drawing: false,
            dirty: true,
        }
    }

    fn draw_pixel(&mut self) {
        self.canvas[self.cy][self.cx] = PALETTE[self.color];
        self.dirty = true;
    }

    fn erase_pixel(&mut self) {
        self.canvas[self.cy][self.cx] = 0x1A1A1AFF;
        self.dirty = true;
    }

    fn clear(&mut self) {
        for row in self.canvas.iter_mut() {
            for px in row.iter_mut() {
                *px = 0x1A1A1AFF;
            }
        }
        self.dirty = true;
    }
}

const PAL_X: u32 = CANVAS_X;
const PAL_Y: u32 = CANVAS_Y + ROWS as u32 * CELL + 16;
const PAL_SZ: u32 = 28;
const PAL_GAP: u32 = 6;

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Paint");
    text(100, 16, C_HINT, &format!("Color: {}  ({},{})", PALETTE_NAMES[g.color], g.cx, g.cy));
    text(500, 16, C_HINT, "Arrows:move  Space:draw  e:erase  c:clear  Tab:color");

    // Canvas background
    fill(CANVAS_X - 2, CANVAS_Y - 2, COLS as u32 * CELL + 4, ROWS as u32 * CELL + 4, C_BORDER);

    // Draw canvas pixels
    for r in 0..ROWS {
        for c in 0..COLS {
            let px = CANVAS_X + c as u32 * CELL;
            let py = CANVAS_Y + r as u32 * CELL;
            fill(px, py, CELL, CELL, g.canvas[r][c]);
        }
    }

    // Cursor highlight
    let cx = CANVAS_X + g.cx as u32 * CELL;
    let cy = CANVAS_Y + g.cy as u32 * CELL;
    border(cx, cy, CELL, CELL, C_SEL);

    // Palette strip
    for (i, &color) in PALETTE.iter().enumerate() {
        let px = PAL_X + i as u32 * (PAL_SZ + PAL_GAP);
        fill(px, PAL_Y, PAL_SZ, PAL_SZ, color);
        if i == g.color {
            border(px - 2, PAL_Y - 2, PAL_SZ + 4, PAL_SZ + 4, C_SEL);
        } else {
            border(px, PAL_Y, PAL_SZ, PAL_SZ, C_BORDER);
        }
    }

    // Color label
    let sel_px = PAL_X + g.color as u32 * (PAL_SZ + PAL_GAP);
    fill(sel_px, PAL_Y + PAL_SZ + 4, PAL_SZ, 12, PALETTE[g.color]);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise paint");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                if game.cy > 0 { game.cy -= 1; }
                if game.drawing { game.draw_pixel(); }
            }
            "\x1b[B" => {
                if game.cy + 1 < ROWS { game.cy += 1; }
                if game.drawing { game.draw_pixel(); }
            }
            "\x1b[C" => {
                if game.cx + 1 < COLS { game.cx += 1; }
                if game.drawing { game.draw_pixel(); }
            }
            "\x1b[D" => {
                if game.cx > 0 { game.cx -= 1; }
                if game.drawing { game.draw_pixel(); }
            }
            " " => { game.drawing = !game.drawing; game.draw_pixel(); }
            "e" | "E" => { game.erase_pixel(); game.drawing = false; }
            "c" | "C" => { game.clear(); }
            "\t" => { game.color = (game.color + 1) % PALETTE.len(); }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= b'1' && b <= b'9' {
                    let idx = (b - b'1') as usize;
                    if idx < PALETTE.len() { game.color = idx; }
                }
                if b == b'0' { game.color = 0; }
            }
            _ => {}
        }
        draw(&game);
    }
}
