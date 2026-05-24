// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_RED:    u32 = 0xFF7B72FF;
const C_CARD:   u32 = 0x161B22FF;

const KB_ROWS: [&str; 3] = ["QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"];

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

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

fn heat_color(val: u32, max_val: u32) -> u32 {
    if max_val == 0 { return C_CARD; }
    let t = (val * 255 / max_val).min(255) as u32;
    let r = (0x16u32 + (0xFF - 0x16) * t / 255) as u8;
    let g = (0x1Bu32 + (0xA6 - 0x1B) * t / 255) as u8;
    let b = (0x22u32 + (0x57 - 0x22) * t / 255) as u8;
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
}

struct App {
    panel: usize,
    wpm: [u32; 20],
    freq: [u32; 26],
    row_errors: [u32; 3],
    seed: u64,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            panel: 0,
            wpm: [0; 20],
            freq: [0; 26],
            row_errors: [0; 3],
            seed: 0xDEAD_CAFE_1234_5678u64,
        };
        app.randomize();
        app
    }

    fn randomize(&mut self) {
        for i in 0..20usize {
            self.seed = lcg(self.seed);
            self.wpm[i] = 35 + (self.seed % 86) as u32;
        }
        for i in 0..26usize {
            self.seed = lcg(self.seed);
            self.freq[i] = 10 + (self.seed % 120) as u32;
        }
        for &i in &[4usize, 19, 0, 14, 8, 17, 18, 7, 11, 3] {
            self.seed = lcg(self.seed);
            self.freq[i] += 40 + (self.seed % 80) as u32;
        }
        for i in 0..3usize {
            self.seed = lcg(self.seed);
            self.row_errors[i] = 5 + (self.seed % 35) as u32;
        }
    }

    fn draw_wpm_panel(&self) {
        text(40, 48, C_TEXT, "WPM History — last 20 sessions");

        let bx0: i32 = 40;
        let by_bot: i32 = 560;
        let max_bh: i32 = 460;
        let bar_w: i32 = 38;
        let gap: i32 = 6;

        let max_wpm = *self.wpm.iter().max().unwrap_or(&1);
        let avg = self.wpm.iter().sum::<u32>() / 20;

        let avg_y = by_bot - (avg * max_bh as u32 / max_wpm) as i32;
        fill(bx0, avg_y, (bar_w + gap) * 20 - gap, 1, C_SEL);
        text(bx0 + (bar_w + gap) * 20 + 2, avg_y - 6, C_SEL, &format!("avg {}", avg));

        for i in 0..20usize {
            let bh = ((self.wpm[i] * max_bh as u32 / max_wpm) as i32).max(2);
            let bx = bx0 + i as i32 * (bar_w + gap);
            let by = by_bot - bh;
            let col = if self.wpm[i] >= 80 { C_GREEN }
                      else if self.wpm[i] >= 50 { C_ORANGE }
                      else { C_RED };
            fill(bx, by, bar_w, bh, col);
            text(bx + 5, by - 15, col, &self.wpm[i].to_string());
            if i % 5 == 0 {
                text(bx, by_bot + 4, C_HINT, &(i + 1).to_string());
            }
        }

        let lx = bx0;
        let ly = by_bot + 22;
        fill(lx,       ly, 20, 8, C_GREEN);
        text(lx + 24,  ly - 4, C_HINT, "80+ WPM");
        fill(lx + 100, ly, 20, 8, C_ORANGE);
        text(lx + 124, ly - 4, C_HINT, "50-79");
        fill(lx + 190, ly, 20, 8, C_RED);
        text(lx + 214, ly - 4, C_HINT, "<50 WPM");
    }

    fn draw_heatmap_panel(&self) {
        text(40, 48, C_TEXT, "Letter Frequency Heatmap — QWERTY layout");

        let max_freq = *self.freq.iter().max().unwrap_or(&1);
        let kw: i32 = 68;
        let kh: i32 = 56;
        let kg: i32 = 6;
        let offsets: [i32; 3] = [40, 78, 118];
        let base_y: i32 = 90;

        for (row, &row_str) in KB_ROWS.iter().enumerate() {
            let ox = offsets[row];
            let oy = base_y + row as i32 * (kh + kg);
            for (col, ch) in row_str.chars().enumerate() {
                let kx = ox + col as i32 * (kw + kg);
                let li = (ch as u8 - b'A') as usize;
                let bg = heat_color(self.freq[li], max_freq);
                fill(kx, oy, kw, kh, bg);
                fill(kx, oy, kw, 1, C_BORDER);
                fill(kx, oy + kh - 1, kw, 1, C_BORDER);
                fill(kx, oy, 1, kh, C_BORDER);
                fill(kx + kw - 1, oy, 1, kh, C_BORDER);
                text(kx + 26, oy + 10, C_TEXT, &ch.to_string());
                text(kx + 8, oy + 34, C_HINT, &self.freq[li].to_string());
            }
        }

        let lx = 40i32;
        let ly = H - 90;
        text(lx, ly - 20, C_HINT, "Low frequency");
        for i in 0..240i32 {
            let col = heat_color(i as u32, 240);
            fill(lx + i, ly, 1, 16, col);
        }
        text(lx + 244, ly - 20, C_HINT, "High frequency");
    }

    fn draw_errors_panel(&self) {
        text(40, 48, C_TEXT, "Error Rate by Keyboard Row");

        let bx: i32 = 120;
        let bh: i32 = 44;
        let bg: i32 = 20;
        let max_w: i32 = W - 260;
        let names = ["Top row  (Q W E R T ...)", "Home row (A S D F G ...)", "Bottom   (Z X C V B ...)"];
        let cols = [C_ORANGE, C_SEL, C_GREEN];
        let max_err = *self.row_errors.iter().max().unwrap_or(&1);

        let mut y = 90i32;
        for i in 0..3usize {
            let fw = (self.row_errors[i] * max_w as u32 / max_err) as i32;
            fill(bx, y, fw, bh, cols[i]);
            fill(bx + fw, y, max_w - fw, bh, C_CARD);
            fill(bx, y, max_w, 1, C_BORDER);
            fill(bx, y + bh, max_w, 1, C_BORDER);
            text(bx - 116, y + 14, C_HINT, names[i]);
            text(bx + fw + 8, y + 14, cols[i], &format!("{} errors", self.row_errors[i]));
            y += bh + bg;
        }

        y += 20;
        text(bx, y, C_TEXT, "Individual key error index (lower = more accurate):");
        y += 24;

        let max_freq = *self.freq.iter().max().unwrap_or(&1);
        for i in 0..26usize {
            let col = i as i32 % 13;
            let row_i = i as i32 / 13;
            let lx = bx + col * 64;
            let ly = y + row_i * 36;
            let ch = (b'A' + i as u8) as char;
            let err = 100u32.saturating_sub(self.freq[i] * 70 / max_freq);
            let c = if err > 60 { C_RED } else if err > 30 { C_ORANGE } else { C_GREEN };
            fill(lx, ly, 56, 28, C_CARD);
            text(lx + 4, ly + 6, c, &format!("{}: {:2}", ch, err));
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        fill(0, H - 24, W, 24, C_HEADER);

        let names = ["WPM History", "Key Heatmap", "Error Rates"];
        text(12, 8, C_TEXT, &format!("Typing Stats — {}", names[self.panel]));
        text(300, 8, C_HINT, "Tab=next panel  R=randomize  Q=quit");

        for i in 0..3usize {
            let dot_x = W - 44 + i as i32 * 14;
            fill(dot_x, 11, 10, 10, if i == self.panel { C_SEL } else { C_BORDER });
        }

        match self.panel {
            0 => self.draw_wpm_panel(),
            1 => self.draw_heatmap_panel(),
            _ => self.draw_errors_panel(),
        }

        text(12, H - 18, C_HINT, "Simulated typing statistics — Tab to switch panels");
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            "\t" => { self.panel = (self.panel + 1) % 3; }
            "r" | "R" => { self.randomize(); }
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
