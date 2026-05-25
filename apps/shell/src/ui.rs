// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::Write;

// ── Panel geometry (local window coords; window declared at y=440 in vyoma.toml) ──

pub const PX: u32 = 24;       // panel left edge (horizontal margin within window)
pub const PY: u32 = 10;       // panel top edge in LOCAL window coords (window.y=440)
pub const PW: u32 = 1392;     // panel width  (1440 - 24*2)
pub const PH: u32 = 420;      // panel height

const TITLE_H: u32 = 24;                    // title bar height
const INNER_X: u32 = PX + 8;               // content left margin
const INNER_Y: u32 = PY + TITLE_H + 6;     // content top
const LINE_H: u32  = 16;                    // glyph height
const PROMPT_Y: u32 = PY + PH - 22;        // prompt line y position

// ── Colours ───────────────────────────────────────────────────────────────────

const C_PANEL:   u32 = 0x161B22FF; // panel background
const C_TITLE:   u32 = 0x21262DFF; // title bar
const C_ACCENT:  u32 = 0x58A6FFFF; // blue accent
const C_WHITE:   u32 = 0xFFFFFFFF;
const C_DIM:     u32 = 0x8B949EFF; // dimmed text
const C_GREEN:   u32 = 0x3FB950FF;
const C_PROMPT:  u32 = 0x58A6FFFF; // prompt colour

pub fn draw_panel(lines: &[String], input: &str, cursor_pos: usize) {
    // Panel background + border
    fill(PX, PY, PW, PH, C_PANEL);

    // Title bar
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_ACCENT); // accent line
    text(PX + 8, PY + 4, C_ACCENT, "shell");
    text(PX + PW - 136, PY + 4, C_DIM, "VyomaOS v0.1");
    border(PX, PY, PW, PH, C_ACCENT);

    // Clear output area
    clear_region(INNER_X, INNER_Y, PW - 16, PROMPT_Y - INNER_Y);

    // Output lines — each logical line is soft-wrapped at 90 chars
    let mut ly = INNER_Y;
    'outer: for line in lines.iter() {
        let colour = if line.starts_with("> ") { C_DIM } else { C_WHITE };
        for wrapped in super::commands::wrap_line(line, 90) {
            if ly + LINE_H > PROMPT_Y { break 'outer; }
            text(INNER_X, ly, colour, &wrapped);
            ly += LINE_H;
        }
    }

    // Prompt + cursor
    fill(PX, PROMPT_Y - 2, PW, 2, 0x30363DFF); // separator
    text(INNER_X, PROMPT_Y, C_PROMPT, "> ");
    if !input.is_empty() {
        text(INNER_X + 16, PROMPT_Y, C_WHITE, input);
    }
    let cursor_x = INNER_X + 16 + cursor_pos as u32 * 8;
    fill(cursor_x, PROMPT_Y, 8, 14, C_GREEN);

    flush();
}

#[inline]
pub fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}

#[inline]
pub fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}

#[inline]
pub fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba}");
}

#[inline]
pub fn clear_region(x: u32, y: u32, w: u32, h: u32) {
    println!("VYOMA_DRAW:clear_region:{x},{y},{w},{h}");
}

#[inline]
pub fn flush() {
    println!("VYOMA_DRAW:flush");
    // Pipe stdout is block-buffered — must flush explicitly so VYOMA_DRAW
    // commands reach the supervisor without waiting for the buffer to fill.
    let _ = std::io::stdout().flush();
}
