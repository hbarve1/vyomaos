// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::f32::consts::PI;
use std::io::{self, BufRead, Write};

const W: i32 = 800;
const H: i32 = 720;

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

fn gcd(a: u32, b: u32) -> u32 { if b == 0 { a } else { gcd(b, a % b) } }

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

struct Preset {
    name:    &'static str,
    r_big:   u32,
    r_small: u32,
    d:       f32,
    is_epi:  bool,
}

const PRESETS: [Preset; 5] = [
    Preset { name: "Flower",  r_big: 5, r_small: 3, d: 2.4, is_epi: false },
    Preset { name: "Star",    r_big: 5, r_small: 2, d: 1.8, is_epi: false },
    Preset { name: "Rings",   r_big: 7, r_small: 3, d: 2.1, is_epi: false },
    Preset { name: "Oval",    r_big: 7, r_small: 4, d: 2.8, is_epi: false },
    Preset { name: "Galaxy",  r_big: 7, r_small: 2, d: 4.0, is_epi: true  },
];

fn total_steps(p: &Preset) -> usize {
    let g = gcd(p.r_big, p.r_small);
    (p.r_big / g) as usize * 40
}

fn amplitude(p: &Preset) -> f32 {
    let arm = if p.is_epi { p.r_big + p.r_small } else { p.r_big.saturating_sub(p.r_small) };
    arm as f32 + p.d
}

fn point_at(step: usize, total: usize, p: &Preset) -> (f32, f32) {
    let revs = (p.r_big / gcd(p.r_big, p.r_small)) as f32;
    let t = step as f32 / total as f32 * 2.0 * PI * revs;
    if p.is_epi {
        let sum = (p.r_big + p.r_small) as f32;
        let ratio = sum / p.r_small as f32;
        let x = sum * t.cos() - p.d * (ratio * t).cos();
        let y = sum * t.sin() - p.d * (ratio * t).sin();
        (x, y)
    } else {
        let diff = (p.r_big - p.r_small) as f32;
        let ratio = diff / p.r_small as f32;
        let x = diff * t.cos() + p.d * (ratio * t).cos();
        let y = diff * t.sin() - p.d * (ratio * t).sin();
        (x, y)
    }
}

struct App {
    preset: usize,
    step:   usize,
    speed:  usize,
}

impl App {
    fn new() -> Self { App { preset: 0, step: 0, speed: 2 } }

    fn total(&self) -> usize { total_steps(&PRESETS[self.preset]) }

    fn draw(&self) {
        const CX: i32 = 320;
        const CY: i32 = 376;
        const CANVAS_R: f32 = 300.0;

        let p = &PRESETS[self.preset];
        let scale = CANVAS_R / amplitude(p);
        let total = self.total();

        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Spirograph");
        text(160, 8, C_HINT, "←→:preset  +/-:speed  Q:quit");

        // Canvas
        fill(0, 32, 642, H - 56, C_CARD);
        border(0, 32, 642, H - 56, C_BORDER);

        // Control panel
        let px = 644i32;
        fill(px, 32, W - px, H - 56, C_CARD);
        border(px, 32, W - px, H - 56, C_BORDER);
        text(px + 8, 40, C_ORANGE, p.name);
        text(px + 8, 58, C_HINT, &format!("Preset {}/5", self.preset + 1));
        text(px + 8, 78, C_HINT, &format!("R = {}", p.r_big));
        text(px + 8, 96, C_HINT, &format!("r = {}", p.r_small));
        text(px + 8, 114, C_HINT, &format!("d = {:.1}", p.d));
        text(px + 8, 132, C_HINT, if p.is_epi { "Epitrochoid" } else { "Hypotrochoid" });
        text(px + 8, 152, C_HINT, &format!("Speed: {}x", self.speed));

        let pct = self.step * 100 / total.max(1);
        text(px + 8, 172, C_HINT, &format!("Progress: {}%", pct));
        fill(px + 8, 188, 136, 8, C_BORDER);
        fill(px + 8, 188, 136 * pct as i32 / 100, 8, C_SEL);

        text(px + 8, 210, C_HINT, "Presets:");
        for i in 0..5usize {
            let by = 226 + i as i32 * 18;
            let (label_c, marker) = if i == self.preset { (C_SEL, ">") } else { (C_HINT, " ") };
            text(px + 8, by, label_c, &format!("{} {}", marker, PRESETS[i].name));
        }

        // Draw spirograph points
        for i in 0..self.step {
            let (fx, fy) = point_at(i, total, p);
            let px_i = CX + (fx * scale) as i32;
            let py_i = CY + (fy * scale) as i32;
            if px_i >= 2 && px_i < 640 && py_i >= 34 && py_i < H - 26 {
                let hue = i as f32 / total as f32 * 360.0;
                fill(px_i - 1, py_i - 1, 3, 3, hue_color(hue));
            }
        }

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
            &format!("{} | {}/{} pts | {}x speed", p.name, self.step, total, self.speed));
        flush();
    }

    fn tick(&mut self) {
        let total = self.total();
        self.step += self.speed;
        if self.step > total { self.step = 0; }
        self.draw();
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" { self.tick(); return; }
        match line {
            "\x1b[C" => { self.preset = (self.preset + 1) % 5; self.step = 0; self.draw(); }
            "\x1b[D" => { self.preset = (self.preset + 4) % 5; self.step = 0; self.draw(); }
            "+" | "=" => { if self.speed < 8 { self.speed += 1; } self.draw(); }
            "-" => { if self.speed > 1 { self.speed -= 1; } self.draw(); }
            "r" | "R" => { self.step = 0; self.draw(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
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
