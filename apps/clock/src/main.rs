// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Clock — digital uptime clock showing HH:MM:SS with blinking colons.

use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 360;
const H: u32 = 200;
const DEFAULT_SW: u32 = 2560;
const DEFAULT_SH: u32 = 1344;

const C_BG: u32    = 0x0D1117CC; // semi-transparent panel
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

/// Draw one frame: HH:MM:SS centered in the window area, subtitle, and status line.
fn draw_frame(elapsed_secs: u64, blink_on: bool, sw: u32, sh: u32) {
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;

    let colon_color = if blink_on { C_TIME } else { C_DIM };

    // Clear surface to transparent, then draw centered panel.
    println!("VYOMA_DRAW:clear_region:0,0,{sw},{sh}");

    // Center the clock panel in the window area.
    let px = if sw > W { (sw - W) / 2 } else { 0 };
    let py = if sh > H { (sh - H) / 2 } else { 0 };
    fill(px, py, W, H, C_BG);

    // --- Large HH:MM:SS centered in panel ---
    // Large font: 16px wide x 32px tall per char
    // "HH:MM:SS" = 8 chars = 128px wide
    let time_x = px + (W - 128) / 2;
    let time_y = py + 44;

    let hh = format!("{h:02}");
    draw_text_l(time_x, time_y, C_TIME, &hh);
    draw_text_l(time_x + 32, time_y, colon_color, ":");
    let mm = format!("{m:02}");
    draw_text_l(time_x + 48, time_y, C_TIME, &mm);
    draw_text_l(time_x + 80, time_y, colon_color, ":");
    let ss = format!("{s:02}");
    draw_text_l(time_x + 96, time_y, C_TIME, &ss);

    // --- Subtitle: "VyomaOS Uptime" centered in medium font ---
    let sub1 = "VyomaOS Uptime";
    let sub1_w = sub1.len() as u32 * 8;
    let sub1_x = px + (W - sub1_w) / 2;
    draw_text_m(sub1_x, time_y + 40, C_GRAY, sub1);

    // --- Third line: "WASM * Ready" in small font ---
    let sub2 = "WASM * Ready";
    let sub2_w = sub2.len() as u32 * 4;
    let sub2_x = px + if sub2_w < W { (W - sub2_w) / 2 } else { 4 };
    draw_text_s(sub2_x, time_y + 64, C_HINT, sub2);

    flush();
}

fn parse_screen(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("VYOMA_SYSTEM:screen:")?;
    let (ws, hs) = rest.split_once(',')?;
    Some((ws.parse().ok()?, hs.parse().ok()?))
}

fn main() {
    let start = Instant::now();
    let mut sw = DEFAULT_SW;
    let mut sh = DEFAULT_SH;

    // Tell compositor this surface has transparency (panel floats over wallpaper).
    println!("VYOMA_DRAW:set_transparent");
    let _ = io::stdout().flush();
    draw_frame(0, true, sw, sh);

    // Spawn a thread to read stdin for screen size updates.
    let (tx, rx) = std::sync::mpsc::channel::<(u32, u32)>();
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let raw = match line { Ok(l) => l, Err(_) => break };
            if let Some(dims) = parse_screen(&raw) {
                let _ = tx.send(dims);
            }
        }
    });

    loop {
        // Check for screen size updates (non-blocking).
        while let Ok((w, h)) = rx.try_recv() {
            sw = w;
            sh = h;
        }
        let elapsed = start.elapsed();
        let secs = elapsed.as_secs();
        let blink_on = (elapsed.as_millis() / 500) % 2 == 0;
        draw_frame(secs, blink_on, sw, sh);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
