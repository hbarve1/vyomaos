// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS Desktop — polished app launcher with 24 icons in a 6×4 grid.
//!
//! Mouse clicks hit-test the icon grid and launch apps via `@supervisor: run`.
//! Screen dimensions are received via `VYOMA_SYSTEM:screen:<w>,<h>` at startup
//! and fall back to DEFAULT_SW/SH if the notification never arrives.

use std::io::{self, BufRead, Write};

const DEFAULT_SW: u32 = 1440;
const DEFAULT_SH: u32 = 900;
const DOCK_H:     u32 = 60;

// Color palette
const C_BG2:     u32 = 0x0D1120FF;  // matches gradient midpoint for corner clipping
const C_GRID_BG: u32 = 0x161B2299;
const C_LABEL:   u32 = 0xE6EDF3FF;
const C_DIM:     u32 = 0x8B949EFF;
const C_HOVER:   u32 = 0x1F6FEBFF;
const C_SECTION: u32 = 0x21262DFF;
const C_SEC_TXT: u32 = 0x58A6FFFF;

// Icon grid layout
const COLS:     u32 = 6;
const ICON_W:   u32 = 80;
const ICON_H:   u32 = 80;
const CELL_W:   u32 = 200;
const CELL_H:   u32 = 108;
const GRID_TOP: u32 = 90;
const CORNER:   u32 = 8;

struct AppDef {
    label:  &'static str,
    name:   &'static str,
    symbol: &'static str,
    color:  u32,
    icon:   Option<&'static str>,
}

const APPS: &[AppDef] = &[
    // Productivity
    AppDef { label: "Notes",     name: "notes",         symbol: "N",  color: 0x1A6B4AFF, icon: Some("/apps/notes/icon.png") },
    AppDef { label: "Text Edit", name: "text-editor",   symbol: "Te", color: 0x1C4F8AFF, icon: None },
    AppDef { label: "Calendar",  name: "calendar",      symbol: "Ca", color: 0x6B2A8AFF, icon: None },
    AppDef { label: "Kanban",    name: "kanban",        symbol: "Kb", color: 0x8A4B1AFF, icon: None },
    AppDef { label: "Sheets",    name: "spreadsheet",   symbol: "Sh", color: 0x1A7A3AFF, icon: None },
    AppDef { label: "Tasks",     name: "task-manager",  symbol: "Tk", color: 0x4A2A8AFF, icon: None },
    // Media
    AppDef { label: "Music",     name: "music-player",  symbol: "Mu", color: 0x8A1A4AFF, icon: None },
    AppDef { label: "Photos",    name: "photo-editor",  symbol: "Ph", color: 0x1A4A8AFF, icon: None },
    AppDef { label: "Paint",     name: "paint",         symbol: "Pa", color: 0x7A2A1AFF, icon: None },
    AppDef { label: "Fractal",   name: "fractal",       symbol: "Fr", color: 0x2A1A7AFF, icon: None },
    AppDef { label: "Colors",    name: "color-picker",  symbol: "Co", color: 0x7A5A1AFF, icon: None },
    AppDef { label: "Theory",    name: "music-theory",  symbol: "Mt", color: 0x4A6A2AFF, icon: None },
    // Developer
    AppDef { label: "Shell",     name: "shell",         symbol: ">_", color: 0x1A3A1AFF, icon: Some("/apps/shell/icon.png") },
    AppDef { label: "JSON",      name: "json-viewer",   symbol: "Js", color: 0x5A3A1AFF, icon: None },
    AppDef { label: "Hex Edit",  name: "hex-editor",    symbol: "Hx", color: 0x2A4A2AFF, icon: None },
    AppDef { label: "Monitor",   name: "system-monitor",symbol: "Sm", color: 0x1A5A6AFF, icon: None },
    AppDef { label: "Logs",      name: "log-viewer",    symbol: "Lg", color: 0x6A4A1AFF, icon: None },
    AppDef { label: "Settings",  name: "settings",      symbol: "St", color: 0x4A4A4AFF, icon: Some("/apps/settings/icon.png") },
    // Games
    AppDef { label: "Snake",     name: "snake",         symbol: "Sn", color: 0x1A6A1AFF, icon: None },
    AppDef { label: "Tetris",    name: "tetris",        symbol: "Tt", color: 0x6A1A6AFF, icon: None },
    AppDef { label: "Maze",      name: "maze",          symbol: "Mz", color: 0x6A4A1AFF, icon: None },
    AppDef { label: "Chess",     name: "chess",         symbol: "Ch", color: 0x3A3A1AFF, icon: None },
    AppDef { label: "Sudoku",    name: "sudoku",        symbol: "Su", color: 0x1A4A6AFF, icon: None },
    AppDef { label: "Pong",      name: "pong",          symbol: "Po", color: 0x6A1A3AFF, icon: None },
];

#[inline]
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}
#[inline]
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}
#[inline]
fn text_l(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},l,{s}");
}
#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn grid_left(sw: u32) -> u32 {
    let total = COLS * CELL_W;
    if sw > total { (sw - total) / 2 } else { 0 }
}

fn cell_pos(idx: usize, sw: u32) -> (u32, u32) {
    let col = idx as u32 % COLS;
    let row = idx as u32 / COLS;
    (grid_left(sw) + col * CELL_W, GRID_TOP + row * CELL_H)
}

fn icon_xy(cx: u32, cy: u32) -> (u32, u32) {
    (cx + (CELL_W - ICON_W) / 2, cy + 4)
}

/// Interpolate between dark navy colors as t goes 0..=255
fn gradient_color(t: u8) -> u32 {
    // Color stops: deep space (top) → midnight blue → deep indigo (bottom)
    let (r, g, b) = match t {
        0..=63   => lerp_rgb((0x06, 0x0A, 0x14), (0x0D, 0x11, 0x20), t, 63),
        64..=127 => lerp_rgb((0x0D, 0x11, 0x20), (0x0E, 0x0F, 0x2A), t - 64, 63),
        128..=191 => lerp_rgb((0x0E, 0x0F, 0x2A), (0x0A, 0x0C, 0x1E), t - 128, 63),
        _        => lerp_rgb((0x0A, 0x0C, 0x1E), (0x06, 0x08, 0x12), t - 192, 63),
    };
    (r as u32) << 24 | (g as u32) << 16 | (b as u32) << 8 | 0xFF
}

fn lerp_rgb(from: (u8, u8, u8), to: (u8, u8, u8), t: u8, max: u8) -> (u8, u8, u8) {
    let lerp = |a: u8, b: u8| -> u8 {
        let a = a as i32;
        let b = b as i32;
        let t = t as i32;
        let m = max as i32;
        (a + (b - a) * t / m).clamp(0, 255) as u8
    };
    (lerp(from.0, to.0), lerp(from.1, to.1), lerp(from.2, to.2))
}

/// Blend an aurora tint color at given alpha over the gradient background
fn aurora_tint(y: u32, content_h: u32, alpha: u8, ar: u8, ag: u8, ab: u8) -> u32 {
    let t = (y * 255 / content_h.max(1)) as u8;
    let (br, bg, bb) = match t {
        0..=63   => lerp_rgb((0x06, 0x0A, 0x14), (0x0D, 0x11, 0x20), t, 63),
        64..=127 => lerp_rgb((0x0D, 0x11, 0x20), (0x0E, 0x0F, 0x2A), t - 64, 63),
        128..=191 => lerp_rgb((0x0E, 0x0F, 0x2A), (0x0A, 0x0C, 0x1E), t - 128, 63),
        _        => lerp_rgb((0x0A, 0x0C, 0x1E), (0x06, 0x08, 0x12), t - 192, 63),
    };
    // Alpha-blend aurora over background: out = src*a + dst*(1-a)
    let a = alpha as u32;
    let blend = |src: u8, dst: u8| -> u8 {
        let s = src as u32;
        let d = dst as u32;
        ((s * a + d * (255 - a)) / 255) as u8
    };
    let r = blend(ar, br);
    let g = blend(ag, bg);
    let b = blend(ab, bb);
    (r as u32) << 24 | (g as u32) << 16 | (b as u32) << 8 | 0xFF
}

fn draw_wallpaper(sw: u32, sh: u32) {
    let content_h = sh.saturating_sub(DOCK_H);
    let strips = 80u32;
    let strip_h = (content_h + strips - 1) / strips; // ceiling division

    for i in 0..strips {
        let y = i * strip_h;
        let h = strip_h.min(content_h.saturating_sub(y));
        if h == 0 { break; }
        let t = (i * 255 / strips) as u8;
        let color = gradient_color(t);
        fill(0, y, sw, h, color);
    }

    // Aurora band 1: teal glow across upper third
    let a1_y = content_h / 6;
    let a1_h = content_h / 8;
    for i in 0..12u32 {
        let ay = a1_y + i * (a1_h / 12);
        let intensity = if i < 6 { i } else { 11 - i };
        let alpha = (intensity * 18 + 10) as u8;
        let color = aurora_tint(ay, content_h, alpha, 0x00, 0xC8, 0x80);
        fill(0, ay, sw, a1_h / 12 + 1, color);
    }

    // Aurora band 2: purple glow in the middle
    let a2_y = content_h * 2 / 5;
    let a2_h = content_h / 10;
    for i in 0..8u32 {
        let ay = a2_y + i * (a2_h / 8);
        let intensity = if i < 4 { i } else { 7 - i };
        let alpha = (intensity * 14 + 8) as u8;
        let color = aurora_tint(ay, content_h, alpha, 0x80, 0x00, 0xC0);
        fill(0, ay, sw, a2_h / 8 + 1, color);
    }

    // Subtle star field — deterministic positions in upper 40% of wallpaper
    let stars: &[(u32, u32)] = &[
        (120, 30), (340, 55), (680, 20), (890, 70), (1100, 40), (1300, 25),
        (200, 100), (450, 80), (760, 110), (980, 60), (1200, 90), (60, 140),
        (570, 130), (820, 150), (1050, 120), (1380, 80), (300, 180), (640, 170),
        (1150, 160), (400, 200), (750, 190), (1000, 210), (180, 230), (520, 220),
        (870, 240), (1240, 200), (80, 260), (420, 250), (710, 280), (960, 260),
        (1320, 270), (240, 300), (590, 290), (830, 310), (1080, 290), (1410, 300),
        (150, 320), (480, 340), (770, 330), (1200, 320),
    ];
    for &(sx, sy) in stars {
        if sx < sw && sy < content_h {
            fill(sx, sy, 1, 1, 0xCCDDEEFF);
        }
    }
}

fn section_label_y(row: u32) -> u32 {
    GRID_TOP + row * CELL_H - 24
}

fn draw_section(y: u32, sw: u32, label: &str) {
    fill(0, y, sw, 20, C_SECTION);
    text(grid_left(sw) + 8, y + 3, C_SEC_TXT, label);
}

fn draw_icon(cx: u32, cy: u32, app: &AppDef, hover: bool) {
    let (ix, iy) = icon_xy(cx, cy);

    // Body
    fill(ix, iy, ICON_W, ICON_H, app.color);

    // Top-third highlight for depth (blend lighter shade)
    let r = ((app.color >> 24) & 0xFF).saturating_add(0x20) as u32;
    let g = ((app.color >> 16) & 0xFF).saturating_add(0x20) as u32;
    let b = ((app.color >>  8) & 0xFF).saturating_add(0x20) as u32;
    let hi = (r << 24) | (g << 16) | (b << 8) | 0xFF;
    fill(ix, iy, ICON_W, ICON_H / 3, hi);

    // Fake rounded corners — clip with bg
    let bg = C_BG2;
    fill(ix,                   iy,                   CORNER, CORNER, bg);
    fill(ix + ICON_W - CORNER, iy,                   CORNER, CORNER, bg);
    fill(ix,                   iy + ICON_H - CORNER, CORNER, CORNER, bg);
    fill(ix + ICON_W - CORNER, iy + ICON_H - CORNER, CORNER, CORNER, bg);

    // Hover ring
    if hover {
        fill(ix - 2, iy - 2,        ICON_W + 4, 2, C_HOVER);
        fill(ix - 2, iy + ICON_H,   ICON_W + 4, 2, C_HOVER);
        fill(ix - 2, iy,            2, ICON_H,    C_HOVER);
        fill(ix + ICON_W, iy,       2, ICON_H,    C_HOVER);
    }

    // PNG icon or symbol fallback
    if let Some(path) = app.icon {
        println!("VYOMA_DRAW:draw_image:{ix},{iy},{ICON_W},{ICON_H},{path}");
    } else {
        // Symbol centred in icon (16×32 'l' font)
        let sym = app.symbol;
        let sym_w = sym.len() as u32 * 16;
        let sx = ix + ICON_W.saturating_sub(sym_w) / 2;
        let sy = iy + (ICON_H - 32) / 2;
        text_l(sx, sy, 0xFFFFFFFF, sym);
    }

    // Label below icon
    let label = if app.label.len() > 10 { &app.label[..10] } else { app.label };
    let lw = label.len() as u32 * 8;
    let lx = cx + CELL_W.saturating_sub(lw) / 2;
    let ly = iy + ICON_H + 6;
    text(lx, ly, if hover { C_LABEL } else { C_DIM }, label);
}

fn draw(sw: u32, sh: u32, hover_idx: Option<usize>) {
    draw_wallpaper(sw, sh);

    // Section bars (placed above each of the 4 rows)
    draw_section(section_label_y(0), sw, "Productivity");
    draw_section(section_label_y(1), sw, "Media");
    draw_section(section_label_y(2), sw, "Developer");
    draw_section(section_label_y(3), sw, "Games");

    // Subtle grid panel backdrop
    let rows = (APPS.len() as u32 + COLS - 1) / COLS;
    fill(grid_left(sw), GRID_TOP - 4, COLS * CELL_W, rows * CELL_H + 8, C_GRID_BG);

    for (i, app) in APPS.iter().enumerate() {
        let (cx, cy) = cell_pos(i, sw);
        draw_icon(cx, cy, app, hover_idx == Some(i));
    }

    // Status bar at very bottom (above dock)
    let bar_y = sh.saturating_sub(DOCK_H + 20);
    fill(0, bar_y, sw, 20, C_SECTION);
    text(8, bar_y + 3, C_DIM, "Click an icon to launch  |  VyomaOS Desktop");
    flush();
}

fn parse_screen(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("VYOMA_SYSTEM:screen:")?;
    let (ws, hs) = rest.split_once(',')?;
    Some((ws.parse().ok()?, hs.parse().ok()?))
}

fn parse_mouse_pos(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("VYOMA_INPUT:mouse:")?;
    let coords = rest.strip_prefix("move:")
        .or_else(|| rest.strip_prefix("click:"))?;
    let (xs, tail) = coords.split_once(',')?;
    let ys = tail.split(':').next()?;
    Some((xs.parse().ok()?, ys.parse().ok()?))
}

fn hit_test(mx: u32, my: u32, sw: u32) -> Option<usize> {
    for i in 0..APPS.len() {
        let (cx, cy) = cell_pos(i, sw);
        let (ix, iy) = icon_xy(cx, cy);
        if mx >= ix && mx < ix + ICON_W && my >= iy && my < iy + ICON_H {
            return Some(i);
        }
    }
    None
}

fn launch(app: &AppDef) {
    println!("@supervisor: run /apps/{}/vyoma.toml", app.name);
    let _ = io::stdout().flush();
}

fn main() {
    let stdin = io::stdin();
    let mut sw = DEFAULT_SW;
    let mut sh = DEFAULT_SH;
    let mut hover_idx: Option<usize> = None;

    println!("@supervisor: raise desktop");
    let _ = io::stdout().flush();
    draw(sw, sh, hover_idx);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if let Some((w, h)) = parse_screen(&raw) {
            sw = w; sh = h;
            draw(sw, sh, hover_idx);
            continue;
        }

        if raw.starts_with("VYOMA_INPUT:mouse:") {
            let pos = parse_mouse_pos(&raw);
            let new_hover = pos.and_then(|(mx, my)| hit_test(mx, my, sw));
            if raw.contains(":click:") && raw.contains(":left") {
                if let Some(idx) = new_hover { launch(&APPS[idx]); }
            }
            if new_hover != hover_idx {
                hover_idx = new_hover;
                draw(sw, sh, hover_idx);
            }
            continue;
        }

        if raw.starts_with("REPLY:") { continue; }

        if raw == "\x03" || raw == "\x1b" {
            fill(0, 0, sw, sh, 0x0D1117FF);
            flush();
            std::process::exit(0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apps_count() {
        assert_eq!(APPS.len(), 24);
    }

    #[test]
    fn grid_left_centered() {
        assert_eq!(grid_left(1440), (1440 - COLS * CELL_W) / 2);
    }

    #[test]
    fn cell_pos_first() {
        let (cx, cy) = cell_pos(0, 1440);
        assert_eq!(cx, grid_left(1440));
        assert_eq!(cy, GRID_TOP);
    }

    #[test]
    fn cell_pos_second_row() {
        let (_, cy) = cell_pos(COLS as usize, 1440);
        assert_eq!(cy, GRID_TOP + CELL_H);
    }

    #[test]
    fn hit_test_first_icon() {
        let sw = 1440;
        let (cx, cy) = cell_pos(0, sw);
        let (ix, iy) = icon_xy(cx, cy);
        assert_eq!(hit_test(ix + 10, iy + 10, sw), Some(0));
    }

    #[test]
    fn hit_test_below_icon_no_hit() {
        let sw = 1440;
        let (cx, cy) = cell_pos(0, sw);
        let (ix, iy) = icon_xy(cx, cy);
        assert_eq!(hit_test(ix + 10, iy + ICON_H + 10, sw), None);
    }

    #[test]
    fn parse_screen_valid() {
        assert_eq!(parse_screen("VYOMA_SYSTEM:screen:1920,1080"), Some((1920, 1080)));
        assert_eq!(parse_screen("VYOMA_SYSTEM:screen:640,480"), Some((640, 480)));
    }

    #[test]
    fn parse_mouse_move() {
        assert_eq!(parse_mouse_pos("VYOMA_INPUT:mouse:move:100,200"), Some((100, 200)));
    }

    #[test]
    fn parse_mouse_click() {
        assert_eq!(parse_mouse_pos("VYOMA_INPUT:mouse:click:50,75:left"), Some((50, 75)));
    }
}
