// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Music Visualizer — 24 animated rainbow bars synced to a BPM timer.
//! Controls: +/= increase BPM, - decrease BPM, Space pause/resume.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const W: u32 = 1440;
const H: u32 = 876;
const BAR_COUNT: usize = 24;
const BAR_W: u32 = W / BAR_COUNT as u32; // 60px
const MAX_BAR_H: f32 = 800.0;
const BAR_BOTTOM: u32 = H - 24; // leave room for BPM text

const C_BG: u32 = 0x0D1117FF;
const C_TEXT: u32 = 0xE6EDF3FF;
const C_HINT: u32 = 0x6E7681FF;

/// Precomputed rainbow colors for 24 bars (RGBA decimal).
const BAR_COLORS: &[u32; BAR_COUNT] = &[
    0xFF0000FF, 0xFF2200FF, 0xFF4400FF, 0xFF6600FF, 0xFF8800FF, 0xFFAA00FF,
    0xFFCC00FF, 0xFFFF00FF, 0xAAFF00FF, 0x55FF00FF, 0x00FF00FF, 0x00FF55FF,
    0x00FFAAFF, 0x00FFFFFF, 0x00AAFFFF, 0x0055FFFF, 0x0000FFFF, 0x5500FFFF,
    0xAA00FFFF, 0xFF00FFFF, 0xFF00AAFF, 0xFF0055FF, 0xFF0022FF, 0xFF0000FF,
];

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}

fn text_m(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}

fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

/// Compute bar height using sine wave animation.
/// t = elapsed_ms / 1000.0
/// freq_i = 0.5 + i * 0.1, phase_i = i * 0.4
fn bar_height(i: usize, t: f32) -> u32 {
    let freq_i = 0.5 + i as f32 * 0.1;
    let phase_i = i as f32 * 0.4;
    let val = 0.5 + 0.5 * (t * freq_i + phase_i).sin();
    ((val * MAX_BAR_H) as u32).max(4).min(MAX_BAR_H as u32)
}

struct State {
    bpm:      u32,
    paused:   bool,
    frozen_t: f32, // animation time when paused
}

fn draw_frame(t: f32, state: &State) {
    fill(0, 0, W, H, C_BG);

    // Draw 24 bars, bottom-aligned
    for i in 0..BAR_COUNT {
        let bh = bar_height(i, t);
        let bx = i as u32 * BAR_W;
        let by = BAR_BOTTOM - bh;
        fill(bx, by, BAR_W - 2, bh, BAR_COLORS[i]);
    }

    // BPM indicator top-right
    let bpm_str = format!("{} BPM", state.bpm);
    let bpm_w = bpm_str.len() as u32 * 8;
    let bpm_x = W - bpm_w - 16;
    text_m(bpm_x, 8, C_TEXT, &bpm_str);

    if state.paused {
        text_m(16, 8, 0xFFAA00FF, "PAUSED");
    }

    // Controls hint at bottom
    text_m(16, H - 16, C_HINT, "+/-: BPM   Space: pause");

    flush();
}

fn main() {
    let start   = Instant::now();
    let state   = Arc::new(Mutex::new(State { bpm: 120, paused: false, frozen_t: 0.0 }));
    let state_t = Arc::clone(&state);

    // Stdin reader thread: handles key events without blocking the draw loop
    std::thread::spawn(move || {
        use std::io::BufRead;
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            let raw = match line { Ok(l) => l, Err(_) => break };

            // Skip supervisor/system messages
            if raw.starts_with("VYOMA_") || raw.starts_with("REPLY:") {
                continue;
            }

            match raw.as_str() {
                "\x1b" | "\x03" => {
                    // Exit: clear screen and quit
                    fill(0, 0, W, H, C_BG);
                    flush();
                    std::process::exit(0);
                }
                "+" | "=" => {
                    let mut s = state_t.lock().unwrap();
                    if s.bpm < 200 { s.bpm += 10; }
                }
                "-" => {
                    let mut s = state_t.lock().unwrap();
                    if s.bpm > 40 { s.bpm -= 10; }
                }
                " " => {
                    let mut s = state_t.lock().unwrap();
                    s.paused = !s.paused;
                }
                _ => {}
            }
        }
    });

    // Main draw loop: ~60 fps
    let frame_ms = std::time::Duration::from_millis(16);
    let mut was_paused = false;
    loop {
        let elapsed_ms = start.elapsed().as_millis() as f32;
        let mut s = state.lock().unwrap();
        let bpm = s.bpm;
        let paused = s.paused;

        // Compute animation time
        let current_t = elapsed_ms * (bpm as f32 / 120.0) / 1000.0;

        // Capture freeze point on pause transition
        if paused && !was_paused {
            s.frozen_t = current_t;
        }
        was_paused = paused;

        let t = if paused { s.frozen_t } else { current_t };
        draw_frame(t, &s);
        drop(s);

        std::thread::sleep(frame_ms);
    }
}
