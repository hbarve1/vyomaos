// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GRAVITY: f32 = 0.15;

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;

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

fn rng(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *s >> 33
}

fn hsv_to_rgba(h: u16, s: u8, v: u8) -> u32 {
    let h6 = (h / 60) as i32;
    let f  = (h % 60) as i32;
    let sv = s as i32;
    let vv = v as i32;
    let p  = (vv * (255 - sv) / 255) as u8;
    let q  = (vv * (255 - sv * f / 60) / 255) as u8;
    let t  = (vv * (255 - sv * (60 - f) / 60) / 255) as u8;
    let (r, g, b) = match h6 % 6 {
        0 => (v, t, p), 1 => (q, v, p), 2 => (p, v, t),
        3 => (p, q, v), 4 => (t, p, v), _ => (v, p, q),
    };
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
}

struct Particle {
    x: f32, y: f32,
    vx: f32, vy: f32,
    hue: u16,
    age: u32,
    max_age: u32,
}

struct App {
    particles:         Vec<Particle>,
    seed:              u64,
    hue_cycle:         u16,
    ticks_since_burst: u32,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            particles: Vec::new(),
            seed: 0xABCDEF0123456789,
            hue_cycle: 0,
            ticks_since_burst: 999,
        };
        app.launch(W as f32 / 2.0, H as f32 / 3.0);
        app
    }

    fn launch(&mut self, cx: f32, cy: f32) {
        let count = 60 + (rng(&mut self.seed) % 61) as usize;
        let base_hue = self.hue_cycle;
        self.hue_cycle = (self.hue_cycle + 40 + (rng(&mut self.seed) % 40) as u16) % 360;
        for _ in 0..count {
            let angle_deg = (rng(&mut self.seed) % 360) as f32;
            let angle_rad = angle_deg * std::f32::consts::PI / 180.0;
            let speed = 1.5 + (rng(&mut self.seed) % 45) as f32 / 10.0;
            let hue = (base_hue + (rng(&mut self.seed) % 60) as u16) % 360;
            let max_age = 40 + (rng(&mut self.seed) % 30) as u32;
            self.particles.push(Particle {
                x: cx, y: cy,
                vx: angle_rad.cos() * speed,
                vy: angle_rad.sin() * speed - 2.5,
                hue,
                age: 0,
                max_age,
            });
        }
        self.ticks_since_burst = 0;
    }

    fn step(&mut self) {
        self.ticks_since_burst = self.ticks_since_burst.saturating_add(1);
        if self.ticks_since_burst >= 60 && self.particles.is_empty() {
            let x = 150.0 + (rng(&mut self.seed) % 660) as f32;
            let y = 120.0 + (rng(&mut self.seed) % 250) as f32;
            self.launch(x, y);
        }
        let mut i = 0;
        while i < self.particles.len() {
            let p = &mut self.particles[i];
            p.vy += GRAVITY;
            p.x  += p.vx;
            p.y  += p.vy;
            p.age += 1;
            if p.age >= p.max_age
                || p.y > H as f32 + 20.0
                || p.x < -20.0
                || p.x > W as f32 + 20.0
            {
                self.particles.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);

        text(12, 8, C_TEXT, "Particle Fireworks");
        text(200, 8, C_HINT, &format!(
            "Space=burst  Q=quit    {} particles",
            self.particles.len()
        ));

        for p in &self.particles {
            let fade = (p.max_age - p.age) * 255 / p.max_age;
            let v = fade as u8;
            if v < 12 { continue; }
            let color = hsv_to_rgba(p.hue, 230, v);
            let px = p.x as i32;
            let py = p.y as i32;
            if px >= 0 && px < W - 1 && py >= 33 && py < H - 1 {
                fill(px, py, 2, 2, color);
            }
        }
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                self.step();
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " " => {
                let x = 150.0 + (rng(&mut self.seed) % 660) as f32;
                let y = 120.0 + (rng(&mut self.seed) % 250) as f32;
                self.launch(x, y);
            }
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
