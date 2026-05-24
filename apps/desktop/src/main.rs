// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1440;
const H: u32 = 832;
const C_BG: u32      = 0x0D1117FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_DIM: u32     = 0x8B949EFF;
const C_SEL: u32     = 0x58A6FFFF;
const C_HINT: u32    = 0x6E7681FF;

const ICON_W: u32    = 80;
const ICON_H: u32    = 80;
const LABEL_H: u32   = 18;
const CELL_W: u32    = 100;
const CELL_H: u32    = ICON_H + LABEL_H + 8;
const COLS: u32      = 12;
const START_X: u32   = 20;
const START_Y: u32   = 20;

// Fixed icons shown when filesystem ls isn't available
const FIXED_ITEMS: &[(&str, &str, u32)] = &[
    ("Desktop",    "folder",  0x58A6FFFF),
    ("Downloads",  "folder",  0x3FB950FF),
    ("Documents",  "folder",  0xFFA657FF),
    ("Pictures",   "folder",  0xF78166FF),
    ("Music",      "folder",  0xBC8CFFFF),
    ("Videos",     "folder",  0xFF7B72FF),
    ("Trash",      "trash",   0x8B949EFF),
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

struct Item {
    name:  String,
    kind:  String, // "folder", "file", "trash"
    color: u32,
}

fn ext_color(name: &str) -> u32 {
    if name.ends_with(".md")   { return 0x79C0FFFF; }
    if name.ends_with(".json") { return 0xFFA657FF; }
    if name.ends_with(".csv")  { return 0x3FB950FF; }
    if name.ends_with(".ppm")  { return 0xF78166FF; }
    if name.ends_with(".log")  { return 0x8B949EFF; }
    if name.ends_with(".toml") { return 0xBC8CFFFF; }
    0x58A6FFFF
}

fn parse_ls(reply: &str) -> Vec<Item> {
    let payload = reply.trim_start_matches("REPLY:ls-data ");
    if payload.is_empty() || payload == "REPLY:ls-data" {
        return Vec::new();
    }
    payload.split_whitespace()
        .map(|name| {
            let color = ext_color(name);
            Item { name: name.to_string(), kind: "file".to_string(), color }
        })
        .collect()
}

fn icon_x(col: u32) -> u32 { START_X + col * CELL_W }
fn icon_y(row: u32) -> u32 { START_Y + row * CELL_H }

fn draw_icon(x: u32, y: u32, kind: &str, color: u32, selected: bool) {
    let bg = if selected { 0x1F4068FF } else { 0x161B22FF };
    fill(x, y, ICON_W, ICON_H, bg);
    let bc = if selected { C_SEL } else { C_BORDER };
    border(x, y, ICON_W, ICON_H, bc);

    match kind {
        "folder" => {
            // Folder tab
            fill(x + 8, y + 18, 30, 8, color);
            // Folder body
            fill(x + 8, y + 24, ICON_W - 16, ICON_H - 36, color);
        }
        "trash" => {
            // Trash bin shape
            fill(x + 20, y + 18, ICON_W - 40, 6, color);
            fill(x + 24, y + 24, ICON_W - 48, ICON_H - 42, color);
            border(x + 24, y + 24, ICON_W - 48, ICON_H - 42, 0x0D1117FF);
        }
        _ => {
            // Generic file
            fill(x + 16, y + 14, ICON_W - 32, ICON_H - 28, color);
            // Dog-ear
            fill(x + ICON_W - 28, y + 14, 12, 12, 0x0D1117FF);
            fill(x + ICON_W - 32, y + 14, 16, 16, bg);
            border(x + ICON_W - 32, y + 14, 16, 16, bc);
        }
    }
}

fn draw(items: &[Item], cursor: usize, input_mode: bool, input_buf: &str) {
    fill(0, 0, W, H, C_BG);

    if items.is_empty() {
        text(20, 20, C_HINT, "Desktop  —  no files in /data");
    }

    for (i, item) in items.iter().enumerate() {
        let col = i as u32 % COLS;
        let row = i as u32 / COLS;
        let ix = icon_x(col);
        let iy = icon_y(row);
        let is_sel = i == cursor;

        draw_icon(ix, iy, &item.kind, item.color, is_sel);

        // Label below icon
        let label: String = item.name.chars().take(10).collect();
        let lw = label.len() as u32 * 8;
        let lx = ix + (ICON_W - lw.min(ICON_W)) / 2;
        let tc = if is_sel { C_TEXT } else { C_DIM };
        text(lx, iy + ICON_H + 4, tc, &label);
    }

    // Status bar
    fill(0, H - 28, W, 28, 0x161B22FF);
    fill(0, H - 28, W, 1, C_BORDER);
    let count = format!("{} items", items.len());
    text(20, H - 20, C_HINT, &count);
    text(W - 360, H - 20, C_HINT, "↑↓←→: navigate  Enter: open  n: new  Esc: quit");

    // New file input
    if input_mode {
        fill(0, H - 60, W, 32, 0x21262DFF);
        fill(0, H - 60, W, 1, C_SEL);
        let prompt = format!("New file: {}_", input_buf);
        text(20, H - 52, C_TEXT, &prompt);
    }

    flush();
}

fn open_file(name: &str) {
    let app = if name.ends_with(".md") { "markdown-viewer" }
        else if name.ends_with(".json") { "json-viewer" }
        else if name.ends_with(".csv") { "csv-viewer" }
        else if name.ends_with(".ppm") { "image-viewer" }
        else if name.ends_with(".log") { "log-viewer" }
        else { "text-editor" };
    println!("@supervisor: run {app}");
    let _ = io::stdout().flush();
}

fn build_items(data_files: &[Item]) -> Vec<Item> {
    let mut result: Vec<Item> = FIXED_ITEMS.iter().map(|(name, kind, color)| {
        Item { name: name.to_string(), kind: kind.to_string(), color: *color }
    }).collect();
    for f in data_files {
        result.push(Item { name: f.name.clone(), kind: f.kind.clone(), color: f.color });
    }
    result
}

fn main() {
    let stdin = io::stdin();
    let mut data_files: Vec<Item> = Vec::new();
    let mut cursor = 0usize;
    let mut input_mode = false;
    let mut input_buf = String::new();

    println!("@supervisor: ls-data");
    println!("@supervisor: raise desktop");
    let _ = io::stdout().flush();

    let items = build_items(&data_files);
    draw(&items, cursor, input_mode, &input_buf);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:ls-data") {
            data_files = parse_ls(&raw);
            let items = build_items(&data_files);
            if cursor >= items.len() { cursor = items.len().saturating_sub(1); }
            draw(&items, cursor, input_mode, &input_buf);
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        let items = build_items(&data_files);
        let n = items.len();

        if input_mode {
            match raw.as_str() {
                "\x1b" | "\x03" => {
                    input_mode = false;
                    input_buf.clear();
                }
                "" => {
                    if !input_buf.is_empty() {
                        println!("@supervisor: touch /data/{input_buf}");
                        let _ = io::stdout().flush();
                        // Re-query after create
                        println!("@supervisor: ls-data");
                        let _ = io::stdout().flush();
                        input_mode = false;
                        input_buf.clear();
                    }
                }
                "\x7f" => { input_buf.pop(); }
                ch if ch.len() == 1 => {
                    let c = ch.chars().next().unwrap();
                    if c.is_ascii_graphic() || c == ' ' { input_buf.push(c); }
                }
                _ => {}
            }
            let items = build_items(&data_files);
            draw(&items, cursor, input_mode, &input_buf);
            continue;
        }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if cursor >= COLS as usize { cursor -= COLS as usize; }
                draw(&items, cursor, input_mode, &input_buf);
            }
            "\x1b[B" => {
                if cursor + (COLS as usize) < n { cursor += COLS as usize; }
                draw(&items, cursor, input_mode, &input_buf);
            }
            "\x1b[D" => {
                if cursor > 0 { cursor -= 1; }
                draw(&items, cursor, input_mode, &input_buf);
            }
            "\x1b[C" => {
                if cursor + 1 < n { cursor += 1; }
                draw(&items, cursor, input_mode, &input_buf);
            }
            "" => {
                if let Some(item) = items.get(cursor) {
                    open_file(&item.name);
                }
                draw(&items, cursor, input_mode, &input_buf);
            }
            "n" | "N" => {
                input_mode = true;
                input_buf.clear();
                draw(&items, cursor, input_mode, &input_buf);
            }
            _ => {}
        }
    }
}
