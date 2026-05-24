// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 800;
const H: i32 = 600;

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

fn rgb_color(r: u32, g: u32, b: u32) -> u32 {
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

// Convert RGB (0-255 each) to HSV (h: 0-359, s: 0-100, v: 0-100)
fn rgb_to_hsv(r: u32, g: u32, b: u32) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let cmax = rf.max(gf).max(bf);
    let cmin = rf.min(gf).min(bf);
    let delta = cmax - cmin;
    let h = if delta < 0.001 { 0.0 }
            else if (cmax - rf).abs() < 0.001 { 60.0 * (((gf - bf) / delta) % 6.0) }
            else if (cmax - gf).abs() < 0.001 { 60.0 * (((bf - rf) / delta) + 2.0) }
            else                               { 60.0 * (((rf - gf) / delta) + 4.0) };
    let h = if h < 0.0 { h + 360.0 } else { h };
    let s = if cmax < 0.001 { 0.0 } else { delta / cmax * 100.0 };
    let v = cmax * 100.0;
    (h, s, v)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u32, u32, u32) {
    let s = s / 100.0;
    let v = v / 100.0;
    let c = v * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (((r1 + m) * 255.0) as u32, ((g1 + m) * 255.0) as u32, ((b1 + m) * 255.0) as u32)
}

// Analogous hue offset color (hue ±offset degrees, same S and V)
fn analogous(r: u32, g: u32, b: u32, offset: f32) -> u32 {
    let (h, s, v) = rgb_to_hsv(r, g, b);
    let h2 = (h + offset + 360.0) % 360.0;
    let (r2, g2, b2) = hsv_to_rgb(h2, s, v);
    rgb_color(r2, g2, b2)
}

const SLIDER_W: i32 = 400;
const SLIDER_H: i32 = 24;
const SX: i32 = 200;

struct App {
    r: u32, g: u32, b: u32,
    focus: usize, // 0=R 1=G 2=B
}

impl App {
    fn new() -> Self { App { r: 100, g: 149, b: 237, focus: 0 } }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "RGB Mixer");
        text(130, 8, C_HINT, "Tab:slider  Arrows:adjust  C:copy hex  Q:quit");

        let color = rgb_color(self.r, self.g, self.b);
        let comp  = rgb_color(255 - self.r, 255 - self.g, 255 - self.b);
        let ana1  = analogous(self.r, self.g, self.b, 30.0);
        let ana2  = analogous(self.r, self.g, self.b, -30.0);

        // Color preview
        fill(20, 52, 200, 200, color);
        border(20, 52, 200, 200, C_BORDER);
        text(20, 258, C_HINT, &format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b));

        // Complementary + analogous swatches
        text(240, 52, C_ORANGE, "Complementary:");
        fill(240, 68, 80, 60, comp);
        border(240, 68, 80, 60, C_BORDER);
        text(240, 134, C_HINT, &format!("#{:02X}{:02X}{:02X}", 255-self.r, 255-self.g, 255-self.b));

        text(340, 52, C_ORANGE, "Analogous +30:");
        fill(340, 68, 80, 60, ana1);
        border(340, 68, 80, 60, C_BORDER);

        text(440, 52, C_ORANGE, "Analogous -30:");
        fill(440, 68, 80, 60, ana2);
        border(440, 68, 80, 60, C_BORDER);

        // RGB Sliders
        let sliders = [
            ("R", self.r, 0xFF0000FFu32),
            ("G", self.g, 0x00CC44FFu32),
            ("B", self.b, 0x4488FFFFu32),
        ];
        let base_y = 290i32;
        for (i, &(label, val, sc)) in sliders.iter().enumerate() {
            let sy = base_y + i as i32 * 70;
            let sel = i == self.focus;
            let lc = if sel { C_SEL } else { C_HINT };

            text(20, sy + 6, lc, label);

            // Gradient track (simplified: fill with component color)
            fill(SX, sy, SLIDER_W, SLIDER_H, C_CARD);
            border(SX, sy, SLIDER_W, SLIDER_H, if sel { C_SEL } else { C_BORDER });
            // Filled portion
            let filled = (val as i32 * SLIDER_W / 255).max(0);
            fill(SX, sy, filled, SLIDER_H, sc);

            // Thumb
            let tx = SX + filled;
            fill(tx - 2, sy - 4, 6, SLIDER_H + 8, 0xFFFFFFFF);

            // Value
            text(SX + SLIDER_W + 12, sy + 6, C_TEXT, &format!("{:3}", val));
            text(SX + SLIDER_W + 52, sy + 6, C_HINT, &format!("0x{:02X}", val));
        }

        // Hex output row
        let hy = base_y + 3 * 70 + 10;
        fill(20, hy, W - 40, 40, C_CARD);
        border(20, hy, W - 40, 40, C_BORDER);
        text(32, hy + 12, C_ORANGE, "HEX:");
        text(80, hy + 12, C_TEXT, &format!("#{:02X}{:02X}{:02X}FF", self.r, self.g, self.b));
        text(220, hy + 12, C_HINT, "  C = copy to stdout");

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        let (h, s, v) = rgb_to_hsv(self.r, self.g, self.b);
        text(12, H - 18, C_HINT, &format!("RGB({},{},{})  HSV({:.0}°,{:.0}%,{:.0}%)",
             self.r, self.g, self.b, h, s, v));
        flush();
    }

    fn adjust(&mut self, delta: i32) {
        let val = match self.focus { 0 => &mut self.r, 1 => &mut self.g, _ => &mut self.b };
        *val = (*val as i32 + delta).clamp(0, 255) as u32;
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\t" => self.focus = (self.focus + 1) % 3,
            "\x1b[C" | "\x1b[A" => self.adjust(5),
            "\x1b[D" | "\x1b[B" => self.adjust(-5),
            "+" | "=" => self.adjust(1),
            "-" => self.adjust(-1),
            "c" | "C" => {
                println!("color: #{:02X}{:02X}{:02X}FF", self.r, self.g, self.b);
                let _ = io::stdout().flush();
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
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
