// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1000;
const H: u32 = 760;
const HEADER_H: u32 = 48;

const COLS: usize = 32;
const ROWS: usize = 32;
const CELL: u32 = 16;
const CANVAS_X: u32 = 80;
const CANVAS_Y: u32 = 64;
const PAL_X: u32 = CANVAS_X + COLS as u32 * CELL + 24;
const PAL_SZ: u32 = 36;
const PAL_GAP: u32 = 8;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_GRID: u32    = 0x1C2128FF;

const PALETTE: [u32; 16] = [
    0x000000FF, 0xFFFFFFFF, 0xFF3B30FF, 0x30D158FF,
    0x0A84FFFF, 0xFFD60AFF, 0x32D2FFFF, 0xFF375FFF,
    0xFF9F0AFF, 0xBF5AF2FF, 0xA2845EFF, 0xFF6EB4FF,
    0x30D158FF, 0x003366FF, 0x008080FF, 0x8E8E93FF,
];
const PAL_NAMES: [&str; 16] = [
    "Black","White","Red","Green","Blue","Yellow","Cyan","Magenta",
    "Orange","Purple","Brown","Pink","Lime","Navy","Teal","Grey",
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

const EMPTY: u32 = 0x1A1A1AFF;

struct App {
    canvas:  Vec<u32>,       // ROWS*COLS colors
    undo:    Vec<Vec<u32>>,  // up to 10 snapshots
    cx:      usize,
    cy:      usize,
    color:   usize,
}

impl App {
    fn new() -> Self {
        App {
            canvas: vec![EMPTY; ROWS * COLS],
            undo:   Vec::new(),
            cx: 0,
            cy: 0,
            color: 1, // white
        }
    }

    fn idx(r: usize, c: usize) -> usize { r * COLS + c }

    fn snapshot(&mut self) {
        if self.undo.len() >= 10 { self.undo.remove(0); }
        self.undo.push(self.canvas.clone());
    }

    fn undo(&mut self) {
        if let Some(snap) = self.undo.pop() {
            self.canvas = snap;
        }
    }

    fn draw_pixel(&mut self) {
        self.snapshot();
        self.canvas[Self::idx(self.cy, self.cx)] = PALETTE[self.color];
    }

    fn erase(&mut self) {
        self.snapshot();
        self.canvas[Self::idx(self.cy, self.cx)] = EMPTY;
    }

    fn clear(&mut self) {
        self.snapshot();
        self.canvas = vec![EMPTY; ROWS * COLS];
    }

    fn flood_fill(&mut self, target: u32, fill_color: u32, r: usize, c: usize) {
        if r >= ROWS || c >= COLS { return; }
        if self.canvas[Self::idx(r, c)] != target { return; }
        if fill_color == target { return; }
        // BFS
        let mut queue: Vec<(usize, usize)> = Vec::new();
        self.canvas[Self::idx(r, c)] = fill_color;
        queue.push((r, c));
        let mut qi = 0;
        while qi < queue.len() {
            let (qr, qc) = queue[qi]; qi += 1;
            let neighbors = [(qr.wrapping_sub(1), qc), (qr+1, qc), (qr, qc.wrapping_sub(1)), (qr, qc+1)];
            for (nr, nc) in neighbors {
                if nr < ROWS && nc < COLS && self.canvas[Self::idx(nr, nc)] == target {
                    self.canvas[Self::idx(nr, nc)] = fill_color;
                    queue.push((nr, nc));
                }
            }
        }
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H-1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Pixel Art");
    text(160, 16, C_HINT, &format!("Color: {}  ({},{})", PAL_NAMES[app.color], app.cx, app.cy));
    text(500, 16, C_HINT, "Arrows:move  Spc:draw  e:erase  f:fill  u:undo  c:clear  Tab:color");

    // Canvas background
    fill(CANVAS_X-1, CANVAS_Y-1, COLS as u32*CELL+2, ROWS as u32*CELL+2, C_BORDER);

    // Grid + pixels
    for r in 0..ROWS {
        for c in 0..COLS {
            let px = CANVAS_X + c as u32 * CELL;
            let py = CANVAS_Y + r as u32 * CELL;
            let color = app.canvas[App::idx(r, c)];
            fill(px, py, CELL, CELL, color);
            // Grid lines
            fill(px, py, CELL, 1, C_GRID);
            fill(px, py, 1, CELL, C_GRID);
        }
    }

    // Cursor highlight
    let cx = CANVAS_X + app.cx as u32 * CELL;
    let cy = CANVAS_Y + app.cy as u32 * CELL;
    border(cx, cy, CELL, CELL, C_SEL);

    // Palette
    text(PAL_X, CANVAS_Y - 16, C_HINT, "Palette");
    for i in 0..16 {
        let row = i / 2;
        let col = i % 2;
        let px = PAL_X + col as u32 * (PAL_SZ + PAL_GAP);
        let py = CANVAS_Y + row as u32 * (PAL_SZ + PAL_GAP);
        fill(px, py, PAL_SZ, PAL_SZ, PALETTE[i]);
        if i == app.color {
            border(px-2, py-2, PAL_SZ+4, PAL_SZ+4, C_SEL);
        } else {
            border(px, py, PAL_SZ, PAL_SZ, C_BORDER);
        }
    }

    // Current color swatch
    let swatch_y = CANVAS_Y + 8 * (PAL_SZ + PAL_GAP) + 16;
    fill(PAL_X, swatch_y, PAL_SZ*2+PAL_GAP, PAL_SZ, PALETTE[app.color]);
    text(PAL_X, swatch_y + PAL_SZ + 4, C_HINT, PAL_NAMES[app.color]);

    // Undo count
    text(PAL_X, swatch_y + PAL_SZ + 24, C_HINT, &format!("Undo: {}/10", app.undo.len()));

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise pixel-art");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => { if app.cy > 0      { app.cy -= 1; } }
            "\x1b[B" => { if app.cy+1 < ROWS { app.cy += 1; } }
            "\x1b[C" => { if app.cx+1 < COLS { app.cx += 1; } }
            "\x1b[D" => { if app.cx > 0      { app.cx -= 1; } }
            " "       => { app.draw_pixel(); }
            "e" | "E" => { app.erase(); }
            "c" | "C" => { app.clear(); }
            "u" | "U" | "\x1a" => { app.undo(); } // u or Ctrl+Z
            "f" | "F" => {
                let target = app.canvas[App::idx(app.cy, app.cx)];
                let fill_c = PALETTE[app.color];
                app.snapshot();
                app.flood_fill(target, fill_c, app.cy, app.cx);
            }
            "\t" => { app.color = (app.color + 1) % 16; }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= b'1' && b <= b'9' { app.color = (b - b'1') as usize; }
                if b == b'0' { app.color = 9; }
            }
            _ => {}
        }
        draw(&app);
    }
}
