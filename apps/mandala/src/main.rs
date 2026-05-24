// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const CX: i32 = 420;
const CY: i32 = 390;
const CURSOR_R: i32 = 180;
const PI: f32 = std::f32::consts::PI;

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_CANVAS: u32 = 0x0A0E14FF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_SEL:    u32 = 0x58A6FFFF;

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

fn isqrt(n: i64) -> i64 {
    if n <= 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}

fn draw_circle(cx: i32, cy: i32, r: i32, c: u32) {
    for dy in -r..=r {
        let dx = isqrt((r * r - dy * dy) as i64) as i32;
        fill(cx - dx, cy + dy, 2 * dx + 1, 1, c);
    }
}

fn draw_line(x1: i32, y1: i32, x2: i32, y2: i32, c: u32) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let steps = dx.abs().max(dy.abs()) / 2;
    if steps == 0 { return; }
    for t in 0..=steps {
        let x = x1 + dx * t / steps;
        let y = y1 + dy * t / steps;
        fill(x, y, 2, 2, c);
    }
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

#[derive(Clone, Copy, PartialEq)]
enum Kind { Dot, Line, Arc }

impl Kind {
    fn name(self) -> &'static str { match self { Kind::Dot=>"Dot", Kind::Line=>"Line", Kind::Arc=>"Arc" } }
    fn next(self) -> Self { match self { Kind::Dot=>Kind::Line, Kind::Line=>Kind::Arc, Kind::Arc=>Kind::Dot } }
    fn prev(self) -> Self { match self { Kind::Dot=>Kind::Arc, Kind::Line=>Kind::Dot, Kind::Arc=>Kind::Line } }
}

#[derive(Clone)]
struct Mark { kind: Kind, angle: f32, hue: u16 }

fn draw_mark(mark: &Mark, sym: usize) {
    let da = 2.0 * PI / sym as f32;
    let color = hsv_to_rgba(mark.hue, 220, 255);
    for i in 0..sym {
        let angle = mark.angle + i as f32 * da;
        let px = CX + (CURSOR_R as f32 * angle.cos()) as i32;
        let py = CY + (CURSOR_R as f32 * angle.sin()) as i32;
        match mark.kind {
            Kind::Dot => draw_circle(px, py, 5, color),
            Kind::Line => draw_line(CX, CY, px, py, color),
            Kind::Arc => {
                let arc_half = PI / 8.0;
                let mut a = -arc_half;
                while a <= arc_half {
                    let ta = angle + a;
                    let apx = CX + (CURSOR_R as f32 * ta.cos()) as i32;
                    let apy = CY + (CURSOR_R as f32 * ta.sin()) as i32;
                    fill(apx - 1, apy - 1, 4, 4, color);
                    a += 0.09;
                }
            }
        }
    }
}

struct App {
    marks:      Vec<Mark>,
    symmetry:   usize,
    kind:       Kind,
    base_angle: f32,
    hue_cycle:  u16,
    seed:       u64,
}

impl App {
    fn new() -> Self {
        App {
            marks: Vec::new(),
            symmetry: 6,
            kind: Kind::Dot,
            base_angle: 0.0,
            hue_cycle: 0,
            seed: 0x9E3779B97F4A7C15,
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);

        text(12, 8, C_TEXT, "Mandala Builder");
        text(160, 8, C_HINT, &format!(
            "{}-fold  {}  ←→=sym  ↑↓=type  Space=add  C=clear  Q=quit",
            self.symmetry, self.kind.name()
        ));

        // Canvas background
        draw_circle(CX, CY, CURSOR_R + 12, C_CANVAS);

        // Guide circle (dotted ring at cursor radius)
        for i in 0..90usize {
            if i % 3 == 0 { continue; }
            let a = i as f32 * 2.0 * PI / 90.0;
            let gx = CX + (CURSOR_R as f32 * a.cos()) as i32;
            let gy = CY + (CURSOR_R as f32 * a.sin()) as i32;
            fill(gx, gy, 2, 2, C_BORDER);
        }

        // Center dot
        draw_circle(CX, CY, 3, C_BORDER);

        // Symmetry guide lines (faint)
        let da = 2.0 * PI / self.symmetry as f32;
        for i in 0..self.symmetry {
            let a = i as f32 * da;
            let ex = CX + ((CURSOR_R + 12) as f32 * a.cos()) as i32;
            let ey = CY + ((CURSOR_R + 12) as f32 * a.sin()) as i32;
            // Draw faint guide just as 2 dots along the axis
            for t in 1..=3i32 {
                let fx = CX + ((CURSOR_R / 4 * t) as f32 * a.cos()) as i32;
                let fy = CY + ((CURSOR_R / 4 * t) as f32 * a.sin()) as i32;
                fill(fx, fy, 2, 2, 0x1C2128FF);
            }
            let _ = (ex, ey);
        }

        // Draw all marks
        for mark in &self.marks {
            draw_mark(mark, self.symmetry);
        }

        // Cursor indicators at current base_angle (replicated)
        for i in 0..self.symmetry {
            let angle = self.base_angle + i as f32 * da;
            let px = CX + (CURSOR_R as f32 * angle.cos()) as i32;
            let py = CY + (CURSOR_R as f32 * angle.sin()) as i32;
            fill(px - 3, py - 3, 7, 7, 0xFFD700FF);
        }

        // Right info panel
        let rx = 660i32;
        fill(rx, 32, 2, H - 32, C_BORDER);
        fill(rx + 2, 32, W - rx - 2, H - 32, 0x0A0E14FF);

        text(rx + 16, 50, C_TEXT, &format!("Symmetry: {}-fold", self.symmetry));
        text(rx + 16, 74, C_TEXT, &format!("Pattern:  {}", self.kind.name()));
        text(rx + 16, 98, C_TEXT, &format!("Marks:    {}/20", self.marks.len()));

        // Pattern selector
        text(rx + 16, 132, C_HINT, "Patterns (↑↓):");
        let kinds = [Kind::Dot, Kind::Line, Kind::Arc];
        for (i, &k) in kinds.iter().enumerate() {
            let by = 154 + i as i32 * 28;
            let sel = k == self.kind;
            if sel { fill(rx + 12, by - 2, 260, 24, C_SEL); }
            let tc = if sel { 0x0D1117FF } else { C_TEXT };
            text(rx + 16, by + 4, tc, k.name());
        }

        // Hue spectrum preview
        text(rx + 16, 250, C_HINT, "Hue palette:");
        for i in 0i32..24 {
            let hue = (i * 15) as u16;
            let col = hsv_to_rgba(hue, 220, 255);
            fill(rx + 16 + i * 11, 268, 10, 20, col);
        }

        // Current hue indicator
        let ci = (self.hue_cycle / 15) as i32;
        fill(rx + 16 + ci.min(23) * 11, 264, 10, 4, 0xFFFFFFFF);

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                self.base_angle = (self.base_angle + PI / 180.0).rem_euclid(2.0 * PI);
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " " => {
                if self.marks.len() < 20 {
                    self.marks.push(Mark {
                        kind:  self.kind,
                        angle: self.base_angle,
                        hue:   self.hue_cycle,
                    });
                    self.hue_cycle = (self.hue_cycle
                        + 15
                        + (rng(&mut self.seed) % 30) as u16)
                        % 360;
                }
            }
            "c" | "C"    => { self.marks.clear(); }
            "\x1b[C"     => { if self.symmetry < 16 { self.symmetry += 1; } }
            "\x1b[D"     => { if self.symmetry > 3  { self.symmetry -= 1; } }
            "\x1b[A"     => { self.kind = self.kind.prev(); }
            "\x1b[B"     => { self.kind = self.kind.next(); }
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
