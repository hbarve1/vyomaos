// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::Write;

// -- Default screen dimensions (fallback if VYOMA_SYSTEM:screen: never arrives) --

pub const DEFAULT_SW: u32 = 1440;
pub const DEFAULT_SH: u32 = 900;

// -- Panel geometry (local window coords; window declared at y=440 in vyoma.toml) --

pub const PX: u32 = 24;       // panel left edge (horizontal margin within window)
pub const PY: u32 = 10;       // panel top edge in LOCAL window coords
pub const PH: u32 = 420;      // panel height

const TITLE_H: u32 = 24;                    // title bar height
const TAB_BAR_H: u32 = 20;                  // tab bar height
const INNER_X: u32 = PX + 8;               // content left margin
const LINE_H: u32  = 16;                    // glyph height

// -- Colours --

const C_PANEL:      u32 = 0x161B22FF; // panel background
const C_TITLE:      u32 = 0x21262DFF; // title bar
const C_ACCENT:     u32 = 0x58A6FFFF; // blue accent
const C_WHITE:      u32 = 0xFFFFFFFF;
const C_DIM:        u32 = 0x8B949EFF; // dimmed text
const C_GREEN:      u32 = 0x3FB950FF;
const C_PROMPT:     u32 = 0x58A6FFFF; // prompt colour
const C_TAB_BG:     u32 = 0x0D1117FF; // inactive tab background
const C_TAB_ACTIVE: u32 = 0x161B22FF; // active tab (matches panel)
const C_TAB_BORDER: u32 = 0x30363DFF; // tab separator

/// Render the full shell panel including tab bar.
///
/// `tabs` carries per-tab metadata (only `cwd` is read here for the label);
/// `lines`, `input`, `cursor_pos` are from the active tab.
pub fn draw_shell(
    tabs: &[crate::Tab],
    active: usize,
    lines: &[String],
    input: &str,
    cursor_pos: usize,
    sw: u32,
    _sh: u32,
) {
    let pw = sw.saturating_sub(PX * 2);
    let prompt_y = PY + PH - 22;
    let inner_y = PY + TITLE_H + TAB_BAR_H + 6;

    // Panel background + border
    fill(PX, PY, pw, PH, C_PANEL);

    // Title bar
    fill(PX, PY, pw, TITLE_H, C_TITLE);
    text(PX + 8, PY + 4, C_ACCENT, "shell");
    if pw >= 136 { text(PX + pw - 136, PY + 4, C_DIM, "VyomaOS v0.1"); }

    // Tab bar (below title bar)
    let tab_bar_y = PY + TITLE_H;
    fill(PX, tab_bar_y, pw, TAB_BAR_H, C_TAB_BG);

    draw_tab_bar(tabs, active, tab_bar_y, pw);

    // Accent line below tab bar
    fill(PX, tab_bar_y + TAB_BAR_H, pw, 2, C_ACCENT);
    border(PX, PY, pw, PH, C_ACCENT);

    // Clear output area
    clear_region(INNER_X, inner_y, pw.saturating_sub(16), prompt_y - inner_y);

    // Output lines -- each logical line is soft-wrapped at 90 chars
    let mut ly = inner_y;
    'outer: for line in lines.iter() {
        let colour = if line.starts_with("> ") { C_DIM } else { C_WHITE };
        for wrapped in super::commands::wrap_line(line, 90) {
            if ly + LINE_H > prompt_y { break 'outer; }
            text(INNER_X, ly, colour, &wrapped);
            ly += LINE_H;
        }
    }

    // Prompt + cursor
    fill(PX, prompt_y - 2, pw, 2, C_TAB_BORDER); // separator
    text(INNER_X, prompt_y, C_PROMPT, "> ");
    if !input.is_empty() {
        text(INNER_X + 16, prompt_y, C_WHITE, input);
    }
    let cursor_x = INNER_X + 16 + cursor_pos as u32 * 8;
    fill(cursor_x, prompt_y, 8, 14, C_GREEN);

    flush();
}

/// Draw the tab bar: `[1:~] [2:~] ...` with the active tab highlighted.
fn draw_tab_bar(tabs: &[crate::Tab], active: usize, y: u32, _pw: u32) {
    let tab_w: u32 = 80; // width per tab label area
    let pad: u32 = 4;

    for (i, tab) in tabs.iter().enumerate() {
        let tx = PX + pad + i as u32 * (tab_w + pad);
        let bg = if i == active { C_TAB_ACTIVE } else { C_TAB_BG };
        let fg = if i == active { C_ACCENT } else { C_DIM };

        fill(tx, y + 2, tab_w, TAB_BAR_H - 4, bg);
        // Label: "N:cwd" (truncated to fit)
        let label = format!("{}:{}", i + 1, truncate_cwd(&tab.cwd, 7));
        text(tx + 4, y + 3, fg, &label);

        // Separator after each tab except the last
        if i + 1 < tabs.len() {
            fill(tx + tab_w, y + 4, 1, TAB_BAR_H - 8, C_TAB_BORDER);
        }
    }
}

/// Truncate a cwd string to at most `max` chars, adding ".." if needed.
fn truncate_cwd(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}..", &s[..max.saturating_sub(2)])
    }
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
    // Pipe stdout is block-buffered -- must flush explicitly so VYOMA_DRAW
    // commands reach the supervisor without waiting for the buffer to fill.
    let _ = std::io::stdout().flush();
}
