// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Clock — digital uptime clock showing HH:MM:SS with blinking colons.

use std::io::{self, Write};
use std::time::Instant;

// Widget dimensions (the clock panel itself)
const PANEL_W: u32 = 360;
const PANEL_H: u32 = 200;

// Default screen area (updated via VYOMA_SYSTEM:resize on stdin)
const DEFAULT_SW: u32 = 2560;
const DEFAULT_SH: u32 = 1344;

const C_PANEL: u32 = 0x0D111780; // semi-transparent dark panel
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

/// Draw one frame: HH:MM:SS centered in a floating panel widget.
fn draw_frame(elapsed_secs: u64, blink_on: bool, sw: u32, sh: u32) {
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;

    let colon_color = if blink_on { C_TIME } else { C_DIM };

    // Center the panel widget in the assigned screen area
    let ox = sw.saturating_sub(PANEL_W) / 2;
    let oy = sh.saturating_sub(PANEL_H) / 2;

    // Semi-transparent panel background (supervisor gradient shows through)
    fill(ox, oy, PANEL_W, PANEL_H, C_PANEL);

    // --- Large HH:MM:SS centered in panel ---
    // Large font: 16px wide x 32px tall per char
    // "HH:MM:SS" = 8 chars = 128px wide
    // Center in panel: x = ox + (360 - 128) / 2 = ox + 116
    let time_y = oy + 44;

    // Draw HH
    let hh = format!("{h:02}");
    draw_text_l(ox + 116, time_y, C_TIME, &hh);

    // Draw first colon
    draw_text_l(ox + 148, time_y, colon_color, ":");

    // Draw MM
    let mm = format!("{m:02}");
    draw_text_l(ox + 164, time_y, C_TIME, &mm);

    // Draw second colon
    draw_text_l(ox + 196, time_y, colon_color, ":");

    // Draw SS
    let ss = format!("{s:02}");
    draw_text_l(ox + 212, time_y, C_TIME, &ss);

    // --- Subtitle: "VyomaOS Uptime" centered in panel ---
    let sub1 = "VyomaOS Uptime";
    let sub1_w = sub1.len() as u32 * 8;
    let sub1_x = ox + (PANEL_W - sub1_w) / 2;
    draw_text_m(sub1_x, time_y + 40, C_GRAY, sub1);

    // --- Third line: "WASM * Ready" in small font ---
    let sub2 = "WASM * Ready";
    let sub2_w = sub2.len() as u32 * 4;
    let sub2_x = ox + if sub2_w < PANEL_W { (PANEL_W - sub2_w) / 2 } else { 4 };
    draw_text_s(sub2_x, time_y + 64, C_HINT, sub2);

    flush();
}

fn parse_screen(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("VYOMA_SYSTEM:screen:")?;
    let (ws, hs) = rest.split_once(',')?;
    Some((ws.parse().ok()?, hs.parse().ok()?))
}

fn main() {
    use std::io::BufRead;
    use std::sync::{Arc, Mutex};

    let start = Instant::now();
    let dims = Arc::new(Mutex::new((DEFAULT_SW, DEFAULT_SH)));

    // Spawn a thread to read stdin for screen-size updates
    let dims_rx = Arc::clone(&dims);
    std::thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let raw = match line { Ok(l) => l, Err(_) => break };
            if let Some((w, h)) = parse_screen(&raw) {
                if let Ok(mut d) = dims_rx.lock() { *d = (w, h); }
            }
        }
    });

    draw_frame(0, true, DEFAULT_SW, DEFAULT_SH);
    loop {
        let elapsed = start.elapsed();
        let secs = elapsed.as_secs();
        let blink_on = (elapsed.as_millis() / 500) % 2 == 0;
        let (sw, sh) = *dims.lock().unwrap_or_else(|e| e.into_inner());
        draw_frame(secs, blink_on, sw, sh);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}
