// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GY: i32 = 24;
const NCOLS: usize = 40;
const CELL_W: i32 = W / NCOLS as i32;
const CELL_H: i32 = 16;
const ROWS: i32 = (H - GY) / CELL_H;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn printable(s: u64) -> &'static str {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()[]{}|<>";
    let idx = ((s >> 32) as usize) % CHARS.len();
    // SAFETY: all bytes in CHARS are valid ASCII
    std::str::from_utf8(&CHARS[idx..=idx]).unwrap_or("A")
}

fn trail_color(pos: u8, len: u8) -> u32 {
    if pos == 0 {
        return 0xDDFFDDFF;
    }
    let t = (pos as f32) / (len as f32).max(1.0);
    let g = (210.0 * (1.0 - t) + 40.0 * t) as u8;
    let r = (20.0 * (1.0 - t) + 5.0 * t) as u8;
    let b = (30.0 * (1.0 - t) + 8.0 * t) as u8;
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
}

struct Column {
    head:     i32,
    speed:    u8,
    len:      u8,
    seed:     u64,
    sub_tick: u8,
}

struct App {
    cols: Vec<Column>,
    rng:  u64,
}

impl App {
    fn new() -> Self {
        let mut rng: u64 = 0xDEADBEEF_CAFEBABE;
        let cols = (0..NCOLS).map(|i| {
            rng = lcg(rng);
            let speed = 1 + (rng % 3) as u8;
            rng = lcg(rng);
            let len = 10 + (rng % 16) as u8;
            rng = lcg(rng);
            let head = -((rng % 35) as i32) - 5;
            let seed = rng.wrapping_add(i as u64 * 0x9E3779B97F4A7C15);
            Column { head, speed, len, seed, sub_tick: 0 }
        }).collect();
        App { cols, rng }
    }

    fn step(&mut self) {
        let mut rng = self.rng;
        for col in self.cols.iter_mut() {
            col.sub_tick += 1;
            if col.sub_tick >= col.speed {
                col.sub_tick = 0;
                col.head += 1;
                col.seed = lcg(col.seed);
                if col.head - col.len as i32 > ROWS {
                    rng = lcg(rng);
                    col.head = -((rng % 25) as i32) - 5;
                    rng = lcg(rng);
                    col.speed = 1 + (rng % 3) as u8;
                    rng = lcg(rng);
                    col.len = 10 + (rng % 16) as u8;
                    rng = lcg(rng);
                    col.seed = rng;
                }
            }
        }
        self.rng = rng;
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(12, 4, C_TEXT, "Pixel Rain");
        text(700, 4, C_HINT, "Q=quit");

        fill(0, GY, W, H - GY, C_BG);

        for (ci, col) in self.cols.iter().enumerate() {
            let cx = ci as i32 * CELL_W;
            let mut seed = col.seed;
            for j in 0..col.len as i32 {
                let row = col.head - j;
                if row >= 0 && row < ROWS {
                    let py = GY + row * CELL_H;
                    let color = trail_color(j as u8, col.len);
                    let ch = printable(seed);
                    text(cx + 4, py + 1, color, ch);
                }
                seed = lcg(seed);
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
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.render();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.render();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
