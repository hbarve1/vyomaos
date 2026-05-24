// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_RED: u32    = 0xFF7B72FF;
const C_PURPLE: u32 = 0xBC8CFFFF;

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

// First 40 Fibonacci numbers
fn fibs(n: usize) -> Vec<u64> {
    let mut v = vec![0u64; n];
    if n >= 1 { v[0] = 1; }
    if n >= 2 { v[1] = 1; }
    for i in 2..n { v[i] = v[i-1].saturating_add(v[i-2]); }
    v
}

fn bar_color(i: usize) -> u32 {
    match i % 6 {
        0 => C_SEL,
        1 => C_GREEN,
        2 => C_ORANGE,
        3 => C_PURPLE,
        4 => C_RED,
        _ => C_TEXT,
    }
}

enum View { Bar, Table, Spiral }

struct App {
    view:    View,
    zoom:    i32,  // bar chart zoom: height scale factor ×1..×8
    tick:    u64,
    hi_bar:  usize, // animated highlight index
}

impl App {
    fn new() -> Self { App { view: View::Bar, zoom: 4, tick: 0, hi_bar: 0 } }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Fibonacci Visualizer");
        text(260, 8, C_HINT, "Tab:view  +/-:zoom(bar)  Q:quit");

        // View tab bar
        let tabs = [("Bar Chart", matches!(self.view, View::Bar)),
                    ("Table",     matches!(self.view, View::Table)),
                    ("Spiral",    matches!(self.view, View::Spiral))];
        for (i, &(label, active)) in tabs.iter().enumerate() {
            let tx = 700 + i as i32 * 90;
            if active {
                fill(tx - 4, 4, label.len() as i32 * 8 + 8, 24, C_CARD);
                border(tx - 4, 4, label.len() as i32 * 8 + 8, 24, C_SEL);
                text(tx, 10, C_SEL, label);
            } else {
                text(tx, 10, C_HINT, label);
            }
        }

        match self.view {
            View::Bar    => self.draw_bar(),
            View::Table  => self.draw_table(),
            View::Spiral => self.draw_spiral(),
        }

        fill(0, H - 24, W, 24, C_HEADER);
        let vname = match self.view { View::Bar => "Bar", View::Table => "Table", View::Spiral => "Spiral" };
        text(12, H - 18, C_HINT, &format!("View: {}  |  Zoom: {}x  |  Tick: {}  |  φ ≈ 1.618",
            vname, self.zoom, self.tick));
        flush();
    }

    fn draw_bar(&self) {
        let fibs = fibs(20);
        let max_val = *fibs.iter().max().unwrap_or(&1) as i32;
        let chart_h = 520i32;
        let chart_y = 36i32;
        let bar_w  = 36i32;
        let bar_gap = 10i32;
        let chart_x = (W - (bar_w + bar_gap) * 20) / 2;

        fill(chart_x - 12, chart_y, (bar_w + bar_gap) * 20 + 24, chart_h, C_CARD);
        border(chart_x - 12, chart_y, (bar_w + bar_gap) * 20 + 24, chart_h, C_BORDER);

        // Baseline
        fill(chart_x - 8, chart_y + chart_h - 28, (bar_w + bar_gap) * 20 + 16, 1, C_BORDER);

        for (i, &val) in fibs.iter().enumerate() {
            let bar_h = if max_val > 0 {
                ((val as i32) * (chart_h - 48) * self.zoom / max_val / 8).min(chart_h - 48)
            } else { 0 };
            let bx = chart_x + i as i32 * (bar_w + bar_gap);
            let by = chart_y + chart_h - 28 - bar_h;
            let color = bar_color(i);
            let is_hi = i == self.hi_bar;
            let bar_color_actual = if is_hi { C_TEXT } else { color };

            fill(bx, by, bar_w, bar_h, bar_color_actual);
            if is_hi {
                border(bx, by, bar_w, bar_h, C_ORANGE);
            }

            // Index label
            text(bx + bar_w / 2 - 4, chart_y + chart_h - 20, C_HINT, &format!("{}", i + 1));

            // Value above bar (only if bar is tall enough)
            if bar_h > 18 {
                let val_s = if val >= 1000 {
                    format!("{}k", val / 1000)
                } else {
                    format!("{}", val)
                };
                text(bx + 2, by + 2, C_BG, &val_s);
            }
        }

        // Ratio annotation for current highlight
        if self.hi_bar > 0 && self.hi_bar < 20 {
            let a = fibs[self.hi_bar - 1];
            let b = fibs[self.hi_bar];
            if a > 0 {
                let ratio_int  = b / a;
                let ratio_frac = (b * 1000 / a) % 1000;
                let ratio_str = format!("F({})/{}: {}.{:03}",
                    self.hi_bar + 1, self.hi_bar, ratio_int, ratio_frac);
                text(12, chart_y + chart_h + 4, C_ORANGE, &ratio_str);
                let diff = if ratio_frac >= 618 { ratio_frac - 618 } else { 618 - ratio_frac };
                text(300, chart_y + chart_h + 4, C_HINT,
                    &format!("diff from φ(1.618): 0.{:03}", diff));
            }
        }

        text(12, 40, C_HINT, &format!("First 20 Fibonacci numbers  |  zoom: {}x  (+/- to adjust)", self.zoom));
    }

    fn draw_table(&self) {
        let fibs = fibs(30);
        let col_x  = [48i32, 200, 460, 700];
        let hdr_y  = 46i32;
        let row_h  = 20i32;
        let n_rows = 30i32;

        fill(32, 36, W - 64, n_rows * row_h + 24, C_CARD);
        border(32, 36, W - 64, n_rows * row_h + 24, C_BORDER);

        // Header
        fill(32, 36, W - 64, 18, C_HEADER);
        text(col_x[0], hdr_y - 10, C_HINT, "n");
        text(col_x[1], hdr_y - 10, C_HINT, "F(n)");
        text(col_x[2], hdr_y - 10, C_HINT, "F(n)/F(n-1)");
        text(col_x[3], hdr_y - 10, C_HINT, "Convergence to φ");

        for (i, &val) in fibs.iter().enumerate() {
            let ry = hdr_y + i as i32 * row_h;
            let is_hi = i == self.hi_bar % 30;
            let row_c = if is_hi { C_HEADER } else { C_CARD };
            fill(32, ry, W - 64, row_h, row_c);

            let nc = if is_hi { C_SEL } else { C_TEXT };
            text(col_x[0], ry + 2, nc, &format!("{}", i + 1));
            text(col_x[1], ry + 2, nc, &format!("{}", val));

            if i > 0 && fibs[i - 1] > 0 {
                let prev = fibs[i - 1];
                let ratio_int  = val / prev;
                let ratio_frac = (val * 1000 / prev) % 1000;
                text(col_x[2], ry + 2, nc, &format!("{}.{:03}", ratio_int, ratio_frac));

                let frac = (val * 1000 / prev) % 1000;
                let diff = if frac >= 618 { frac - 618 } else { 618 - frac };
                let bar_w = (1000i32.saturating_sub(diff as i32) * 200 / 1000).max(0);
                fill(col_x[3], ry + 4, bar_w, 10, if diff < 10 { C_GREEN } else { C_ORANGE });
                text(col_x[3] + 210, ry + 2, C_HINT, &format!("±0.{:03}", diff));
            }
        }

        text(32, 36 + n_rows * row_h + 8, C_HINT,
            "φ = (1 + √5) / 2 ≈ 1.6180339...  convergence bar = proximity to φ");
    }

    fn draw_spiral(&self) {
        // Draw golden-ratio spiral as nested rectangles
        // Fibonacci squares tiling: each square's side = F(n)
        // Scale so F(12) = ~60px max
        let fibs = fibs(12);
        let scale = 40i32;
        let cx = W / 2;
        let cy = (H - 24) / 2 + 36 / 2;

        fill(32, 36, W - 64, H - 60 - 36, C_CARD);
        border(32, 36, W - 64, H - 60 - 36, C_BORDER);

        text(40, 44, C_HINT, "Golden Ratio Spiral  —  each square side = F(n)");

        // Place squares: start at center with F(1)=1, expand outward
        // Tiling directions: right, up, left, down (cycle of 4)
        let mut ox = cx;
        let mut oy = cy;
        for i in 0..12 {
            let side = (fibs[i] as i32 * scale).max(2);
            let color = bar_color(i);
            let dir = i % 4;
            let (sx, sy) = match dir {
                0 => (ox, oy - side),          // right (draw above current)
                1 => (ox - side, oy - side),   // up
                2 => (ox - side, oy),           // left
                _ => (ox, oy),                  // down
            };
            border(sx, sy, side, side, color);

            // Label square with F(n)
            let label = format!("{}", fibs[i]);
            let lx = sx + (side - label.len() as i32 * 8) / 2;
            let ly = sy + (side - 16) / 2;
            if side >= 16 {
                text(lx.max(sx + 2), ly.max(sy + 2), color, &label);
            }

            // Animated tick highlight
            if i == self.tick as usize % 12 {
                fill(sx + 1, sy + 1, side - 2, side - 2,
                    color & 0xFFFFFF33);
            }

            // Advance origin for next square
            match dir {
                0 => { oy -= side; }          // next square goes left of this
                1 => { ox -= side; }          // next square goes below
                2 => { oy += side; }          // next square goes right
                _ => { ox += side; }          // next square goes above
            }
        }

        // Ratio labels in corner
        let rx = W - 250;
        text(rx, 44, C_ORANGE, "φ = 1.618033...");
        text(rx, 62, C_HINT,   "F(n+1)/F(n) →");
        for i in 1..8 {
            let a = fibs[i - 1];
            let b = fibs[i];
            if a > 0 {
                let ri = b / a;
                let rf = (b * 1000 / a) % 1000;
                let c = if (rf as i64 - 618).abs() < 20 { C_GREEN } else { C_TEXT };
                text(rx, 80 + (i as i32 - 1) * 16, c,
                    &format!("F({})/{}: {}.{:03}", i + 1, i, ri, rf));
            }
        }
    }

    fn tick(&mut self) {
        self.tick += 1;
        self.hi_bar = (self.tick / 2) as usize % 20;
        self.draw();
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" { self.tick(); return; }
        match line {
            "\t" => {
                self.view = match self.view {
                    View::Bar    => View::Table,
                    View::Table  => View::Spiral,
                    View::Spiral => View::Bar,
                };
                self.draw();
            }
            "+" | "=" => {
                if matches!(self.view, View::Bar) && self.zoom < 8 { self.zoom += 1; self.draw(); }
            }
            "-" => {
                if matches!(self.view, View::Bar) && self.zoom > 1 { self.zoom -= 1; self.draw(); }
            }
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
