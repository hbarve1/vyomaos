// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 1440;
const H: u32 = 900;
const C_BG: u32    = 0x0D1117FF;
const C_TEXT: u32  = 0xE6EDF3FF;
const C_DIM: u32   = 0x8B949EFF;
const C_HINT: u32  = 0x6E7681FF;
const C_SEL: u32   = 0x58A6FFFF;

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

fn elapsed_to_hms(secs: u64) -> (u64, u64, u64) {
    (secs / 3600 % 24, (secs / 60) % 60, secs % 60)
}

// Draws a padlock icon centered at (cx, cy)
fn draw_lock(cx: u32, cy: u32) {
    // Shackle (arch)
    border(cx - 20, cy - 36, 40, 30, C_SEL);
    fill(cx - 16, cy - 32, 32, 26, C_BG);
    // Body
    fill(cx - 28, cy - 8, 56, 44, C_SEL);
    border(cx - 28, cy - 8, 56, 44, C_TEXT);
    // Keyhole
    fill(cx - 6, cy + 4, 12, 12, C_BG);
    fill(cx - 3, cy + 14, 6, 10, C_BG);
}

fn draw(elapsed: u64) {
    fill(0, 0, W, H, C_BG);

    let (h, m, s) = elapsed_to_hms(elapsed + 9 * 3600);

    // Large clock
    let time_str = format!("{:02}:{:02}", h, m);
    let tw = time_str.len() as u32 * 24;
    let tx = (W - tw) / 2;
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},l,{time_str}", tx, H / 2 - 140, C_TEXT);

    // Seconds
    let sec_str = format!(":{:02}", s);
    text(tx + tw + 4, H / 2 - 120, C_DIM, &sec_str);

    // Date
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let total_days = elapsed / 86400;
    let wday = (3 + total_days) % 7; // 2026-05-21 is Thursday = 4, offset from epoch
    let date_str = format!("{}, {} {:02}", days[wday as usize], months[4], 21 + total_days % 10);
    let dw = date_str.len() as u32 * 8;
    text((W - dw) / 2, H / 2 - 70, C_DIM, &date_str);

    // Padlock icon
    draw_lock(W / 2, H / 2 + 20);

    // VyomaOS label
    let label = "VyomaOS";
    let lw = label.len() as u32 * 8;
    text((W - lw) / 2, H / 2 + 90, C_SEL, label);

    // Hint
    let hint = "Press any key to unlock";
    let hw = hint.len() as u32 * 8;
    text((W - hw) / 2, H - 60, C_HINT, hint);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let start = Instant::now();

    println!("@supervisor: raise screen-lock");
    let _ = io::stdout().flush();

    draw(0);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            let elapsed = start.elapsed().as_secs();
            draw(elapsed);
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        // Any non-reply keypress unlocks
        fill(0, 0, W, H, 0x0D1117FF);
        flush();
        std::process::exit(0);
    }
}
