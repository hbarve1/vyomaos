// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::f32::consts::PI;
use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

// Canvas: 600×600, centered at (CX, CY)
const CW: i32 = 600;
const CH: i32 = 600;
const CX: i32 = (W - CW) / 2;
const CY: i32 = 56;
const HALF_W: i32 = CW / 2;
const HALF_H: i32 = CH / 2;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn hue_color(h: f32) -> u32 {
    let h = h % 360.0;
    let s = h / 60.0;
    let i = s as u32;
    let f = s - i as f32;
    let (r, g, b): (f32, f32, f32) = match i {
        0 => (1.0, f,       0.0),
        1 => (1.0-f, 1.0,   0.0),
        2 => (0.0,   1.0,   f),
        3 => (0.0,   1.0-f, 1.0),
        4 => (f,     0.0,   1.0),
        _ => (1.0,   0.0,   1.0-f),
    };
    ((r * 220.0) as u32) << 24
        | ((g * 220.0) as u32) << 16
        | ((b * 220.0) as u32) << 8
        | 0xFF
}

const TOTAL_STEPS: usize = 360;
const DELTA_STEP: f32 = 2.0 * PI / TOTAL_STEPS as f32;

struct App {
    a:     usize,   // x frequency 1-8
    b:     usize,   // y frequency 1-8
    delta: usize,   // phase delta in units of 15 degrees (0-23 → 0-345 deg)
    tick:  usize,   // current animation step 0..TOTAL_STEPS
}

impl App {
    fn new() -> Self { App { a: 3, b: 2, delta: 4, tick: 0 } }

    fn reset(&mut self) { self.tick = 0; }

    fn delta_rad(&self) -> f32 {
        self.delta as f32 * 15.0 * PI / 180.0
    }

    fn point_at(&self, step: usize) -> (i32, i32) {
        let t = step as f32 * DELTA_STEP;
        let x = (self.a as f32 * t + self.delta_rad()).sin();
        let y = (self.b as f32 * t).sin();
        let px = CX + HALF_W + (x * (HALF_W as f32 - 4.0)) as i32;
        let py = CY + HALF_H + (y * (HALF_H as f32 - 4.0)) as i32;
        (px, py)
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Lissajous Curves");
        text(200, 8, C_HINT, "Left/Right: a ratio  Up/Down: delta  R: reset  Q: quit");

        // Canvas background
        fill(CX, CY, CW, CH, 0x050810FF);
        border(CX, CY, CW, CH, C_BORDER);

        // Draw trail up to current tick
        let steps = if self.tick == 0 { TOTAL_STEPS } else { self.tick };
        for i in 0..steps {
            let (px, py) = self.point_at(i);
            let hue = i as f32 * 360.0 / TOTAL_STEPS as f32;
            let c = hue_color(hue);
            // 2×2 dot
            fill(px - 1, py - 1, 3, 3, c);
        }

        // Current position marker
        let (cpx, cpy) = self.point_at(self.tick.saturating_sub(1));
        fill(cpx - 3, cpy - 3, 7, 7, 0xFFFFFFFF);

        // Axes
        fill(CX + HALF_W, CY + 2, 1, CH - 4, 0x30363DFF);
        fill(CX + 2, CY + HALF_H, CW - 4, 1, 0x30363DFF);

        // Right info panel
        let px = CX + CW + 12;
        fill(px, CY, W - px - 4, CH, C_CARD);
        border(px, CY, W - px - 4, CH, C_BORDER);

        text(px + 8, CY + 10, C_ORANGE, "Parameters");
        text(px + 8, CY + 32, C_HINT,   "a (X freq):");
        text(px + 8, CY + 50, C_SEL,    &format!("{}", self.a));
        text(px + 8, CY + 74, C_HINT,   "b (Y freq):");
        text(px + 8, CY + 92, C_SEL,    &format!("{}", self.b));
        text(px + 8, CY + 116, C_HINT,  "a:b ratio:");
        text(px + 8, CY + 134, C_ORANGE, &format!("{}:{}", self.a, self.b));
        text(px + 8, CY + 160, C_HINT,  "delta:");
        text(px + 8, CY + 178, C_SEL,   &format!("{}°", self.delta * 15));

        text(px + 8, CY + 210, C_HINT,  "Progress:");
        let pct = self.tick * 100 / TOTAL_STEPS;
        fill(px + 8, CY + 230, 80, 8, C_BORDER);
        fill(px + 8, CY + 230, 80 * pct as i32 / 100, 8, C_SEL);

        text(px + 8, CY + 256, C_HINT,  "Controls:");
        text(px + 8, CY + 274, C_HINT,  "← → : a freq");
        text(px + 8, CY + 292, C_HINT,  "↑ ↓ : delta");
        text(px + 8, CY + 310, C_HINT,  "R : reset");
        text(px + 8, CY + 328, C_HINT,  "Q : quit");

        // Formula
        text(px + 8, CY + 360, C_ORANGE, "Formula:");
        text(px + 8, CY + 378, C_HINT,  "x=sin(a*t+d)");
        text(px + 8, CY + 396, C_HINT,  "y=sin(b*t)");

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("a={} b={} delta={}deg  step={}/{}", self.a, self.b, self.delta*15, self.tick, TOTAL_STEPS));
        flush();
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" {
            self.tick = (self.tick + 1) % TOTAL_STEPS;
            self.draw();
            return;
        }
        match line {
            "\x1b[C" => { if self.a < 8 { self.a += 1; } self.reset(); }
            "\x1b[D" => { if self.a > 1 { self.a -= 1; } self.reset(); }
            "\x1b[A" => { self.delta = (self.delta + 1) % 24; self.reset(); }
            "\x1b[B" => { self.delta = (self.delta + 23) % 24; self.reset(); }
            "r" | "R" => self.reset(),
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
        if line == "REPLY:pong" {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }
    }
}
