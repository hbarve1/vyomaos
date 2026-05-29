// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Clock — digital uptime clock showing HH:MM:SS with blinking colons.

use std::io::{self, Write};
use std::time::Instant;

const W: u32 = 360;
const H: u32 = 200;

const C_BG: u32    = 0x0D1117FF; // GitHub dark
const C_TIME: u32  = 0x00F5FFFF; // bright cyan
const C_DIM: u32   = 0x334155FF; // dim (colon off-state)
const C_GRAY: u32  = 0x8B949EFF; // subtitle gray
const C_HINT: u32  = 0x6E7681FF; // dim hint

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}

fn draw_text_l(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},l,{s}");
}

fn draw_text_m(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}

fn draw_text_s(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},s,{s}");
}

fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

/// Draw one frame: HH:MM:SS centered, subtitle, and status line.
fn draw_frame(elapsed_secs: u64, blink_on: bool) {
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;

    let colon_color = if blink_on { C_TIME } else { C_DIM };

    // Background
    fill(0, 0, W, H, C_BG);

    // --- Large HH:MM:SS centered ---
    // Large font: 16px wide × 32px tall per char
    // "HH:MM:SS" = 8 chars = 128px wide
    // Center in 360px: x = (360 - 128) / 2 = 116
    let time_y = 44u32;

    // Draw HH
    let hh = format!("{h:02}");
    draw_text_l(116, time_y, C_TIME, &hh);

    // Draw first colon (at x = 116 + 2*16 = 148)
    draw_text_l(148, time_y, colon_color, ":");

    // Draw MM (at x = 148 + 16 = 164)
    let mm = format!("{m:02}");
    draw_text_l(164, time_y, C_TIME, &mm);

    // Draw second colon (at x = 164 + 2*16 = 196)
    draw_text_l(196, time_y, colon_color, ":");

    // Draw SS (at x = 196 + 16 = 212)
    let ss = format!("{s:02}");
    draw_text_l(212, time_y, C_TIME, &ss);

    // --- Subtitle: "VyomaOS Uptime" centered in medium font ---
    // medium font: 8px wide × 16px tall
    let sub1 = "VyomaOS Uptime";
    let sub1_w = sub1.len() as u32 * 8;
    let sub1_x = (W - sub1_w) / 2;
    draw_text_m(sub1_x, time_y + 40, C_GRAY, sub1);

    // --- Third line: "WASM * Ready" in small font ---
    // Use ASCII asterisk to avoid multi-byte char width confusion
    let sub2 = "WASM * Ready";
    let sub2_w = sub2.len() as u32 * 4; // small font: 4px wide per char
    let sub2_x = if sub2_w < W { (W - sub2_w) / 2 } else { 4 };
    draw_text_s(sub2_x, time_y + 64, C_HINT, sub2);

    flush();
}

fn main() {
    let start = Instant::now();
    draw_frame(0, true);
    loop {
        let elapsed = start.elapsed();
        let secs = elapsed.as_secs();
        let blink_on = (elapsed.as_millis() / 500) % 2 == 0;
        draw_frame(secs, blink_on);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
