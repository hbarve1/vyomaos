// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const FP: i32 = 1024;
const PCX: i32 = 230;
const PCY: i32 = 370;
const WX0: i32 = 492;
const WCY: i32 = 370;
const WW: i32 = 440;
const MAX_R: i32 = 70;

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL:    u32 = 0x58A6FFFF;

const ARM_COLS: [u32; 10] = [
    0x58A6FFFF, 0x3FB950FF, 0xFFA657FF, 0xC77DFFFF,
    0xFF7B72FF, 0x00CECBFF, 0xFFD93DFF, 0xFF6392FF,
    0x79C0FFFF, 0x56D364FF,
];

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn isqrt(n: i64) -> i64 {
    if n <= 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}

#[derive(Clone, Copy, PartialEq)]
enum Wave { Square, Sawtooth, Triangle }

struct App {
    sin_tab: [i32; 360],
    cos_tab: [i32; 360],
    phi: usize,
    harmonics: usize,
    wave: Wave,
    paused: bool,
}

impl App {
    fn new() -> Self {
        let mut sin_tab = [0i32; 360];
        let mut cos_tab = [0i32; 360];
        for i in 0..360usize {
            let a = i as f32 * std::f32::consts::PI / 180.0;
            sin_tab[i] = (a.sin() * FP as f32) as i32;
            cos_tab[i] = (a.cos() * FP as f32) as i32;
        }
        App { sin_tab, cos_tab, phi: 0, harmonics: 5, wave: Wave::Square, paused: false }
    }

    fn harm_amp(&self, k: usize) -> Option<i32> {
        match self.wave {
            Wave::Square => {
                if k % 2 == 0 { return None; }
                Some(1303 / k as i32)
            }
            Wave::Sawtooth => {
                let sign: i32 = if k % 2 == 1 { 1 } else { -1 };
                Some(sign * 651 / k as i32)
            }
            Wave::Triangle => {
                if k % 2 == 0 { return None; }
                let sign: i32 = if (k / 2) % 2 == 0 { 1 } else { -1 };
                Some(sign * 838 / (k as i32 * k as i32))
            }
        }
    }

    fn fund_amp(&self) -> i32 {
        match self.wave { Wave::Square => 1303, Wave::Sawtooth => 651, Wave::Triangle => 838 }
    }

    fn wave_y(&self, phase: usize) -> i32 {
        let mut sum = 0i32;
        for k in 1..=self.harmonics {
            if let Some(amp) = self.harm_amp(k) {
                let angle = (k * phase) % 360;
                sum += amp * self.sin_tab[angle] / FP;
            }
        }
        sum
    }

    fn draw_arm(x1: i32, y1: i32, x2: i32, y2: i32, c: u32) {
        let dx = x2 - x1;
        let dy = y2 - y1;
        let len = isqrt((dx * dx + dy * dy) as i64) as i32;
        if len == 0 { fill(x1, y1, 2, 2, c); return; }
        for t in 0..=len {
            let x = x1 + dx * t / len;
            let y = y1 + dy * t / len;
            fill(x, y, 2, 2, c);
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        fill(0, H - 24, W, 24, C_HEADER);

        let wname = match self.wave {
            Wave::Square   => "Square",
            Wave::Sawtooth => "Sawtooth",
            Wave::Triangle => "Triangle",
        };
        text(12, 8, C_TEXT, "Fourier Series");
        text(148, 8, C_HINT, &format!(
            "Tab=wave({})  +/-=harmonics({:02})  Space={}  Q=quit",
            wname, self.harmonics, if self.paused { "PAUSED" } else { "pause" }
        ));
        text(12, H - 18, C_HINT, "Rotating phasors approximating periodic waveforms");

        fill(482, 32, 2, H - 56, C_BORDER);

        // --- LEFT: Phasors ---
        let fund = self.fund_amp();
        let mut cx = PCX;
        let mut cy = PCY;

        for k in 1..=self.harmonics {
            if let Some(amp) = self.harm_amp(k) {
                let r = (amp.abs() * MAX_R / fund).max(2);
                let col = ARM_COLS[(k - 1) % 10];

                for d in (0..360usize).step_by(5) {
                    let ox = (cx + r * self.cos_tab[d] / FP).clamp(0, 480);
                    let oy = (cy + r * self.sin_tab[d] / FP).clamp(33, H - 25);
                    fill(ox, oy, 1, 1, C_BORDER);
                }

                let angle = (k * self.phi) % 360;
                let tx = cx + r * self.cos_tab[angle] / FP;
                let ty = cy + r * self.sin_tab[angle] / FP;
                Self::draw_arm(cx, cy, tx, ty, col);
                cx = tx;
                cy = ty;
            }
        }

        fill(cx - 2, cy - 2, 5, 5, C_TEXT);

        // --- RIGHT: Wave ---
        let wave_scale = 155i32;

        fill(WX0, WCY, WW, 1, C_BORDER);

        for px in 0..WW {
            let phase = (px * 360 / WW) as usize;
            let wy = self.wave_y(phase);
            let sy = (WCY - wy * wave_scale / FP).clamp(33, H - 25);
            fill(WX0 + px, sy, 2, 2, C_SEL);
        }

        let mx = WX0 + self.phi as i32 * WW / 360;
        fill(mx, 33, 1, H - 58, C_HINT);

        let wy = self.wave_y(self.phi);
        let sy = (WCY - wy * wave_scale / FP).clamp(33, H - 25);
        fill(mx - 2, sy - 2, 5, 5, C_ORANGE);

        if (cy - sy).abs() < 6 && cx < mx {
            fill(cx, cy, mx - cx, 1, C_HINT);
        }

        // --- BOTTOM: Harmonic bars ---
        let bx0 = 10i32;
        for k in 1..=20usize {
            let bx = bx0 + (k as i32 - 1) * 46;
            let active = k <= self.harmonics;
            let col = if active { ARM_COLS[(k - 1) % 10] } else { C_BORDER };
            if let Some(amp) = self.harm_amp(k) {
                let bh = (amp.abs() * 18 / fund).max(1);
                fill(bx, H - 6 - bh, 38, bh, col);
            } else {
                fill(bx, H - 7, 38, 2, C_BORDER);
            }
            let tc = if active { C_TEXT } else { C_HINT };
            text(bx + 12, H - 26, tc, &k.to_string());
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                if !self.paused { self.phi = (self.phi + 2) % 360; }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " " => { self.paused = !self.paused; }
            "\t" => {
                self.wave = match self.wave {
                    Wave::Square   => Wave::Sawtooth,
                    Wave::Sawtooth => Wave::Triangle,
                    Wave::Triangle => Wave::Square,
                };
            }
            "+" | "=" => { if self.harmonics < 20 { self.harmonics += 1; } }
            "-" => { if self.harmonics > 1 { self.harmonics -= 1; } }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
