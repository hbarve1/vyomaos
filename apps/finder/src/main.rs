// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1280;
const H: u32 = 760;
const SIDEBAR_W: u32 = 220;
const TOOLBAR_H: u32 = 40;
const STATUS_H: u32  = 28;
const MAIN_W: u32    = W - SIDEBAR_W;
const MAIN_H: u32    = H - TOOLBAR_H - STATUS_H;

const C_BG: u32       = 0x161B22FF;
const C_SIDEBAR: u32  = 0x0D1117FF;
const C_BORDER: u32   = 0x30363DFF;
const C_SEL: u32      = 0x58A6FFFF;
const C_SEL_BG: u32   = 0x1F4068FF;
const C_TEXT: u32     = 0xE6EDF3FF;
const C_DIM: u32      = 0x8B949EFF;
const C_HINT: u32     = 0x6E7681FF;
const C_TOOLBAR: u32  = 0x21262DFF;

const ICON_W: u32    = 80;
const ICON_H: u32    = 80;
const CELL_W: u32    = 110;
const CELL_H: u32    = ICON_H + 20;
const GRID_COLS: u32 = (MAIN_W - 20) / CELL_W;
const GRID_START_X: u32 = SIDEBAR_W + 10;
const GRID_START_Y: u32 = TOOLBAR_H + 10;

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

const SIDEBAR_ITEMS: &[(&str, &str, u32)] = &[
    ("Desktop",    "desktop",   0x58A6FFFF),
    ("Downloads",  "downloads", 0x3FB950FF),
    ("Documents",  "documents", 0xFFA657FF),
    ("Pictures",   "pictures",  0xF78166FF),
    ("Music",      "music",     0xBC8CFFFF),
    ("Trash",      "trash",     0x8B949EFF),
];

struct FsItem {
    name:  String,
    kind:  String,
    color: u32,
}

fn ext_color(name: &str) -> u32 {
    if name.ends_with(".md")   { return 0x79C0FFFF; }
    if name.ends_with(".json") { return 0xFFA657FF; }
    if name.ends_with(".csv")  { return 0x3FB950FF; }
    if name.ends_with(".ppm")  { return 0xF78166FF; }
    if name.ends_with(".log")  { return 0x8B949EFF; }
    if name.ends_with(".toml") { return 0xBC8CFFFF; }
    if name.ends_with(".enc")  { return 0xFF7B72FF; }
    0x58A6FFFF
}

fn parse_ls(reply: &str) -> Vec<FsItem> {
    let payload = reply.trim_start_matches("REPLY:ls-data ");
    if payload.is_empty() || payload == "REPLY:ls-data" {
        return Vec::new();
    }
    payload.split_whitespace()
        .map(|name| {
            let color = ext_color(name);
            FsItem { name: name.to_string(), kind: "file".to_string(), color }
        })
        .collect()
}

fn location_for_sidebar(idx: usize) -> &'static str {
    match idx {
        0 => "Desktop",
        1 => "Downloads (/data)",
        2 => "Documents (/data)",
        3 => "Pictures (/data)",
        4 => "Music (/data)",
        5 => "Trash",
        _ => "Desktop",
    }
}

fn items_for_sidebar(idx: usize, data_files: &[FsItem]) -> Vec<FsItem> {
    match idx {
        0 => vec![
            FsItem { name: "data".to_string(), kind: "folder".to_string(), color: 0x58A6FFFF },
            FsItem { name: "settings.toml".to_string(), kind: "file".to_string(), color: 0xBC8CFFFF },
        ],
        5 => vec![], // Trash is empty
        _ => data_files.iter().map(|f| FsItem { name: f.name.clone(), kind: f.kind.clone(), color: f.color }).collect(),
    }
}

fn draw_file_icon(x: u32, y: u32, kind: &str, color: u32, selected: bool) {
    let bg = if selected { C_SEL_BG } else { 0x21262DFF };
    fill(x, y, ICON_W, ICON_H, bg);
    let bc = if selected { C_SEL } else { C_BORDER };
    border(x, y, ICON_W, ICON_H, bc);

    match kind {
        "folder" => {
            fill(x + 8, y + 22, 28, 6, color);
            fill(x + 8, y + 26, ICON_W - 16, ICON_H - 40, color);
        }
        "trash" => {
            fill(x + 20, y + 20, ICON_W - 40, 4, color);
            fill(x + 24, y + 24, ICON_W - 48, ICON_H - 44, color);
        }
        _ => {
            // File page
            fill(x + 14, y + 16, ICON_W - 28, ICON_H - 32, color);
            // Dog-ear
            fill(x + ICON_W - 28, y + 16, 14, 14, bg);
            fill(x + ICON_W - 30, y + 16, 16, 16, 0x0D1117FF);
        }
    }
}

fn open_file(name: &str) {
    let app = if name.ends_with(".md")   { "markdown-viewer" }
        else if name.ends_with(".json") { "json-viewer" }
        else if name.ends_with(".csv")  { "csv-viewer" }
        else if name.ends_with(".ppm")  { "image-viewer" }
        else if name.ends_with(".log")  { "log-viewer" }
        else { "text-editor" };
    println!("@supervisor: run {app}");
    let _ = io::stdout().flush();
}

#[derive(PartialEq)]
enum Focus { Sidebar, Main }

fn draw(
    sidebar_cursor: usize,
    main_items: &[FsItem],
    main_cursor: usize,
    focus: &Focus,
    data_files: &[FsItem],
) {
    // Background
    fill(0, 0, W, H, C_BG);
    fill(0, 0, SIDEBAR_W, H, C_SIDEBAR);

    // Toolbar
    fill(0, 0, W, TOOLBAR_H, C_TOOLBAR);
    fill(0, TOOLBAR_H, W, 1, C_BORDER);

    // Breadcrumb
    let loc = location_for_sidebar(sidebar_cursor);
    let crumb = format!("  Finder  >  {loc}");
    text(12, 12, C_DIM, &crumb);

    // Sidebar title
    text(12, TOOLBAR_H + 12, C_HINT, "FAVORITES");

    // Sidebar items
    for (i, &(name, _, color)) in SIDEBAR_ITEMS.iter().enumerate() {
        let sy = TOOLBAR_H + 32 + i as u32 * 36;
        let is_sel = i == sidebar_cursor;
        if is_sel {
            fill(0, sy - 2, SIDEBAR_W, 36, C_SEL_BG);
            fill(0, sy - 2, 3, 36, C_SEL);
        }
        // Folder icon
        fill(12, sy + 4, 16, 14, color);
        let tc = if is_sel { C_TEXT } else { C_DIM };
        text(34, sy + 8, tc, name);
    }

    // Sidebar border
    fill(SIDEBAR_W, TOOLBAR_H, 1, H - TOOLBAR_H, C_BORDER);

    // Main area grid
    let visible_rows = (MAIN_H / CELL_H) as usize;
    let visible = (GRID_COLS as usize * visible_rows).min(main_items.len());

    for (i, item) in main_items.iter().take(visible).enumerate() {
        let col = i as u32 % GRID_COLS;
        let row = i as u32 / GRID_COLS;
        let ix = GRID_START_X + col * CELL_W + (CELL_W - ICON_W) / 2;
        let iy = GRID_START_Y + row * CELL_H;
        let is_sel = i == main_cursor && *focus == Focus::Main;

        draw_file_icon(ix, iy, &item.kind, item.color, is_sel);

        let label: String = item.name.chars().take(12).collect();
        let lw = label.len() as u32 * 8;
        let lx = GRID_START_X + col * CELL_W + (CELL_W - lw.min(CELL_W)) / 2;
        let tc = if is_sel { C_TEXT } else { C_DIM };
        text(lx, iy + ICON_H + 4, tc, &label);
    }

    if main_items.is_empty() {
        text(SIDEBAR_W + 40, TOOLBAR_H + 60, C_HINT, "This folder is empty");
    }

    // Status bar
    fill(0, H - STATUS_H, W, STATUS_H, C_TOOLBAR);
    fill(0, H - STATUS_H, W, 1, C_BORDER);
    let count = format!("{} items", main_items.len());
    text(SIDEBAR_W + 12, H - STATUS_H + 8, C_HINT, &count);
    let hint = if *focus == Focus::Sidebar {
        "s: main  ↑↓: sidebar  Enter: navigate"
    } else {
        "s: sidebar  ↑↓←→: navigate  Enter: open  Esc: close"
    };
    text(SIDEBAR_W + 200, H - STATUS_H + 8, C_HINT, hint);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut data_files: Vec<FsItem> = Vec::new();
    let mut sidebar_cursor = 0usize;
    let mut main_cursor = 0usize;
    let mut focus = Focus::Main;

    println!("@supervisor: ls-data");
    println!("@supervisor: raise finder");
    let _ = io::stdout().flush();

    let init_items = items_for_sidebar(sidebar_cursor, &data_files);
    draw(sidebar_cursor, &init_items, main_cursor, &focus, &data_files);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:ls-data") {
            data_files = parse_ls(&raw);
            let items = items_for_sidebar(sidebar_cursor, &data_files);
            draw(sidebar_cursor, &items, main_cursor, &focus, &data_files);
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        let main_items = items_for_sidebar(sidebar_cursor, &data_files);

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "s" | "S" => {
                focus = if focus == Focus::Sidebar { Focus::Main } else { Focus::Sidebar };
                let items = items_for_sidebar(sidebar_cursor, &data_files);
                draw(sidebar_cursor, &items, main_cursor, &focus, &data_files);
            }
            "\x1b[A" => {
                match focus {
                    Focus::Sidebar => {
                        if sidebar_cursor > 0 { sidebar_cursor -= 1; main_cursor = 0; }
                    }
                    Focus::Main => {
                        if main_cursor >= GRID_COLS as usize { main_cursor -= GRID_COLS as usize; }
                    }
                }
                let items = items_for_sidebar(sidebar_cursor, &data_files);
                draw(sidebar_cursor, &items, main_cursor, &focus, &data_files);
            }
            "\x1b[B" => {
                match focus {
                    Focus::Sidebar => {
                        if sidebar_cursor + 1 < SIDEBAR_ITEMS.len() { sidebar_cursor += 1; main_cursor = 0; }
                    }
                    Focus::Main => {
                        let n = items_for_sidebar(sidebar_cursor, &data_files).len();
                        if main_cursor + GRID_COLS as usize < n { main_cursor += GRID_COLS as usize; }
                    }
                }
                let items = items_for_sidebar(sidebar_cursor, &data_files);
                draw(sidebar_cursor, &items, main_cursor, &focus, &data_files);
            }
            "\x1b[D" => {
                if focus == Focus::Main && main_cursor > 0 { main_cursor -= 1; }
                let items = items_for_sidebar(sidebar_cursor, &data_files);
                draw(sidebar_cursor, &items, main_cursor, &focus, &data_files);
            }
            "\x1b[C" => {
                let n = items_for_sidebar(sidebar_cursor, &data_files).len();
                if focus == Focus::Main && main_cursor + 1 < n { main_cursor += 1; }
                let items = items_for_sidebar(sidebar_cursor, &data_files);
                draw(sidebar_cursor, &items, main_cursor, &focus, &data_files);
            }
            "" => {
                match focus {
                    Focus::Sidebar => {
                        focus = Focus::Main;
                        main_cursor = 0;
                    }
                    Focus::Main => {
                        let items = items_for_sidebar(sidebar_cursor, &data_files);
                        if let Some(item) = items.get(main_cursor) {
                            open_file(&item.name);
                        }
                    }
                }
                let items = items_for_sidebar(sidebar_cursor, &data_files);
                draw(sidebar_cursor, &items, main_cursor, &focus, &data_files);
            }
            _ => {}
        }
    }
}
