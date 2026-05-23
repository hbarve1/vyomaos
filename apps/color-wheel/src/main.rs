use std::f32::consts::PI;
use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const CX: i32 = 420;
const CY: i32 = 380;
const R: i32 = 200;
const CELL: i32 = 6;

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
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

fn hsv_to_rgba(h: i32, s: i32, v: i32) -> u32 {
    if s == 0 {
        let c = v as u8;
        return ((c as u32) << 24) | ((c as u32) << 16) | ((c as u32) << 8) | 0xFF;
    }
    let h6 = (h / 60) % 6;
    let f  = h % 60;
    let p  = (v * (255 - s) / 255) as u8;
    let q  = (v * (255 * 60 - s * f) / (255 * 60)) as u8;
    let t  = (v * (255 * 60 - s * (60 - f)) / (255 * 60)) as u8;
    let vb = v as u8;
    let (r, g, b): (u8, u8, u8) = match h6 {
        0 => (vb, t,  p),
        1 => (q,  vb, p),
        2 => (p,  vb, t),
        3 => (p,  q,  vb),
        4 => (t,  p,  vb),
        _ => (vb, p,  q),
    };
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
}

fn hue_offset(base: i32, offset: i32) -> i32 {
    (base + offset).rem_euclid(360)
}

fn hue_to_pos(h: i32, radius: i32) -> (i32, i32) {
    let a = h as f32 * PI / 180.0;
    (CX + (a.cos() * radius as f32) as i32, CY + (a.sin() * radius as f32) as i32)
}

fn draw_marker(cx: i32, cy: i32, col: u32) {
    for i in 0..12 {
        let a = i as f32 * PI / 6.0;
        let mx = cx + (a.cos() * 9.0) as i32;
        let my = cy + (a.sin() * 9.0) as i32;
        fill(mx - 1, my - 1, 3, 3, col);
    }
    fill(cx - 3, cy - 3, 7, 7, col);
}

#[derive(Clone, Copy, PartialEq)]
enum Mode { Complementary, Triadic, Analogous, Tetradic }

impl Mode {
    fn name(self) -> &'static str {
        match self {
            Mode::Complementary => "Complementary",
            Mode::Triadic       => "Triadic",
            Mode::Analogous     => "Analogous",
            Mode::Tetradic      => "Tetradic",
        }
    }
    fn next(self) -> Self {
        match self {
            Mode::Complementary => Mode::Triadic,
            Mode::Triadic       => Mode::Analogous,
            Mode::Analogous     => Mode::Tetradic,
            Mode::Tetradic      => Mode::Complementary,
        }
    }
    fn prev(self) -> Self {
        match self {
            Mode::Complementary => Mode::Tetradic,
            Mode::Triadic       => Mode::Complementary,
            Mode::Analogous     => Mode::Triadic,
            Mode::Tetradic      => Mode::Analogous,
        }
    }
    fn harmonies(self, base: i32) -> Vec<i32> {
        match self {
            Mode::Complementary => vec![base, hue_offset(base, 180)],
            Mode::Triadic       => vec![base, hue_offset(base, 120), hue_offset(base, 240)],
            Mode::Analogous     => vec![base, hue_offset(base, 30), hue_offset(base, -30)],
            Mode::Tetradic      => vec![base, hue_offset(base, 90), hue_offset(base, 180), hue_offset(base, 270)],
        }
    }
}

struct App {
    base_hue: i32,
    mode: Mode,
    paused: bool,
}

impl App {
    fn new() -> Self { App { base_hue: 0, mode: Mode::Complementary, paused: false } }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        fill(0, H - 24, W, 24, C_HEADER);

        text(12, 8, C_TEXT, "Color Theory Wheel");
        text(200, 8, C_HINT, &format!(
            "+/-=mode({})  Space={}  Q=quit",
            self.mode.name(), if self.paused { "PAUSED" } else { "pause" }
        ));

        // Draw color wheel (cell resolution = CELL px)
        let r_sq = (R * R) as f32;
        let mut dy = -R;
        while dy < R {
            let mut dx = -R;
            while dx < R {
                let dsq = (dx * dx + dy * dy) as f32;
                if dsq <= r_sq {
                    let dist = dsq.sqrt();
                    let angle = (dy as f32).atan2(dx as f32).to_degrees();
                    let hue = ((angle + 360.0) as i32 % 360 + self.base_hue).rem_euclid(360);
                    let sat = ((dist / R as f32) * 255.0) as i32;
                    fill(CX + dx, CY + dy, CELL, CELL, hsv_to_rgba(hue, sat.min(255), 255));
                }
                dx += CELL;
            }
            dy += CELL;
        }

        // Harmony markers on outer rim
        let harmonies = self.mode.harmonies(self.base_hue);
        for &h in &harmonies {
            let (mx, my) = hue_to_pos(h, R + 12);
            draw_marker(mx, my, 0xFFFFFFFF);
            let inner = hue_to_pos(h, R - 1);
            fill(inner.0 - 1, inner.1 - 1, 3, 3, 0xFFFFFFFF);
        }

        // Right info panel
        let px = 660i32;
        let py = 44i32;
        text(px, py, C_TEXT, "Color Theory");
        text(px, py + 20, C_HINT, &format!("Mode:  {}", self.mode.name()));
        text(px, py + 36, C_HINT, &format!("Base:  {}°", self.base_hue));

        let base_rgba = hsv_to_rgba(self.base_hue, 255, 255);
        let rr = (base_rgba >> 24) & 0xFF;
        let gg = (base_rgba >> 16) & 0xFF;
        let bb = (base_rgba >> 8) & 0xFF;
        text(px, py + 52, C_HINT, &format!("RGB:   #{:02X}{:02X}{:02X}", rr, gg, bb));

        // Swatches
        let swatch_w = 270i32;
        let swatch_h = 52i32;
        let gap = 6i32;
        for (i, &h) in harmonies.iter().enumerate() {
            let sy = py + 76 + i as i32 * (swatch_h + gap);
            let col = hsv_to_rgba(h, 255, 255);
            fill(px, sy, swatch_w, swatch_h, col);
            fill(px, sy, swatch_w, 1, 0xFFFFFFFF);
            fill(px, sy + swatch_h - 1, swatch_w, 1, 0xFFFFFFFF);
            let cr = (col >> 24) & 0xFF;
            let cg = (col >> 16) & 0xFF;
            let cb = (col >> 8) & 0xFF;
            let label = match i {
                0 => "Base",
                1 => match self.mode {
                    Mode::Complementary => "Complement",
                    Mode::Triadic       => "Triadic +120°",
                    Mode::Analogous     => "Analogous +30°",
                    Mode::Tetradic      => "Tetrad +90°",
                },
                2 => match self.mode {
                    Mode::Triadic   => "Triadic +240°",
                    Mode::Analogous => "Analogous -30°",
                    _               => "Tetrad +180°",
                },
                _ => "Tetrad +270°",
            };
            text(px + 4, sy + 6, C_BG, &format!("{}", label));
            text(px + 4, sy + 26, C_BG, &format!("{}°  #{:02X}{:02X}{:02X}", h, cr, cg, cb));
        }

        // Center dot
        fill(CX - 6, CY - 6, 13, 13, 0xFFFFFFFF);
        fill(CX - 4, CY - 4, 9, 9, base_rgba);

        text(12, H - 18, C_HINT, "HSV wheel: hue=angle sat=radius val=100% — harmony markers as white circles");
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                if !self.paused { self.base_hue = (self.base_hue + 1) % 360; }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " " => { self.paused = !self.paused; }
            "+" | "=" => { self.mode = self.mode.next(); }
            "-" => { self.mode = self.mode.prev(); }
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
