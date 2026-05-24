// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const FP: i32 = 1024;
const GY: i32 = 32;
const GRID_H: i32 = H - 56; // 664

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_CARD:   u32 = 0x161B22FF;

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

#[derive(Clone, Copy, PartialEq)]
enum Wave { Sin, Square, Sawtooth, Triangle }

impl Wave {
    fn name(self) -> &'static str {
        match self {
            Wave::Sin      => "Sin",
            Wave::Square   => "Square",
            Wave::Sawtooth => "Sawtooth",
            Wave::Triangle => "Triangle",
        }
    }
}

struct Channel {
    wave: Wave,
    amp:  i32,   // 1-5
    freq: usize, // 1-8 (cycles per screen)
}

struct App {
    sin_tab: [i32; 360],
    phi:     usize,
    ch:      [Channel; 2],
    active:  usize,
    overlay: bool,
    paused:  bool,
}

impl App {
    fn new() -> Self {
        let mut sin_tab = [0i32; 360];
        for i in 0..360usize {
            let a = i as f32 * std::f32::consts::PI / 180.0;
            sin_tab[i] = (a.sin() * FP as f32) as i32;
        }
        App {
            sin_tab,
            phi: 0,
            ch: [
                Channel { wave: Wave::Sin,    amp: 3, freq: 1 },
                Channel { wave: Wave::Square, amp: 3, freq: 2 },
            ],
            active: 0,
            overlay: false,
            paused: false,
        }
    }

    fn sample(&self, ch_idx: usize, phase: usize) -> i32 {
        let c = &self.ch[ch_idx];
        let p = phase % 360;
        let raw = match c.wave {
            Wave::Sin      => self.sin_tab[p],
            Wave::Square   => if p < 180 { FP } else { -FP },
            Wave::Sawtooth => FP - FP * 2 * p as i32 / 360,
            Wave::Triangle => {
                if p < 90       { FP * p as i32 / 90 }
                else if p < 270 { FP - FP * 2 * (p as i32 - 90) / 180 }
                else            { -FP + FP * (p as i32 - 270) / 90 }
            }
        };
        raw * c.amp / 5
    }

    fn draw_channel(&self, ch_idx: usize, cy: i32, ph: i32, top: i32, bot: i32) {
        let c = &self.ch[ch_idx];
        let col = if ch_idx == 0 { C_SEL } else { C_ORANGE };
        let half = ph / 2;
        for px in 0..W {
            let phase = (self.phi + px as usize * 360 * c.freq / W as usize) % 360;
            let s = self.sample(ch_idx, phase);
            let py = (cy - s * half / FP).clamp(top, bot - 2);
            fill(px, py, 1, 2, col);
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        fill(0, H - 24, W, 24, C_HEADER);

        let ac = if self.active == 0 { 'A' } else { 'B' };
        let mode = if self.overlay { "overlay" } else { "split" };
        text(12, 8, C_TEXT, "Oscilloscope");
        text(136, 8, C_HINT, &format!(
            "Tab=ch({})  1-4=wave  A/Z=amp  S/X=freq  O={}  Space={}  Q=quit",
            ac, mode, if self.paused { "PAUSED" } else { "pause" }
        ));

        // Grid
        fill(0, GY, W, GRID_H, C_CARD);
        for i in 1..10 { fill(W * i / 10, GY, 1, GRID_H, C_BORDER); }
        for i in 0..=8 { fill(0, GY + GRID_H * i / 8, W, 1, C_BORDER); }

        if self.overlay {
            let cy = GY + GRID_H / 2;
            fill(0, cy, W, 1, C_BORDER);
            self.draw_channel(1, cy, GRID_H, GY, GY + GRID_H);
            self.draw_channel(0, cy, GRID_H, GY, GY + GRID_H);
            let la = format!("A {} amp:{} freq:{}", self.ch[0].wave.name(), self.ch[0].amp, self.ch[0].freq);
            let lb = format!("B {} amp:{} freq:{}", self.ch[1].wave.name(), self.ch[1].amp, self.ch[1].freq);
            let ca = if self.active == 0 { C_SEL } else { C_HINT };
            let cb = if self.active == 1 { C_ORANGE } else { C_HINT };
            text(8, GY + 8, ca, &la);
            text(8, GY + 22, cb, &lb);
        } else {
            let cy_a = GY + GRID_H / 4;
            let cy_b = GY + GRID_H * 3 / 4;
            let sep = GY + GRID_H / 2;
            fill(0, sep, W, 2, C_BORDER);
            fill(0, cy_a, W, 1, 0x30363DFF);
            fill(0, cy_b, W, 1, 0x30363DFF);
            self.draw_channel(0, cy_a, GRID_H / 2, GY, sep);
            self.draw_channel(1, cy_b, GRID_H / 2, sep, GY + GRID_H);
            let ca = if self.active == 0 { C_SEL } else { C_HINT };
            let cb = if self.active == 1 { C_ORANGE } else { C_HINT };
            text(8, GY + 6, ca, &format!("A: {} amp:{} freq:{}", self.ch[0].wave.name(), self.ch[0].amp, self.ch[0].freq));
            text(8, sep + 6, cb, &format!("B: {} amp:{} freq:{}", self.ch[1].wave.name(), self.ch[1].amp, self.ch[1].freq));
        }

        text(12, H - 18, C_HINT, "Dual-channel oscilloscope — Tab selects active channel for controls");
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
            "\t" => { self.active = 1 - self.active; }
            "o" | "O" => { self.overlay = !self.overlay; }
            "1" => { self.ch[self.active].wave = Wave::Sin; }
            "2" => { self.ch[self.active].wave = Wave::Square; }
            "3" => { self.ch[self.active].wave = Wave::Sawtooth; }
            "4" => { self.ch[self.active].wave = Wave::Triangle; }
            "a" | "A" => { if self.ch[self.active].amp < 5 { self.ch[self.active].amp += 1; } }
            "z" | "Z" => { if self.ch[self.active].amp > 1 { self.ch[self.active].amp -= 1; } }
            "s" | "S" => { if self.ch[self.active].freq < 8 { self.ch[self.active].freq += 1; } }
            "x" | "X" => { if self.ch[self.active].freq > 1 { self.ch[self.active].freq -= 1; } }
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
