// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 880;
const H: u32 = 680;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_CARD: u32    = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;

const HARMONY_NAMES: [&str; 5] = [
    "Analogous", "Complementary", "Triadic", "Split-Comp", "Tetradic",
];

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn hsv_to_rgb(h: u32, s: u32, v: u32) -> u32 {
    // h: 0-359, s: 0-100, v: 0-100
    // Returns RGBA u32
    let s = s * 255 / 100;
    let v = v * 255 / 100;
    if s == 0 {
        return (v << 24) | (v << 16) | (v << 8) | 0xFF;
    }
    let h6 = h * 6;
    let hi = (h6 / 360) % 6;
    let f  = h6 % 360;
    let p  = v * (255 - s) / 255;
    let q  = v * (255 * 360 - s * f) / (255 * 360);
    let t  = v * (255 * 360 - s * (360 - f)) / (255 * 360);
    let (r, g, b) = match hi {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

fn hue_offsets(mode: usize) -> &'static [i32] {
    match mode {
        0 => &[-30, 0, 30],           // Analogous
        1 => &[0, 180],               // Complementary
        2 => &[0, 120, 240],          // Triadic
        3 => &[0, 150, 210],          // Split-Comp
        _ => &[0, 90, 180, 270],      // Tetradic
    }
}

struct App {
    hue:      u32,
    harmony:  usize,
    sel:      usize,
    hue_buf:  [u8; 3],
    hue_blen: usize,
}

impl App {
    fn new() -> Self {
        App { hue: 200, harmony: 0, sel: 0, hue_buf: [0; 3], hue_blen: 0 }
    }

    fn palette(&self) -> Vec<u32> {
        let offsets = hue_offsets(self.harmony);
        offsets.iter().map(|&off| {
            let h = ((self.hue as i32 + off).rem_euclid(360)) as u32;
            hsv_to_rgb(h, 85, 90)
        }).collect()
    }

    fn push_digit(&mut self, d: u8) {
        if self.hue_blen < 3 {
            self.hue_buf[self.hue_blen] = d;
            self.hue_blen += 1;
        }
        // Parse current buffer
        let mut val = 0u32;
        for i in 0..self.hue_blen {
            val = val * 10 + self.hue_buf[i] as u32;
        }
        if val <= 359 { self.hue = val; }
        if self.hue_blen == 3 { self.hue_blen = 0; }
    }
}

const SWATCH_W: u32 = 160;
const SWATCH_H: u32 = 180;
const SWATCH_GAP: u32 = 20;
const SWATCH_Y: u32 = HEADER_H + 80;

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Color Palette");
    text(200, 16, C_HINT, "+/-:hue  digits:type hue  Tab:harmony  c:clipboard");

    // Hue input display
    let base_color = hsv_to_rgb(app.hue, 85, 90);
    let hue_str = format!("Hue: {}°", app.hue);
    fill(16, HEADER_H + 12, 120, 28, base_color);
    text(144, HEADER_H + 18, C_TEXT, &hue_str);

    // Harmony mode tabs
    for (i, &name) in HARMONY_NAMES.iter().enumerate() {
        let tx = 300 + i as u32 * 116;
        let is_sel = i == app.harmony;
        fill(tx, HEADER_H + 10, 108, 28, if is_sel { C_CARD } else { C_BG });
        border(tx, HEADER_H + 10, 108, 28, if is_sel { C_SEL } else { C_BORDER });
        text(tx + 8, HEADER_H + 18, if is_sel { C_SEL } else { C_HINT }, name);
    }

    // Color swatches
    let palette = app.palette();
    let n = palette.len() as u32;
    let total_w = n * SWATCH_W + (n - 1) * SWATCH_GAP;
    let start_x = (W - total_w) / 2;

    for (i, &color) in palette.iter().enumerate() {
        let sx = start_x + i as u32 * (SWATCH_W + SWATCH_GAP);
        let is_sel = i == app.sel;

        fill(sx, SWATCH_Y, SWATCH_W, SWATCH_H, color);
        if is_sel {
            border(sx - 3, SWATCH_Y - 3, SWATCH_W + 6, SWATCH_H + 6, C_SEL);
        } else {
            border(sx, SWATCH_Y, SWATCH_W, SWATCH_H, 0x30363DFF);
        }

        // Hex label
        let r = (color >> 24) & 0xFF;
        let g = (color >> 16) & 0xFF;
        let b = (color >>  8) & 0xFF;
        let hex = format!("#{:02X}{:02X}{:02X}", r, g, b);
        text(sx + (SWATCH_W - 56) / 2, SWATCH_Y + SWATCH_H + 8, C_TEXT, &hex);

        // RGB values
        text(sx + 8, SWATCH_Y + SWATCH_H + 26, C_HINT, &format!("R:{} G:{} B:{}", r, g, b));

        // Hue label
        let offsets = hue_offsets(app.harmony);
        let h = ((app.hue as i32 + offsets[i]).rem_euclid(360)) as u32;
        text(sx + SWATCH_W / 2 - 20, SWATCH_Y + SWATCH_H + 44, C_HINT, &format!("{}°", h));
    }

    // Harmony description
    let desc = match app.harmony {
        0 => "Analogous: colors adjacent on the color wheel (+/-30°)",
        1 => "Complementary: opposite colors on the wheel (+180°)",
        2 => "Triadic: three evenly spaced colors (+120°/+240°)",
        3 => "Split-Complementary: base + two adjacent to complement",
        _ => "Tetradic: four colors evenly spaced (rectangle)",
    };
    text(16, SWATCH_Y + SWATCH_H + 80, C_HINT, desc);

    // Selected swatch indicator
    text(16, SWATCH_Y + SWATCH_H + 100, C_HINT,
         &format!("←→:select swatch  c:copy #{:02X}{:02X}{:02X} to clipboard",
                  (palette[app.sel] >> 24) & 0xFF,
                  (palette[app.sel] >> 16) & 0xFF,
                  (palette[app.sel] >>  8) & 0xFF));

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise color-gen");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "+" | "=" => { app.hue = (app.hue + 1) % 360; }
            "-"       => { app.hue = (app.hue + 359) % 360; }
            "\x1b[C"  => {
                let n = hue_offsets(app.harmony).len();
                app.sel = (app.sel + 1) % n;
            }
            "\x1b[D"  => {
                let n = hue_offsets(app.harmony).len();
                app.sel = (app.sel + n - 1) % n;
            }
            "\t" => {
                app.harmony = (app.harmony + 1) % 5;
                let n = hue_offsets(app.harmony).len();
                if app.sel >= n { app.sel = 0; }
            }
            "c" | "C" => {
                let palette = app.palette();
                let color = palette[app.sel];
                let hex = format!("#{:02X}{:02X}{:02X}",
                    (color >> 24) & 0xFF, (color >> 16) & 0xFF, (color >> 8) & 0xFF);
                println!("@supervisor: clipboard-set {}", hex);
                let _ = io::stdout().flush();
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= b'0' && b <= b'9' { app.push_digit(b - b'0'); }
            }
            _ => {}
        }
        draw(&app);
    }
}
