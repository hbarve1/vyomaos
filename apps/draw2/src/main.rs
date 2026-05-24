// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1280;
const H: u32 = 800;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CANVAS_W: usize = 300;
const CANVAS_H: usize = 200;
const CELL: u32 = 3; // display pixels per canvas pixel
const CANVAS_PX: u32 = CANVAS_W as u32 * CELL; // 900
const CANVAS_PY: u32 = HEADER_H;
const SIDEBAR_X: u32 = CANVAS_PX;
const SIDEBAR_W: u32 = W - SIDEBAR_X; // 380
const CHAR_W: u32 = 8;
const MAX_UNDO: usize = 10;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_RED: u32    = 0xFF7B72FF;

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

const PALETTE: [u32; 16] = [
    0x000000FF, 0xFFFFFFFF, 0xFF0000FF, 0x00CC00FF,
    0x0000FFFF, 0xFFFF00FF, 0xFF00FFFF, 0x00FFFFFF,
    0xFF8800FF, 0x880088FF, 0x008844FF, 0xFF88CCFF,
    0x664400FF, 0x008888FF, 0x888888FF, 0xCCCCCCFF,
];

#[derive(Clone, Copy, PartialEq)]
enum Tool { Brush, Eraser, Fill, Line }

impl Tool {
    fn name(self) -> &'static str {
        match self { Tool::Brush=>"BRUSH", Tool::Eraser=>"ERASER", Tool::Fill=>"FILL", Tool::Line=>"LINE" }
    }
}

struct Canvas {
    pixels:     Vec<u32>,          // CANVAS_W*CANVAS_H flat, index = y*CANVAS_W+x
    undo_stack: Vec<Vec<u32>>,
    cur_x:      usize,
    cur_y:      usize,
    tool:       Tool,
    color_idx:  usize,
    draw_mode:  bool,              // D key: auto-brush on cursor move
    line_start: Option<(usize, usize)>,
    status:     String,
}

impl Canvas {
    fn new() -> Self {
        Canvas {
            pixels: vec![0xFFFFFFFF; CANVAS_W * CANVAS_H],
            undo_stack: Vec::new(),
            cur_x: 0,
            cur_y: 0,
            tool: Tool::Brush,
            color_idx: 0,
            draw_mode: false,
            line_start: None,
            status: String::new(),
        }
    }

    fn idx(x: usize, y: usize) -> usize { y * CANVAS_W + x }

    fn push_undo(&mut self) {
        if self.undo_stack.len() >= MAX_UNDO { self.undo_stack.remove(0); }
        self.undo_stack.push(self.pixels.clone());
    }

    fn color(&self) -> u32 { PALETTE[self.color_idx] }

    fn apply_tool(&mut self) {
        match self.tool {
            Tool::Brush => {
                self.push_undo();
                self.pixels[Self::idx(self.cur_x, self.cur_y)] = self.color();
            }
            Tool::Eraser => {
                self.push_undo();
                self.pixels[Self::idx(self.cur_x, self.cur_y)] = 0xFFFFFFFF;
            }
            Tool::Fill => {
                self.push_undo();
                self.flood_fill(self.cur_x, self.cur_y, self.color());
            }
            Tool::Line => {
                if let Some((sx, sy)) = self.line_start {
                    self.push_undo();
                    self.draw_line(sx, sy, self.cur_x, self.cur_y);
                    self.line_start = None;
                    self.status = "Line drawn".to_string();
                } else {
                    self.line_start = Some((self.cur_x, self.cur_y));
                    self.status = format!("Line start: ({},{})", self.cur_x, self.cur_y);
                }
            }
        }
    }

    fn flood_fill(&mut self, sx: usize, sy: usize, new_col: u32) {
        let old_col = self.pixels[Self::idx(sx, sy)];
        if old_col == new_col { return; }
        let mut stack = vec![(sx, sy)];
        while let Some((x, y)) = stack.pop() {
            if self.pixels[Self::idx(x, y)] != old_col { continue; }
            self.pixels[Self::idx(x, y)] = new_col;
            if x > 0 { stack.push((x-1, y)); }
            if x + 1 < CANVAS_W { stack.push((x+1, y)); }
            if y > 0 { stack.push((x, y-1)); }
            if y + 1 < CANVAS_H { stack.push((x, y+1)); }
        }
    }

    fn draw_line(&mut self, x0: usize, y0: usize, x1: usize, y1: usize) {
        let (mut px, mut py) = (x0 as i32, y0 as i32);
        let (ex, ey) = (x1 as i32, y1 as i32);
        let dx = (ex - px).abs();
        let dy = -(ey - py).abs();
        let sx = if px < ex { 1i32 } else { -1 };
        let sy = if py < ey { 1i32 } else { -1 };
        let mut err = dx + dy;
        let col = self.color();
        loop {
            if px >= 0 && px < CANVAS_W as i32 && py >= 0 && py < CANVAS_H as i32 {
                self.pixels[Self::idx(px as usize, py as usize)] = col;
            }
            if px == ex && py == ey { break; }
            let e2 = 2 * err;
            if e2 >= dy { err += dy; px += sx; }
            if e2 <= dx { err += dx; py += sy; }
        }
    }

    fn save_ppm(&self) -> Result<(), String> {
        let mut data: Vec<u8> = b"P6\n300 200\n255\n".to_vec();
        data.reserve(CANVAS_W * CANVAS_H * 3);
        for &p in &self.pixels {
            data.push(((p >> 24) & 0xFF) as u8);
            data.push(((p >> 16) & 0xFF) as u8);
            data.push(((p >>  8) & 0xFF) as u8);
        }
        std::fs::write("/data/draw2.ppm", data).map_err(|e| e.to_string())
    }

    fn load_ppm(&mut self) -> Result<(), String> {
        let bytes = std::fs::read("/data/draw2.ppm").map_err(|e| e.to_string())?;
        let mut pos = 0usize;
        // Skip 3 header lines (P6, dimensions, maxval)
        for _ in 0..3 {
            while pos < bytes.len() && bytes[pos] != b'\n' { pos += 1; }
            pos += 1;
        }
        for i in 0..(CANVAS_W * CANVAS_H) {
            if pos + 2 >= bytes.len() { break; }
            let r = bytes[pos] as u32;
            let g = bytes[pos+1] as u32;
            let b = bytes[pos+2] as u32;
            self.pixels[i] = (r << 24) | (g << 16) | (b << 8) | 0xFF;
            pos += 3;
        }
        Ok(())
    }
}

fn draw_canvas_pixels(pixels: &[u32]) {
    // Run-length encode rows to minimize fill_rect calls
    for y in 0..CANVAS_H {
        let row_y = CANVAS_PY + y as u32 * CELL;
        let mut x = 0usize;
        while x < CANVAS_W {
            let col = pixels[y * CANVAS_W + x];
            let mut run = 1usize;
            while x + run < CANVAS_W && pixels[y * CANVAS_W + x + run] == col {
                run += 1;
            }
            fill(x as u32 * CELL, row_y, run as u32 * CELL, CELL, col);
            x += run;
        }
    }
}

fn draw(c: &Canvas) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Draw v2");
    text(130, 16, C_HINT, "B/E/F/L:tool  1-9,a-g:color  Space:apply  D:draw-mode  Ctrl+Z:undo  Ctrl+X:clear  Ctrl+W:save");

    // Canvas
    draw_canvas_pixels(&c.pixels);

    // Cursor highlight
    let cx_disp = c.cur_x as u32 * CELL;
    let cy_disp = CANVAS_PY + c.cur_y as u32 * CELL;
    border(cx_disp, cy_disp, CELL, CELL, 0xFF0000FF);

    // Line start indicator
    if let Some((lx, ly)) = c.line_start {
        let ldx = lx as u32 * CELL;
        let ldy = CANVAS_PY + ly as u32 * CELL;
        border(ldx.saturating_sub(1), ldy.saturating_sub(1), CELL + 2, CELL + 2, C_ORANGE);
    }

    // Canvas border
    border(0, CANVAS_PY, CANVAS_PX, CANVAS_H as u32 * CELL, C_BORDER);

    // Sidebar
    fill(SIDEBAR_X, HEADER_H, SIDEBAR_W, H - HEADER_H - STATUS_H, C_CARD);
    fill(SIDEBAR_X, HEADER_H, 1, H - HEADER_H - STATUS_H, C_BORDER);

    let sx = SIDEBAR_X + 8;
    let mut sy = HEADER_H + 8;

    // Tools
    text(sx, sy, C_HINT, "TOOLS");
    sy += 20;
    for (tool, key) in &[(Tool::Brush,"B"),(Tool::Eraser,"E"),(Tool::Fill,"F"),(Tool::Line,"L")] {
        let is_cur = c.tool == *tool;
        let bg = if is_cur { C_SEL_BG } else { C_CARD };
        let tc = if is_cur { C_SEL } else { C_HINT };
        fill(sx - 4, sy - 2, SIDEBAR_W - 12, 20, bg);
        if is_cur { border(sx - 4, sy - 2, SIDEBAR_W - 12, 20, C_SEL); }
        text(sx, sy, tc, &format!("[{}] {}", key, tool.name()));
        sy += 22;
    }

    sy += 8;
    fill(SIDEBAR_X + 4, sy, SIDEBAR_W - 8, 1, C_BORDER);
    sy += 8;

    // Palette
    text(sx, sy, C_HINT, "COLORS (1-9, a-g)");
    sy += 18;
    for (i, &pal_col) in PALETTE.iter().enumerate() {
        let px = sx + (i % 4) as u32 * 44;
        let py = sy + (i / 4) as u32 * 44;
        fill(px, py, 40, 40, pal_col);
        if i == c.color_idx {
            border(px - 1, py - 1, 42, 42, C_SEL);
        }
        // Key label
        let key = if i < 9 { format!("{}", i + 1) } else { format!("{}", (b'a' + (i - 9) as u8) as char) };
        text(px + 2, py + 28, if pal_col == 0x000000FF { 0xFFFFFFFF } else { 0x00000088 }, &key);
    }
    sy += 4 * 44 + 8;

    fill(SIDEBAR_X + 4, sy, SIDEBAR_W - 8, 1, C_BORDER);
    sy += 8;

    // Current color swatch
    text(sx, sy, C_HINT, "ACTIVE COLOR");
    sy += 18;
    fill(sx, sy, 48, 24, c.color());
    border(sx - 1, sy - 1, 50, 26, C_BORDER);
    sy += 32;

    // Info
    text(sx, sy, C_HINT, &format!("Cursor: {},{}", c.cur_x, c.cur_y));
    sy += 16;
    let dm = if c.draw_mode { "[D] Draw ON " } else { "[D] Draw OFF" };
    text(sx, sy, if c.draw_mode { C_GREEN } else { C_HINT }, dm);
    sy += 16;
    if c.line_start.is_some() {
        text(sx, sy, C_ORANGE, "LINE: set start");
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    if !c.status.is_empty() {
        text(8, sb_y + 6, C_HINT, &c.status);
    } else {
        text(8, sb_y + 6, C_HINT, &format!("Tool: {}  Color: {}  Undo: {} levels", c.tool.name(), c.color_idx, c.undo_stack.len()));
    }
    flush();
}

// Make C_SEL_BG accessible
const C_SEL_BG: u32 = 0x1C2D4EFF;

fn main() {
    let stdin = io::stdin();
    let mut canvas = Canvas::new();

    println!("@supervisor: raise draw2");
    let _ = io::stdout().flush();
    draw(&canvas);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }
        canvas.status.clear();

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            // Navigation
            "\x1b[A" => {
                if canvas.cur_y > 0 { canvas.cur_y -= 1; }
                if canvas.draw_mode && canvas.tool == Tool::Brush {
                    canvas.pixels[Canvas::idx(canvas.cur_x, canvas.cur_y)] = canvas.color();
                }
            }
            "\x1b[B" => {
                if canvas.cur_y + 1 < CANVAS_H { canvas.cur_y += 1; }
                if canvas.draw_mode && canvas.tool == Tool::Brush {
                    canvas.pixels[Canvas::idx(canvas.cur_x, canvas.cur_y)] = canvas.color();
                }
            }
            "\x1b[C" => {
                if canvas.cur_x + 1 < CANVAS_W { canvas.cur_x += 1; }
                if canvas.draw_mode && canvas.tool == Tool::Brush {
                    canvas.pixels[Canvas::idx(canvas.cur_x, canvas.cur_y)] = canvas.color();
                }
            }
            "\x1b[D" => {
                if canvas.cur_x > 0 { canvas.cur_x -= 1; }
                if canvas.draw_mode && canvas.tool == Tool::Brush {
                    canvas.pixels[Canvas::idx(canvas.cur_x, canvas.cur_y)] = canvas.color();
                }
            }
            // Apply tool
            " " => { canvas.apply_tool(); }
            // Tool select
            "b" | "B" => { canvas.tool = Tool::Brush; canvas.line_start = None; }
            "e" | "E" => { canvas.tool = Tool::Eraser; canvas.line_start = None; }
            "f" | "F" => { canvas.tool = Tool::Fill; canvas.line_start = None; }
            "l" | "L" => { canvas.tool = Tool::Line; canvas.line_start = None; }
            // Draw mode toggle
            "d" | "D" => { canvas.draw_mode = !canvas.draw_mode; }
            // Color select 1-9 → palette 0-8
            "1"=>{canvas.color_idx=0;} "2"=>{canvas.color_idx=1;} "3"=>{canvas.color_idx=2;}
            "4"=>{canvas.color_idx=3;} "5"=>{canvas.color_idx=4;} "6"=>{canvas.color_idx=5;}
            "7"=>{canvas.color_idx=6;} "8"=>{canvas.color_idx=7;} "9"=>{canvas.color_idx=8;}
            // Color select a,c,g,h,i,j,k → palette 9-15 (b/d/e/f taken by tools)
            "a"=>{canvas.color_idx=9;}  "c"=>{canvas.color_idx=10;}
            "g"=>{canvas.color_idx=11;} "h"=>{canvas.color_idx=12;}
            "i"=>{canvas.color_idx=13;} "j"=>{canvas.color_idx=14;} "k"=>{canvas.color_idx=15;}
            // Undo
            "\x1a" => { // Ctrl+Z
                if let Some(prev) = canvas.undo_stack.pop() {
                    canvas.pixels = prev;
                    canvas.status = "Undo".to_string();
                } else {
                    canvas.status = "Nothing to undo".to_string();
                }
            }
            // Clear
            "\x18" => { // Ctrl+X
                canvas.push_undo();
                canvas.pixels = vec![0xFFFFFFFF; CANVAS_W * CANVAS_H];
                canvas.status = "Canvas cleared".to_string();
            }
            // Save
            "\x17" => { // Ctrl+W
                match canvas.save_ppm() {
                    Ok(()) => canvas.status = "Saved /data/draw2.ppm".to_string(),
                    Err(e) => canvas.status = format!("Save error: {}", e),
                }
            }
            // Load
            "\x0c" => { // Ctrl+L
                match canvas.load_ppm() {
                    Ok(()) => canvas.status = "Loaded /data/draw2.ppm".to_string(),
                    Err(e) => canvas.status = format!("Load error: {}", e),
                }
            }
            _ => {}
        }
        draw(&canvas);
    }
}
