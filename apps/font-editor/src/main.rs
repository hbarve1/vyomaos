// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1000;
const H: u32 = 760;
const HDR: u32 = 40;
const SB_H: u32 = 28;
const CELL: u32 = 24;           // px per glyph cell
const GRID_X: u32 = 20;
const GRID_Y: u32 = HDR + 60;
const GRID_W: u32 = CELL * 8;   // 192px
const GRID_H: u32 = CELL * 16;  // 384px
const LEFT_W: u32 = GRID_X + GRID_W + 24;  // ~240
const RIGHT_X: u32 = LEFT_W;
const RIGHT_W: u32 = W - RIGHT_X;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
const C_CARD: u32   = 0x161B22FF;
const C_PIXEL_ON: u32  = 0xE6EDF3FF;
const C_PIXEL_OFF: u32 = 0x161B22FF;
const C_GRID: u32      = 0x21262DFF;

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

fn get_pixel(glyph: &[u8; 16], col: usize, row: usize) -> bool {
    (glyph[row] >> (7 - col)) & 1 == 1
}

fn toggle_pixel(glyph: &mut [u8; 16], col: usize, row: usize) {
    glyph[row] ^= 1 << (7 - col);
}

struct App {
    glyphs:  Vec<[u8; 16]>,
    cur:     usize,   // which ASCII char (0 = 0x20 space, 94 = 0x7E tilde)
    cx:      usize,   // cursor col 0..8
    cy:      usize,   // cursor row 0..16
    preview_scale: u32,
    status:  String,
}

impl App {
    fn new() -> Self {
        App {
            glyphs: vec![[0u8; 16]; 95],
            cur: 0,
            cx: 0,
            cy: 0,
            preview_scale: 2,
            status: String::from("Arrows=move  Space=toggle  ←→=char  Tab=scale  C=hex  R=reset  Ctrl+C=quit"),
        }
    }

    fn char_at(&self, i: usize) -> char {
        (0x20u8 + i as u8) as char
    }

    fn hex_row(&self) -> String {
        self.glyphs[self.cur].iter()
            .map(|b| format!("{:02X}", b))
            .collect::<Vec<_>>()
            .join(",")
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HDR, C_HEADER);
    fill(0, HDR - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Pixel Font Editor");
    let cur_char = app.char_at(app.cur);
    let display = if cur_char == ' ' { "SPC".to_string() } else { cur_char.to_string() };
    text(220, 12, C_TEXT, &format!("Char: '{}' (0x{:02X})  #{}/95",
        display, 0x20 + app.cur as u8, app.cur + 1));
    text(500, 12, C_HINT, &format!("cursor ({},{})", app.cx, app.cy));

    // Left panel: glyph editor
    fill(0, HDR, LEFT_W, H - HDR, C_BG);

    // Char display above grid
    text(GRID_X, HDR + 8, C_HINT, "Editing:");
    text(GRID_X + 72, HDR + 8, C_SEL, &format!("'{}' (ASCII {})", display, 0x20 + app.cur));
    text(GRID_X, HDR + 26, C_HINT, &format!("← prev  → next  Space=toggle pixel"));

    // Grid background
    fill(GRID_X - 1, GRID_Y - 1, GRID_W + 2, GRID_H + 2, C_BORDER);

    for row in 0..16usize {
        for col in 0..8usize {
            let px = GRID_X + col as u32 * CELL;
            let py = GRID_Y + row as u32 * CELL;
            let on = get_pixel(&app.glyphs[app.cur], col, row);
            let is_cur = col == app.cx && row == app.cy;

            let bg = if on { C_PIXEL_ON } else { C_PIXEL_OFF };
            fill(px, py, CELL - 1, CELL - 1, bg);

            // Grid lines
            fill(px + CELL - 1, py, 1, CELL - 1, C_GRID);
            fill(px, py + CELL - 1, CELL, 1, C_GRID);

            // Cursor highlight
            if is_cur {
                border(px, py, CELL - 1, CELL - 1, C_SEL);
            }
        }
    }

    // Row numbers on right of grid
    for row in 0..16usize {
        let py = GRID_Y + row as u32 * CELL + 4;
        text(GRID_X + GRID_W + 4, py, C_HINT, &format!("{:2}", row));
    }
    // Col numbers above grid
    for col in 0..8usize {
        let px = GRID_X + col as u32 * CELL + 8;
        text(px, GRID_Y - 14, C_HINT, &format!("{}", col));
    }

    // Divider
    fill(LEFT_W - 1, HDR, 1, H - HDR, C_BORDER);

    // Right panel
    fill(RIGHT_X, HDR, RIGHT_W, H - HDR - SB_H, C_CARD);

    // Preview section
    let preview_y = HDR + 8;
    text(RIGHT_X + 8, preview_y, C_HINT, &format!("Preview ({}×):", app.preview_scale));

    let scales = [1u32, 2, 4];
    let mut px_offset = RIGHT_X + 8;
    for &scale in &scales {
        let is_cur_scale = scale == app.preview_scale;
        let py_start = preview_y + 16;
        let pw = scale * 8;
        let ph = scale * 16;
        fill(px_offset - 2, py_start - 2, pw + 4, ph + 4, if is_cur_scale { C_HEADER } else { C_BG });
        if is_cur_scale { border(px_offset - 2, py_start - 2, pw + 4, ph + 4, C_SEL); }

        for row in 0..16usize {
            for col in 0..8usize {
                let on = get_pixel(&app.glyphs[app.cur], col, row);
                let c = if on { C_PIXEL_ON } else { C_PIXEL_OFF };
                fill(px_offset + col as u32 * scale, py_start + row as u32 * scale, scale, scale, c);
            }
        }

        // Scale label
        text(px_offset, py_start + ph + 4, C_HINT, &format!("{}×", scale));
        px_offset += pw + 16;
    }

    // Character browser
    let browser_y = HDR + 120;
    text(RIGHT_X + 8, browser_y, C_HINT, "All 95 characters (click ←→ to navigate):");

    let cell_w = 32u32;
    let cell_h = 20u32;
    let cols = 10u32;
    for i in 0..95usize {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;
        let bx = RIGHT_X + 8 + col * cell_w;
        let by = browser_y + 16 + row * cell_h;

        let is_cur = i == app.cur;
        if is_cur {
            fill(bx - 1, by - 1, cell_w, cell_h, C_HEADER);
            border(bx - 1, by - 1, cell_w, cell_h, C_SEL);
        }

        // Show glyph density as brightness
        let glyph = &app.glyphs[i];
        let bits: u32 = glyph.iter().map(|b| b.count_ones()).sum();
        let density_col = if bits == 0 {
            C_HINT
        } else if is_cur {
            C_TEXT
        } else {
            C_SEL
        };

        let ch = (0x20u8 + i as u8) as char;
        let ch_str = if ch == ' ' { "·".to_string() } else { ch.to_string() };
        text(bx + 4, by + 2, density_col, &ch_str);

        // Tiny glyph preview (2px per pixel)
        if bits > 0 {
            for pr in 0..8usize {  // only top 8 rows for tiny preview
                let row_bits = glyph[pr * 2]; // every other row
                for pc in 0..4usize {  // only 4 leftmost columns
                    if (row_bits >> (7 - pc)) & 1 == 1 {
                        fill(bx + 16 + pc as u32 * 2, by + 2 + pr as u32 * 2, 2, 2, density_col);
                    }
                }
            }
        }
    }

    // Hex output area
    let hex_y = browser_y + 16 + 10 * cell_h + 12;
    if hex_y + 40 < H - SB_H {
        text(RIGHT_X + 8, hex_y, C_HINT, "Hex (C=copy):  [row bytes, MSB=leftmost]");
        let hex = app.hex_row();
        let max_chars = (RIGHT_W / 8 - 2) as usize;
        let shown = if hex.len() > max_chars { &hex[..max_chars] } else { &hex };
        text(RIGHT_X + 8, hex_y + 16, C_YELLOW, shown);
    }

    // Status bar
    let sb_y = H - SB_H;
    fill(0, sb_y, W, SB_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise font-editor");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

            // Move edit cursor
            "\x1b[A" => { if app.cy > 0  { app.cy -= 1; } else { app.cy = 15; } }
            "\x1b[B" => { if app.cy < 15 { app.cy += 1; } else { app.cy = 0; } }
            "\x1b[D" => {
                // Left: move cursor left, or go to prev glyph at col 7
                if app.cx > 0 { app.cx -= 1; }
                else if app.cur > 0 { app.cur -= 1; app.cx = 7; }
            }
            "\x1b[C" => {
                // Right: move cursor right, or go to next glyph at col 0
                if app.cx < 7 { app.cx += 1; }
                else if app.cur < 94 { app.cur += 1; app.cx = 0; }
            }

            // Toggle pixel under cursor
            " " => {
                toggle_pixel(&mut app.glyphs[app.cur], app.cx, app.cy);
            }

            // Previous / next glyph
            "\x1b[5~" => {
                if app.cur > 0 { app.cur -= 1; }
                app.status = format!("Char: '{}' (0x{:02X})", app.char_at(app.cur), 0x20+app.cur as u8);
            }
            "\x1b[6~" => {
                if app.cur < 94 { app.cur += 1; }
                app.status = format!("Char: '{}' (0x{:02X})", app.char_at(app.cur), 0x20+app.cur as u8);
            }

            // Cycle preview scale
            "\t" => {
                app.preview_scale = match app.preview_scale { 1 => 2, 2 => 4, _ => 1 };
            }

            // Copy hex
            "c" | "C" => {
                let hex = app.hex_row();
                println!("@supervisor: clipboard-set {}", hex);
                let _ = io::stdout().flush();
                app.status = format!("Hex copied: {}", &hex[..hex.len().min(40)]);
            }

            // Reset glyph
            "r" | "R" => {
                app.glyphs[app.cur] = [0u8; 16];
                app.status = format!("Glyph '{}' cleared.", app.char_at(app.cur));
            }

            // Direct char navigation: type a char to jump to it
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b <= 0x7E {
                    app.cur = (b - 0x20) as usize;
                    app.status = format!("Jumped to '{}' (0x{:02X})", b as char, b);
                }
            }

            _ => {}
        }

        draw(&app);
    }
}
