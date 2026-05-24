// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32    = 960;
const H: i32    = 720;
const GY: i32   = 24;
const CX: i32   = 480;
const CY: i32   = 370;
const FACE_R: i32 = 280;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;
const C_FACE:   u32 = 0x161B22FF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_RED:    u32 = 0xFF7B72FF;

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
    loop { let x1 = (x + n / x) / 2; if x1 >= x { return x; } x = x1; }
}

fn draw_filled_circle(cx: i32, cy: i32, r: i32, col: u32) {
    let r2 = (r * r) as i64;
    for dy in -r..=r {
        let dx = isqrt(r2 - (dy * dy) as i64) as i32;
        fill(cx - dx, cy + dy, dx * 2 + 1, 1, col);
    }
}

fn draw_ring(cx: i32, cy: i32, r_in: i32, r_out: i32, col: u32) {
    let ri2 = (r_in * r_in) as i64;
    let ro2 = (r_out * r_out) as i64;
    for dy in -r_out..=r_out {
        let dy2 = (dy * dy) as i64;
        let ox = isqrt(ro2 - dy2) as i32;
        let ix = if dy2 < ri2 { isqrt(ri2 - dy2) as i32 } else { 0 };
        if ox > ix {
            fill(cx - ox, cy + dy, ox - ix, 1, col);
            fill(cx + ix, cy + dy, ox - ix, 1, col);
        }
    }
}

fn draw_thick_line(x0: i32, y0: i32, x1: i32, y1: i32, thick: i32, col: u32) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (((dx * dx + dy * dy) as f32).sqrt() as i32).max(1);
    let h = thick / 2;
    for i in 0..=len {
        let x = x0 + dx * i / len;
        let y = y0 + dy * i / len;
        fill(x - h, y - h, thick, thick, col);
    }
}

struct Trig {
    sin: [i32; 60],
    cos: [i32; 60],
}

impl Trig {
    fn new() -> Self {
        let mut sin = [0i32; 60];
        let mut cos = [0i32; 60];
        for i in 0..60 {
            let a = 2.0 * std::f32::consts::PI * i as f32 / 60.0;
            sin[i] = (a.sin() * 10000.0) as i32;
            cos[i] = (a.cos() * 10000.0) as i32;
        }
        Trig { sin, cos }
    }

    fn tip(&self, cx: i32, cy: i32, pos: usize, length: i32) -> (i32, i32) {
        let p = pos % 60;
        (cx + length * self.sin[p] / 10000, cy - length * self.cos[p] / 10000)
    }
}

struct App {
    hour: u32,
    min:  u32,
    sec:  u32,
    trig: Trig,
}

impl App {
    fn new() -> Self {
        App { hour: 12, min: 0, sec: 0, trig: Trig::new() }
    }

    fn tick_time(&mut self) {
        self.sec += 1;
        if self.sec >= 60 { self.sec = 0; self.min += 1; }
        if self.min >= 60 { self.min = 0; self.hour += 1; }
        if self.hour >= 24 { self.hour = 0; }
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(12, 4, C_TEXT, "Clock Tower");
        text(700, 4, C_HINT, "Q=quit");

        fill(0, GY, W, H - GY, C_BG);

        draw_filled_circle(CX, CY, FACE_R, C_FACE);
        draw_ring(CX, CY, FACE_R - 4, FACE_R + 4, C_BORDER);

        let t = &self.trig;

        // tick marks
        for i in 0usize..60 {
            let (major, len, thick) = if i % 5 == 0 { (true, 22, 3) } else { (false, 12, 1) };
            let col = if major { C_TEXT } else { C_HINT };
            let (ox, oy) = t.tip(CX, CY, i, FACE_R);
            let (ix, iy) = t.tip(CX, CY, i, FACE_R - len);
            draw_thick_line(ox, oy, ix, iy, thick, col);
        }

        // hour numbers
        const LABELS: [&str; 12] = ["12","1","2","3","4","5","6","7","8","9","10","11"];
        for i in 0usize..12 {
            let pos = i * 5;
            let (nx, ny) = t.tip(CX, CY, pos, FACE_R - 42);
            let label = LABELS[i];
            let offset = label.len() as i32 * 4;
            text(nx - offset, ny - 7, C_TEXT, label);
        }

        // hour hand
        let hour_pos = (self.hour % 12) as usize * 5 + self.min as usize / 12;
        let (hx, hy) = t.tip(CX, CY, hour_pos, 145);
        draw_thick_line(CX, CY, hx, hy, 7, C_TEXT);

        // minute hand
        let (mx, my) = t.tip(CX, CY, self.min as usize, 215);
        draw_thick_line(CX, CY, mx, my, 4, C_TEXT);

        // second hand
        let (sx, sy) = t.tip(CX, CY, self.sec as usize, 255);
        draw_thick_line(CX, CY, sx, sy, 2, C_RED);

        // center cap
        draw_filled_circle(CX, CY, 8, C_SEL);

        // time display below face
        let time_str = format!("{:02}:{:02}:{:02}", self.hour, self.min, self.sec);
        text(CX - 36, CY + FACE_R + 16, C_TEXT, &time_str);
        text(CX - 44, CY + FACE_R + 32, C_HINT, "24 May 2026");

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                self.tick_time();
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
