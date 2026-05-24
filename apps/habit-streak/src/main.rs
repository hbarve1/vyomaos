// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 700;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_GREEN: u32   = 0x3FB950FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_CARD: u32    = 0x161B22FF;

const CELL_EMPTY: u32  = 0x161B22FF;
const CELL_DIM: u32    = 0x0E4429FF;
const CELL_MID: u32    = 0x26A641FF;
const CELL_BRIGHT: u32 = 0x39D353FF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

// (name, probability 0-100)
const HABITS: [(&str, u8); 10] = [
    ("Exercise",  65),
    ("Read",      80),
    ("Meditate",  55),
    ("Code",      85),
    ("Sleep 8h",  50),
    ("No Sugar",  45),
    ("Walk 10k",  60),
    ("Hydrate",   75),
    ("Journal",   70),
    ("Stretch",   72),
];

struct HabitData {
    done:   [bool; 364],
    streak: usize,
    best:   usize,
    rate:   u8,
}

fn gen_habit(prob: u8, seed: u64) -> HabitData {
    let mut done = [false; 364];
    let mut s = seed;
    for i in 0..364usize {
        s = lcg(s);
        done[i] = ((s >> 33) % 100) < prob as u64;
    }
    let mut streak = 0usize;
    for i in (0..364).rev() {
        if done[i] { streak += 1; } else { break; }
    }
    let mut best = 0usize;
    let mut cur  = 0usize;
    for &d in &done {
        if d { cur += 1; best = best.max(cur); } else { cur = 0; }
    }
    let rate = (done.iter().filter(|&&d| d).count() * 100 / 364) as u8;
    HabitData { done, streak, best, rate }
}

enum View { HeatMap, List }

struct App {
    habits: Vec<HabitData>,
    sel:    usize,
    view:   View,
}

impl App {
    fn new() -> Self {
        let mut habits = Vec::new();
        for (i, &(_, prob)) in HABITS.iter().enumerate() {
            let seed = 0xABCD_0000_1234u64.wrapping_add(i as u64 * 0x9E37_79B9_7F4A);
            habits.push(gen_habit(prob, seed));
        }
        App { habits, sel: 0, view: View::HeatMap }
    }

    fn draw_heatmap(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Habit Streak Calendar");
        text(W - 408, 8, C_HINT, "\u{2191}\u{2193}=habit  Tab=list view  Q=quit");

        // Left panel — habit list
        let lw = 192i32;
        fill(8, 36, lw, H - 60, C_CARD);
        border(8, 36, lw, H - 60, C_BORDER);
        text(16, 44, C_HINT, "HABITS");

        for (i, &(name, _)) in HABITS.iter().enumerate() {
            let hy = 62 + i as i32 * 58;
            let hd = &self.habits[i];
            let is_sel = i == self.sel;
            if is_sel { fill(10, hy - 2, lw - 4, 56, C_HEADER); }
            let nc = if is_sel { C_SEL } else { C_TEXT };
            // Today indicator dot
            fill(14, hy + 3, 10, 10, if hd.done[363] { C_GREEN } else { C_HINT });
            text(28, hy, nc, name);
            let sc = if hd.streak > 7 { C_GREEN } else if hd.streak > 0 { C_ORANGE } else { C_HINT };
            text(16, hy + 18, sc, &format!("{:2}d streak", hd.streak));
            text(16, hy + 34, C_HINT, &format!("{}% done", hd.rate));
        }

        // Right panel — heat map
        let hx = 208i32;
        let hw = W - hx - 8;
        fill(hx, 36, hw, H - 60, C_CARD);
        border(hx, 36, hw, H - 60, C_BORDER);

        let sel_name = HABITS[self.sel].0;
        let hd = &self.habits[self.sel];
        text(hx + 8, 44, C_ORANGE, sel_name);
        let sc = if hd.streak > 7 { C_GREEN } else if hd.streak > 0 { C_ORANGE } else { C_HINT };
        text(hx + 120, 44, sc,    &format!("Streak: {}", hd.streak));
        text(hx + 280, 44, C_SEL, &format!("Best: {}", hd.best));
        text(hx + 400, 44, C_HINT,&format!("Done: {}%", hd.rate));
        text(hx + 560, 44, C_HINT, "52-week history");

        // Grid coordinates: gx=228, gy=84
        // 52 cols × 14px = 728 → ends at 228+728=956 < 960 ✓
        let gx = 228i32;
        let gy = 84i32;
        let day_labels = ["S","M","T","W","T","F","S"];

        // Day-of-week labels (left of grid)
        for r in 0..7usize {
            text(gx - 16, gy + r as i32 * 14, C_HINT, day_labels[r]);
        }

        // Heat map 52×7
        for col in 0..52usize {
            for row in 0..7usize {
                let day_idx = col * 7 + row;
                let cx = gx + col as i32 * 14;
                let cy = gy + row as i32 * 14;
                let cell_c = if hd.done[day_idx] {
                    let recency = 363 - day_idx;
                    if recency < 14 { CELL_BRIGHT }
                    else if recency < 60 { CELL_MID }
                    else { CELL_DIM }
                } else {
                    CELL_EMPTY
                };
                fill(cx, cy, 12, 12, cell_c);
            }
        }

        // Month labels (every ~4 columns ≈ monthly)
        let months = ["M","J","J","A","S","O","N","D","J","F","M","A","M"];
        for m in 0..13usize {
            let col = m * 4;
            if col < 52 {
                text(gx + col as i32 * 14, gy + 7 * 14 + 4, C_HINT, months[m]);
            }
        }

        // Legend
        let ly = gy + 7 * 14 + 22;
        text(gx, ly, C_HINT, "Less");
        for (li, &lc) in [CELL_EMPTY, CELL_DIM, CELL_MID, CELL_BRIGHT].iter().enumerate() {
            fill(gx + 44 + li as i32 * 16, ly + 2, 12, 12, lc);
        }
        text(gx + 44 + 4 * 16 + 4, ly, C_HINT, "More");

        // Stats below legend
        let sy = ly + 32;
        let count_7  = hd.done[357..364].iter().filter(|&&d| d).count();
        let count_30 = hd.done[334..364].iter().filter(|&&d| d).count();
        let total    = hd.done.iter().filter(|&&d| d).count();
        text(gx, sy,      C_HINT, &format!("Last  7 days:  {}/7   ({}%)", count_7,  count_7  * 100 / 7));
        text(gx, sy + 18, C_HINT, &format!("Last 30 days: {}/30  ({}%)", count_30, count_30 * 100 / 30));
        text(gx, sy + 36, C_HINT, &format!("Full year:    {}/364 ({}%)", total,    hd.rate));

        // Bottom bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("Habit {}/{}  |  {}-day streak  |  {}% completion  |  Today: {}",
                 self.sel + 1, HABITS.len(), hd.streak, hd.rate,
                 if hd.done[363] { "done" } else { "pending" }));

        flush();
    }

    fn draw_list(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Habit Streak Calendar \u{2014} List View");
        text(W - 408, 8, C_HINT, "\u{2191}\u{2193}=navigate  Tab=heat map  Q=quit");

        // Table header
        fill(8, 36, W - 16, 22, C_HEADER);
        text(14,  40, C_HINT, "HABIT");
        text(210, 40, C_HINT, "CUR STREAK");
        text(360, 40, C_HINT, "BEST");
        text(470, 40, C_HINT, "RATE");
        text(560, 40, C_HINT, "7-DAY");
        text(700, 40, C_HINT, "30-DAY");

        // Table rows
        for (i, &(name, _)) in HABITS.iter().enumerate() {
            let ry = 62 + i as i32 * 56;
            let hd = &self.habits[i];
            let is_sel = i == self.sel;
            if is_sel { fill(8, ry - 2, W - 16, 54, C_HEADER); }
            let nc = if is_sel { C_SEL } else { C_TEXT };

            // Today indicator + name
            fill(12, ry + 4, 10, 10, if hd.done[363] { C_GREEN } else { C_HINT });
            text(26, ry, nc, name);

            // Current streak
            let streak_bar = (hd.streak * 2).min(100) as i32;
            fill(210, ry + 6, streak_bar, 8, C_GREEN);
            let sc = if hd.streak > 7 { C_GREEN } else { C_ORANGE };
            text(210, ry + 18, sc, &format!("{} days", hd.streak));

            // Best streak
            text(360, ry, C_SEL, &format!("{}", hd.best));
            let best_bar = (hd.best * 2).min(80) as i32;
            fill(360, ry + 18, best_bar, 6, C_SEL);

            // Completion rate
            let rate_bar = (hd.rate as i32 * 80 / 100).max(2);
            fill(470, ry + 6, rate_bar, 8, C_ORANGE);
            text(470, ry + 18, C_HINT, &format!("{}%", hd.rate));

            // 7-day mini grid
            for d in 0..7usize {
                let day = if d < 7 { 363 - (6 - d) } else { 363 };
                let dc = if hd.done[day] { C_GREEN } else { CELL_EMPTY };
                fill(560 + d as i32 * 14, ry + 4, 12, 12, dc);
            }

            // 30-day count
            let count_30 = hd.done[334..364].iter().filter(|&&d| d).count();
            let bar30 = (count_30 as i32 * 80 / 30).max(2);
            fill(700, ry + 6, bar30, 8, C_SEL);
            text(700, ry + 18, C_HINT, &format!("{}/30", count_30));
        }

        // Bottom bar
        let done_today = self.habits.iter().filter(|h| h.done[363]).count();
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("Today: {}/{} habits done  |  Selected: {}",
                 done_today, HABITS.len(), HABITS[self.sel].0));

        flush();
    }

    fn draw(&self) {
        match self.view {
            View::HeatMap => self.draw_heatmap(),
            View::List    => self.draw_list(),
        }
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[A" => { if self.sel > 0 { self.sel -= 1; } self.draw(); }
            "\x1b[B" => { if self.sel + 1 < HABITS.len() { self.sel += 1; } self.draw(); }
            "\t"     => {
                self.view = match self.view {
                    View::HeatMap => View::List,
                    View::List    => View::HeatMap,
                };
                self.draw();
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
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
