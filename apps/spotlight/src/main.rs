// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Spotlight — full-screen app launcher overlay.
//! Type to filter app names. Enter launches. Escape exits.

mod search;

use std::io::{self, BufRead, Write};

const W: u32 = 1440;
const H: u32 = 876;

// Search box geometry (centered)
const BOX_W: u32   = 800;
const BOX_X: u32   = (W - BOX_W) / 2; // 320
const BOX_Y: u32   = 300;
const BOX_H: u32   = 56;

// Results list
const RESULTS_Y: u32 = BOX_Y + BOX_H + 12;
const ROW_H: u32     = 50;
const MAX_VISIBLE: usize = 10;

// Colors
const C_OVERLAY: u32 = 0x1E1E2ECC; // frosted dark overlay at ~80% alpha
const C_BOX: u32     = 0x2D2D3EFF; // search box bg
const C_SEL: u32     = 0x3B4EE8FF; // selection highlight blue
const C_TEXT: u32    = 0xE6EDF3FF; // white text
const C_HINT: u32    = 0x8B949EFF; // gray hint
const C_DIM: u32     = 0x6E7681FF; // dim text

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}

fn fill_r(x: u32, y: u32, w: u32, h: u32, rgba: u32, radius: u32) {
    println!("VYOMA_DRAW:fill_rect_r:{x},{y},{w},{h},{rgba},{radius}");
}

fn text_m(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}

fn text_l(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},l,{s}");
}

fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn draw(query: &str, results: &[(&str, &str)], cursor: usize) {
    // Full-screen frosted dark overlay (semi-transparent)
    fill_r(0, 0, W, H, C_OVERLAY, 0);

    // Title at top center
    let title = "Spotlight";
    let title_w = title.len() as u32 * 16;
    let title_x = (W - title_w) / 2;
    text_l(title_x, 220, C_TEXT, title);

    // Search box background
    fill(BOX_X, BOX_Y, BOX_W, BOX_H, C_BOX);

    // Search box text (show placeholder when empty, or query with cursor)
    let text_color = if query.is_empty() { C_HINT } else { C_TEXT };
    let show_text = if query.is_empty() {
        "Search apps...".to_string()
    } else {
        format!("{query}_")
    };
    text_m(BOX_X + 20, BOX_Y + 20, text_color, &show_text);

    // Results
    let visible = results.len().min(MAX_VISIBLE);
    for (i, (name, cmd)) in results.iter().take(visible).enumerate() {
        let ry = RESULTS_Y + i as u32 * ROW_H;
        let is_sel = i == cursor;
        if is_sel {
            fill(BOX_X, ry, BOX_W, ROW_H, C_SEL);
        }
        let name_color = if is_sel { C_TEXT } else { C_HINT };
        text_m(BOX_X + 20, ry + 17, name_color, name);
        // Right-align the cmd slug
        let cmd_w = cmd.len() as u32 * 8;
        let cmd_x = BOX_X + BOX_W - cmd_w - 20;
        text_m(cmd_x, ry + 17, C_DIM, cmd);
    }

    if results.is_empty() {
        text_m(BOX_X + 20, RESULTS_Y + 17, C_HINT, "No matching apps");
    }

    // Footer hint
    let hint = "Enter: Launch  |  Esc: Cancel";
    let hint_w = hint.len() as u32 * 8;
    let hint_x = (W - hint_w) / 2;
    text_m(hint_x, H - 40, C_DIM, hint);

    // Result count
    let count_str = format!("{} apps", results.len());
    text_m(BOX_X, H - 40, C_DIM, &count_str);

    flush();
}

fn main() {
    let stdin  = io::stdin();
    let mut query  = String::new();
    let mut cursor = 0usize;

    // Raise spotlight to front
    println!("@supervisor: raise spotlight");
    let _ = io::stdout().flush();

    let init_results = search::filter(&query);
    draw(&query, &init_results, cursor);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        // Skip supervisor replies
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            // Escape or Ctrl+C — exit
            "\x1b" | "\x03" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            // Backspace
            "\x7f" => {
                query.pop();
                cursor = 0;
            }
            // Up arrow
            "\x1b[A" => {
                if cursor > 0 { cursor -= 1; }
            }
            // Down arrow
            "\x1b[B" => {
                let n = search::filter(&query).len().min(MAX_VISIBLE);
                if cursor + 1 < n { cursor += 1; }
            }
            // Enter — launch selected app
            "" => {
                let r = search::filter(&query);
                if let Some((_, cmd)) = r.get(cursor) {
                    println!("@supervisor: run {cmd}");
                    let _ = io::stdout().flush();
                    fill(0, 0, W, H, 0x0D1117FF);
                    flush();
                    std::process::exit(0);
                }
            }
            // Printable character — append to query
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if c.is_ascii_graphic() || c == ' ' {
                    query.push(c);
                    cursor = 0;
                }
            }
            _ => {}
        }

        let results = search::filter(&query);
        // Clamp cursor
        let max_cur = results.len().min(MAX_VISIBLE).saturating_sub(1);
        if cursor > max_cur { cursor = max_cur; }
        draw(&query, &results, cursor);
    }
}
