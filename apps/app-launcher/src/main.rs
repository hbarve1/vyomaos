// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1440;
const H: u32 = 900;
const COLS: u32 = 4;
const BTN_W: u32 = 320;
const BTN_H: u32 = 48;
const GRID_X: u32 = 40;
const GRID_Y: u32 = 120;
const COL_PITCH: u32 = (W - GRID_X * 2) / COLS;
const ROW_PITCH: u32 = BTN_H + 10;

const C_BG: u32 = 0x0A0A0AEE;
const C_TITLE: u32 = 0xFFFFFFFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_BTN: u32 = 0x161B22FF;
const C_SEL: u32 = 0x1F6A4EFF;
const C_HINT: u32 = 0x8B949EFF;
const C_SEARCH_BG: u32 = 0x21262DFF;
const C_SEARCH_BORDER: u32 = 0x30363DFF;

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

fn parse_pkg_list(reply: &str) -> Vec<String> {
    let mut names = Vec::new();
    for part in reply.split('|') {
        let t = part.trim();
        if t.is_empty() {
            continue;
        }
        // format: "[ ] name ver  desc" or "[+] name ver  desc"
        let after = if let Some(s) = t.strip_prefix("[ ] ").or_else(|| t.strip_prefix("[+] ")) {
            s
        } else {
            t
        };
        if let Some(name) = after.split_whitespace().next() {
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    }
    names
}

fn filtered_apps<'a>(all: &'a [String], filter: &str) -> Vec<&'a String> {
    if filter.is_empty() {
        all.iter().collect()
    } else {
        let lf = filter.to_lowercase();
        all.iter().filter(|n| n.to_lowercase().contains(&lf)).collect()
    }
}

fn draw(apps: &[&String], filter: &str, selected: usize) {
    // Background overlay
    fill(0, 0, W, H, C_BG);

    // Title
    text(W / 2 - 60, 28, C_TITLE, "App Launcher");

    // Search box
    let search_x = GRID_X;
    let search_y = 68;
    let search_w = W - GRID_X * 2;
    fill(search_x, search_y, search_w, 34, C_SEARCH_BG);
    border(search_x, search_y, search_w, 34, C_SEARCH_BORDER);
    let display_filter = if filter.is_empty() { "type to filter..." } else { filter };
    let filter_color = if filter.is_empty() { C_HINT } else { C_TITLE };
    text(search_x + 10, search_y + 10, filter_color, display_filter);

    // App grid
    for (i, name) in apps.iter().enumerate() {
        let col = (i as u32) % COLS;
        let row = (i as u32) / COLS;
        let bx = GRID_X + col * COL_PITCH;
        let by = GRID_Y + row * ROW_PITCH;
        let color = if i == selected { C_SEL } else { C_BTN };
        fill(bx, by, BTN_W, BTN_H, color);
        border(bx, by, BTN_W, BTN_H, C_ACCENT);
        text(bx + 14, by + 16, C_TITLE, name);
    }

    // Footer
    text(GRID_X, H - 32, C_HINT, "Enter: launch  Arrow keys: navigate  Ctrl+C/Esc: close");

    flush();
}

fn launch(name: &str) {
    // Try built-in path first; pkg-list includes both
    println!("@supervisor: run /apps/{name}/vyoma.toml");
    let _ = io::stdout().flush();
    println!("@supervisor: focus {name}");
    let _ = io::stdout().flush();
}

fn clear_and_exit() {
    fill(0, 0, W, H, 0x0D1117FF);
    flush();
    std::process::exit(0);
}

fn main() {
    let stdin = io::stdin();
    let mut filter = String::new();
    let mut all_apps: Vec<String> = Vec::new();
    let mut selected: usize = 0;
    let mut loaded = false;

    // Draw loading state then request app list
    fill(0, 0, W, H, C_BG);
    text(W / 2 - 50, H / 2 - 8, C_HINT, "Loading apps...");
    flush();

    println!("@supervisor: pkg-list");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if let Some(reply) = raw.strip_prefix("REPLY:") {
            if !loaded {
                all_apps = parse_pkg_list(reply);
                all_apps.sort();
                loaded = true;
                let vis = filtered_apps(&all_apps, &filter);
                selected = 0;
                draw(&vis, &filter, selected);
            }
            continue;
        }

        // Escape sequences
        if raw == "\x1b" || raw == "\x1b[" {
            clear_and_exit();
        }

        // Arrow keys
        if raw == "\x1b[A" {
            // Up
            if loaded {
                let vis = filtered_apps(&all_apps, &filter);
                if selected > 0 {
                    selected -= 1;
                }
                draw(&vis, &filter, selected);
            }
            continue;
        }
        if raw == "\x1b[B" {
            // Down
            if loaded {
                let vis = filtered_apps(&all_apps, &filter);
                let max = vis.len().saturating_sub(1);
                if selected < max {
                    selected += 1;
                }
                draw(&vis, &filter, selected);
            }
            continue;
        }
        if raw == "\x1b[C" || raw == "\x1b[D" {
            // Left/Right — navigate columns
            if loaded {
                let vis = filtered_apps(&all_apps, &filter);
                let max = vis.len().saturating_sub(1);
                if raw == "\x1b[C" && selected < max {
                    selected += 1;
                } else if raw == "\x1b[D" && selected > 0 {
                    selected -= 1;
                }
                draw(&vis, &filter, selected);
            }
            continue;
        }

        // Ctrl+C
        if raw == "\x03" {
            clear_and_exit();
        }

        // Enter — launch selected
        if raw.is_empty() {
            if loaded {
                let vis = filtered_apps(&all_apps, &filter);
                if let Some(name) = vis.get(selected) {
                    let n = name.to_string();
                    launch(&n);
                    clear_and_exit();
                }
            }
            continue;
        }

        // Backspace
        if raw == "\x7f" {
            if !filter.is_empty() {
                filter.pop();
                if loaded {
                    let vis = filtered_apps(&all_apps, &filter);
                    selected = selected.min(vis.len().saturating_sub(1));
                    draw(&vis, &filter, selected);
                }
            }
            continue;
        }

        // Printable single char — append to filter
        if raw.len() == 1 {
            let ch = raw.chars().next().unwrap();
            if ch.is_ascii_graphic() || ch == ' ' {
                filter.push(ch);
                if loaded {
                    let vis = filtered_apps(&all_apps, &filter);
                    selected = 0;
                    draw(&vis, &filter, selected);
                }
            }
        }
    }
}
