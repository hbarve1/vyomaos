// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1000;
const H: u32 = 200;
const C_BG: u32     = 0x1C2128F4;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_RUN: u32    = 0x3FB950FF;

const THUMB_W: u32  = 120;
const THUMB_H: u32  = 80;
const THUMB_GAP: u32 = 16;
const THUMB_Y: u32  = 50;

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

fn parse_apps(reply: &str) -> Vec<String> {
    let payload = reply
        .trim_start_matches("REPLY:ps-raw:")
        .trim_start_matches("REPLY:ps-raw ");
    payload.split('|')
        .filter_map(|e| e.split(':').next().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty() && s != "app-switcher")
        .collect()
}

fn draw(apps: &[String], cursor: usize) {
    fill(0, 0, W, H, C_BG);
    border(0, 0, W, H, C_BORDER);
    fill(0, 0, W, 1, C_SEL);

    // Title
    text(20, 18, C_DIM, "App Switcher");
    text(W - 160, 18, C_HINT, "Tab/→: next   Enter: focus   Esc: cancel");

    if apps.is_empty() {
        text(20, THUMB_Y + 30, C_HINT, "No running apps.");
        flush();
        return;
    }

    let n = apps.len();
    let total_w = n as u32 * (THUMB_W + THUMB_GAP) - THUMB_GAP;
    let start_x = if total_w < W { (W - total_w) / 2 } else { 10 };

    for (i, app) in apps.iter().enumerate() {
        let tx = start_x + i as u32 * (THUMB_W + THUMB_GAP);
        let is_sel = i == cursor;

        // Thumbnail box
        let bg = if is_sel { 0x1F4068FF } else { 0x0D1117FF };
        fill(tx, THUMB_Y, THUMB_W, THUMB_H, bg);
        let bc = if is_sel { C_SEL } else { C_BORDER };
        border(tx, THUMB_Y, THUMB_W, THUMB_H, bc);

        // App initial letter — large, centered
        let initial = app.chars().next().unwrap_or('?').to_ascii_uppercase().to_string();
        let ix = tx + (THUMB_W - 8) / 2;
        let iy = THUMB_Y + (THUMB_H - 16) / 2;
        println!("VYOMA_DRAW:draw_text:{ix},{iy},{C_SEL:#010x},l,{initial}");

        // App name below thumbnail
        let label: String = app.chars().take(12).collect();
        let lw = label.len() as u32 * 8;
        let lx = tx + (THUMB_W - lw.min(THUMB_W)) / 2;
        let tc = if is_sel { C_TEXT } else { C_DIM };
        text(lx, THUMB_Y + THUMB_H + 6, tc, &label);

        // Running indicator
        fill(tx + THUMB_W/2 - 2, THUMB_Y + THUMB_H + 20, 4, 4, C_RUN);
    }

    // Selection indicator arrow
    if cursor < apps.len() {
        let sel_x = start_x + cursor as u32 * (THUMB_W + THUMB_GAP) + THUMB_W / 2 - 4;
        fill(sel_x, THUMB_Y - 8, 8, 6, C_SEL);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut apps: Vec<String> = Vec::new();
    let mut cursor = 0usize;

    // Query running apps immediately
    println!("@supervisor: ps-raw");
    println!("@supervisor: raise app-switcher");
    let _ = io::stdout().flush();

    draw(&apps, cursor);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:ps-raw") {
            apps = parse_apps(&raw);
            if cursor >= apps.len() { cursor = apps.len().saturating_sub(1); }
            draw(&apps, cursor);
            continue;
        }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\t" | "\x1b[C" => {
                if !apps.is_empty() { cursor = (cursor + 1) % apps.len(); }
                draw(&apps, cursor);
            }
            "\x1b[D" => {
                if !apps.is_empty() { cursor = (cursor + apps.len() - 1) % apps.len(); }
                draw(&apps, cursor);
            }
            "" => {
                if let Some(app) = apps.get(cursor) {
                    println!("@supervisor: focus {app}");
                    let _ = io::stdout().flush();
                }
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            _ => {}
        }
    }
}
