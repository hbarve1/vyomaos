// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 600;
const H: u32 = 400;
const C_BG: u32     = 0x1C2128F0;
const C_BORDER: u32 = 0x58A6FFFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_SEL: u32    = 0x1F4068FF;
const C_HINT: u32   = 0x6E7681FF;
const C_FIELD: u32  = 0x0D1117FF;

const SEARCH_Y: u32 = 20;
const SEARCH_H: u32 = 44;
const RESULTS_Y: u32 = SEARCH_Y + SEARCH_H + 8;
const ROW_H: u32    = 36;
const VIS: usize    = ((H - RESULTS_Y - 20) / ROW_H) as usize;

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

// All known apps
const ALL_APPS: &[(&str, &str)] = &[
    ("File Manager",       "file-manager"),
    ("Text Editor",        "text-editor"),
    ("Browser",            "browser"),
    ("System Monitor",     "system-monitor"),
    ("Calendar",           "calendar"),
    ("Task Manager",       "task-manager"),
    ("Settings",           "settings"),
    ("App Store",          "app-store"),
    ("Shell",              "shell"),
    ("Clock",              "clock"),
    ("Weather",            "weather"),
    ("Calculator",         "sci-calculator"),
    ("Password Manager",   "password-manager"),
    ("Image Viewer",       "image-viewer"),
    ("Hex Editor",         "hex-editor"),
    ("Markdown Viewer",    "markdown-viewer"),
    ("CSV Viewer",         "csv-viewer"),
    ("JSON Viewer",        "json-viewer"),
    ("Log Viewer",         "log-viewer"),
    ("Diff Viewer",        "diff-viewer"),
    ("Color Picker",       "color-picker"),
    ("Process Inspector",  "process-inspector"),
    ("Font Chooser",       "font-chooser"),
    ("SSH Client",         "ssh-client"),
    ("DNS Resolver",       "dns-resolver"),
    ("Network Config",     "network-config"),
    ("Audio Player",       "audio-player"),
    ("Pomodoro",           "pomodoro"),
    ("Virtual Keyboard",   "virtual-keyboard"),
    ("Dock",               "dock"),
    ("Menu Bar",           "menu-bar"),
];

fn filtered<'a>(query: &str) -> Vec<&'a (&'a str, &'a str)> {
    let q = query.to_ascii_lowercase();
    ALL_APPS.iter()
        .filter(|(name, cmd)| {
            q.is_empty()
                || name.to_ascii_lowercase().contains(&q)
                || cmd.to_ascii_lowercase().contains(&q)
        })
        .collect()
}

fn draw(query: &str, results: &[&(&str, &str)], cursor: usize) {
    fill(0, 0, W, H, C_BG);
    border(0, 0, W, H, C_BORDER);

    // Search icon + field
    text(18, SEARCH_Y + 14, C_ACCENT, "⌕");
    fill(44, SEARCH_Y, W - 64, SEARCH_H, C_FIELD);
    border(44, SEARCH_Y, W - 64, SEARCH_H, C_BORDER);
    let disp = if query.is_empty() { "Search apps…" } else { query };
    let tc   = if query.is_empty() { C_HINT } else { C_TEXT };
    text(54, SEARCH_Y + 14, tc, disp);

    // Divider
    fill(20, RESULTS_Y - 4, W - 40, 1, 0x30363DFF);

    // Results
    for (i, &&(name, cmd)) in results.iter().take(VIS).enumerate() {
        let ry = RESULTS_Y + i as u32 * ROW_H;
        let is_sel = i == cursor;
        if is_sel { fill(0, ry, W, ROW_H, C_SEL); }
        let tc = if is_sel { C_TEXT } else { C_DIM };
        text(54, ry + 10, tc, name);
        let cw = cmd.len() as u32 * 8;
        text(W - cw - 20, ry + 10, C_HINT, cmd);
    }

    if results.is_empty() {
        text(54, RESULTS_Y + 10, C_HINT, "No apps match");
    }

    // Footer
    fill(0, H - 22, W, 1, 0x30363DFF);
    let count = format!("{} apps", results.len());
    text(20, H - 14, C_HINT, &count);
    text(W - 140, H - 14, C_HINT, "Enter: launch  Esc: close");

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut query  = String::new();
    let mut cursor = 0usize;

    let init_results = filtered(&query);
    draw(&query, &init_results, cursor);

    // Raise to front
    println!("@supervisor: raise spotlight");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        let results = filtered(&query);

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x7f" => {
                query.pop();
                cursor = 0;
            }
            "\x1b[A" => {
                if cursor > 0 { cursor -= 1; }
            }
            "\x1b[B" => {
                let r = filtered(&query);
                if cursor + 1 < r.len().min(VIS) { cursor += 1; }
            }
            "" => {
                let r = filtered(&query);
                if let Some(&&(_, cmd)) = r.get(cursor) {
                    println!("@supervisor: run {cmd}");
                    let _ = io::stdout().flush();
                    fill(0, 0, W, H, 0x0D1117FF);
                    flush();
                    std::process::exit(0);
                }
            }
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if c.is_ascii_graphic() || c == ' ' {
                    query.push(c);
                    cursor = 0;
                }
            }
            _ => {}
        }

        let results = filtered(&query);
        draw(&query, &results, cursor);
    }
}
