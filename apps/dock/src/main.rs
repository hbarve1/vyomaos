// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 60;
const C_BG: u32     = 0x1C2128FF;
const C_BORDER: u32 = 0x30363DFF;
const C_LABEL: u32  = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_DOT: u32    = 0xFFFFFFFF;
const C_SEL: u32    = 0x58A6FFFF;

const ICON_W: u32   = 52;
const ICON_H: u32   = 44;
const ICON_GAP: u32 = 8;
const ICON_Y: u32   = 6;

// Dock entries: (display label, app name to launch, icon color)
const DOCK_APPS: [(&str, &str, u32); 9] = [
    ("FM",  "file-manager",      0x1F6FEB33),
    ("TXT", "text-editor",       0x3FB95033),
    ("WEB", "browser",           0xFF7B7233),
    ("MON", "system-monitor",    0xFFA65733),
    ("CAL", "calendar",          0x58A6FF33),
    ("TSK", "task-manager",      0x3FB95033),
    ("SET", "settings",          0x8B949E33),
    ("STO", "app-store",         0x79C0FF33),
    ("SHL", "shell",             0x30363DFF),
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

fn total_dock_w() -> u32 {
    DOCK_APPS.len() as u32 * (ICON_W + ICON_GAP) - ICON_GAP
}

fn icon_x(idx: usize) -> u32 {
    let start = (W - total_dock_w()) / 2;
    start + idx as u32 * (ICON_W + ICON_GAP)
}

fn parse_running(reply: &str) -> Vec<String> {
    reply.split('|')
        .filter_map(|entry| entry.split(':').next().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn draw(running: &[String], highlighted: Option<usize>) {
    fill(0, 0, W, H, C_BG);
    border(0, 0, W, H, C_BORDER);
    // Top separator
    fill(0, 0, W, 1, C_BORDER);

    for (i, (label, app_name, icon_color)) in DOCK_APPS.iter().enumerate() {
        let ix = icon_x(i);
        let is_running    = running.iter().any(|r| r == app_name);
        let is_highlighted = highlighted == Some(i);

        // Icon background
        let bg = if is_highlighted { C_SEL } else { *icon_color };
        fill(ix, ICON_Y, ICON_W, ICON_H, bg);
        border(ix, ICON_Y, ICON_W, ICON_H, C_BORDER);

        // Label centered in icon
        let lw = label.len() as u32 * 8;
        let lx = ix + (ICON_W - lw) / 2;
        let lc = if is_highlighted { 0xFFFFFFFF } else { C_LABEL };
        text(lx, ICON_Y + (ICON_H - 14) / 2, lc, label);

        // Running indicator dot
        if is_running {
            let dot_x = ix + ICON_W / 2 - 2;
            let dot_y = ICON_Y + ICON_H + 1;
            fill(dot_x, dot_y, 4, 4, C_DOT);
        }

        // Key hint (1-9) below icon
        let key_str = format!("{}", i + 1);
        let kx = ix + (ICON_W - 8) / 2;
        text(kx, ICON_Y + ICON_H + 6, C_HINT, &key_str);
    }

    flush();
}

fn main() {
    let stdin  = io::stdin();
    let mut running: Vec<String> = Vec::new();
    let mut highlighted: Option<usize> = None;

    draw(&running, highlighted);

    // Raise to front, start polling
    println!("@supervisor: raise dock");
    println!("@supervisor: ps-raw");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:pong") || raw.starts_with("REPLY:ping") {
            println!("@supervisor: ps-raw");
            let _ = io::stdout().flush();
        }

        if raw.starts_with("REPLY:ps-raw") {
            let payload = raw.trim_start_matches("REPLY:ps-raw:").trim_start_matches("REPLY:ps-raw ");
            running = parse_running(payload);
            // Re-poll in ~2s
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            k if k.len() == 1 && k.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) => {
                let idx = k.chars().next().unwrap().to_digit(10).unwrap_or(0) as usize;
                if idx >= 1 && idx <= DOCK_APPS.len() {
                    let app = DOCK_APPS[idx - 1].1;
                    highlighted = Some(idx - 1);
                    draw(&running, highlighted);
                    println!("@supervisor: run {app}");
                    let _ = io::stdout().flush();
                    // Clear highlight after brief moment (next event)
                    highlighted = None;
                }
            }
            _ => {}
        }

        draw(&running, highlighted);
    }
}
