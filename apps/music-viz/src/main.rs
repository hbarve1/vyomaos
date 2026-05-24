// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 560;
const HEADER_H: u32 = 48;

const BAR_COUNT: usize = 32;
const BAR_W: u32 = 20;
const BAR_GAP: u32 = 5;
const BAR_AREA_W: u32 = BAR_COUNT as u32 * (BAR_W + BAR_GAP) - BAR_GAP;
const BAR_X0: u32 = (W - BAR_AREA_W) / 2;
const BAR_BOTTOM: u32 = H - 40;
const MAX_BAR_H: u32 = BAR_BOTTOM - HEADER_H - 20;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

// Fast integer sin approximation (angle in 0..1023 = 0..2pi, returns -1000..1000)
fn isin(angle: i64) -> i64 {
    let a = ((angle % 1024) + 1024) % 1024;
    // Bhaskara I approximation mapped to quarter-wave
    let (q, x) = if a < 256 { (0, a) }
    else if a < 512 { (1, 512 - a) }
    else if a < 768 { (2, a - 512) }
    else            { (3, 1024 - a) };
    let x4 = x * (256 - x);
    let v = (4 * x4 * 1000) / (40960 - x4 + 1);
    match q { 0 | 1 => v, _ => -v }
}

fn hue_to_rgb(h: u32) -> u32 {
    // h in 0..360
    let h6 = h * 6;
    let s = 255u32;
    let v = 220u32;
    let hi = (h6 / 360) % 6;
    let f = h6 % 360;
    let p = v * (360 - s) / 360; // ~0 for full saturation
    let q_val = v * (360 * 360 - s * f) / (360 * 360);
    let t = v * (360 * 360 - s * (360 - f)) / (360 * 360);
    let (r, g, b) = match hi {
        0 => (v, t, p),
        1 => (q_val, v, p),
        2 => (p, v, t),
        3 => (p, q_val, v),
        4 => (t, p, v),
        _ => (v, p, q_val),
    };
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

// Per-bar frequency and phase (compile-time derived from index)
fn bar_freq(i: usize) -> i64 { (i as i64 * 3 + 5) }
fn bar_phase(i: usize) -> i64 { (i as i64 * 31) % 1024 }
fn bar_amp(i: usize) -> u32 {
    // Larger amplitude in mid-range bars (simulated bass/treble roll-off)
    let mid = (BAR_COUNT / 2) as i64;
    let dist = (i as i64 - mid).abs();
    let scale = (mid - dist / 2).max(4) as u32;
    MAX_BAR_H * scale / mid as u32
}

struct Viz {
    tick:   i64,
    speed:  i64, // ticks per step (1..8)
    paused: bool,
    seed:   i64,
    offsets: [i64; BAR_COUNT],
}

impl Viz {
    fn new() -> Self {
        let mut v = Viz {
            tick: 0,
            speed: 2,
            paused: false,
            seed: 42,
            offsets: [0i64; BAR_COUNT],
        };
        v.randomize();
        v
    }

    fn randomize(&mut self) {
        for i in 0..BAR_COUNT {
            let s = (self.seed.wrapping_mul(6364136223846793005).wrapping_add(i as i64 * 1442695040888963407)) >> 10;
            self.offsets[i] = s.abs() % 1024;
            self.seed = s;
        }
    }

    fn bar_height(&self, i: usize) -> u32 {
        let angle = self.tick * self.speed * bar_freq(i) / 4 + bar_phase(i) + self.offsets[i];
        let sv = isin(angle); // -1000..1000
        let base = MAX_BAR_H / 3;
        let amp = bar_amp(i);
        let h = base as i64 + (amp as i64 * sv) / 2000;
        h.clamp(8, MAX_BAR_H as i64) as u32
    }
}

fn draw(v: &Viz) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Music Visualizer");
    let spd = &format!("Speed: {}x", v.speed);
    text(240, 16, C_HINT, spd);
    if v.paused { text(360, 16, C_TEXT, "PAUSED"); }
    text(500, 16, C_HINT, "+/-:speed  Space:pause  R:randomize");

    // Ground line
    fill(0, BAR_BOTTOM, W, 2, 0x30363DFF);

    // Bars
    for i in 0..BAR_COUNT {
        let bh = v.bar_height(i);
        let bx = BAR_X0 + i as u32 * (BAR_W + BAR_GAP);
        let by = BAR_BOTTOM - bh;

        let hue = i as u32 * 360 / BAR_COUNT as u32;
        let color = hue_to_rgb(hue);

        fill(bx, by, BAR_W, bh, color);
        // Bright top cap
        let cap = (bh / 8).max(2);
        let bright = hue_to_rgb((hue + 30) % 360);
        fill(bx, by, BAR_W, cap, bright);

        // Reflection (faded below ground)
        let ref_h = (bh / 4).min(40);
        let ref_color = (color & 0xFFFFFF00) | 0x40;
        fill(bx, BAR_BOTTOM + 2, BAR_W, ref_h, ref_color);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut viz = Viz::new();

    println!("@supervisor: raise music-viz");
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&viz);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            if !viz.paused {
                viz.tick += 1;
                draw(&viz);
            }
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "+" | "=" => { if viz.speed < 8 { viz.speed += 1; } }
            "-" => { if viz.speed > 1 { viz.speed -= 1; } }
            " " => { viz.paused = !viz.paused; }
            "r" | "R" => { viz.seed = viz.tick; viz.randomize(); }
            _ => {}
        }
        draw(&viz);
    }
}
