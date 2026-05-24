use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GY: i32 = 32;
const COLS: usize = 160;
const ROWS: usize = 106;
const CELL: i32 = 6;

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
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

fn density_color(d: u8) -> u32 {
    if d == 0 { return C_BG; }
    let t = d as u32;
    if t <= 128 {
        let r = (0x0Du32 + (0x2Bu32 - 0x0D) * t / 128) as u8;
        let g = (0x11u32 + (0x65u32 - 0x11) * t / 128) as u8;
        let b = (0x17u32 + (0xECu32 - 0x17) * t / 128) as u8;
        ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
    } else {
        let t2 = t - 128;
        let r = (0x2Bu32 + (0xE6u32 - 0x2B) * t2 / 127) as u8;
        let g = (0x65u32 + (0xEDu32 - 0x65) * t2 / 127) as u8;
        let b = (0xECu32 + (0xEDu32 - 0xEC) * t2 / 127) as u8;
        ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
    }
}

struct App {
    grid: Vec<u8>,
    next: Vec<u8>,
    cur_r: usize,
    cur_c: usize,
    seed: u64,
    tick: u64,
}

impl App {
    fn new() -> Self {
        App {
            grid: vec![0u8; ROWS * COLS],
            next: vec![0u8; ROWS * COLS],
            cur_r: ROWS / 4,
            cur_c: COLS / 2,
            seed: 0xDEADBEEFCAFEBABE,
            tick: 0,
        }
    }

    fn idx(r: usize, c: usize) -> usize { r * COLS + c }

    fn pour(&mut self) {
        let r = self.cur_r as i32;
        let c = self.cur_c as i32;
        for dr in -1i32..=1 {
            for dc in -1i32..=1 {
                let nr = r + dr; let nc = c + dc;
                if nr >= 0 && nr < ROWS as i32 && nc >= 0 && nc < COLS as i32 {
                    self.grid[Self::idx(nr as usize, nc as usize)] = 255;
                }
            }
        }
    }

    fn step(&mut self) {
        self.next.copy_from_slice(&self.grid);
        let lr = self.tick % 2 == 0;

        for r in (0..ROWS).rev() {
            for ci in 0..COLS {
                let c = if lr { ci } else { COLS - 1 - ci };
                let d = self.grid[Self::idx(r, c)];
                if d == 0 { continue; }

                // Try to flow down (equalize with cell below)
                if r + 1 < ROWS {
                    let db = self.grid[Self::idx(r + 1, c)];
                    if db < d {
                        let total = d as u16 + db as u16;
                        self.next[Self::idx(r + 1, c)] = ((total + 1) / 2) as u8;
                        self.next[Self::idx(r, c)]     = (total / 2) as u8;
                        continue;
                    }
                }

                // Spread sideways if bottom is full
                let try_l = c > 0 && {
                    let dl = self.grid[Self::idx(r, c - 1)];
                    dl < d.saturating_sub(2)
                };
                let try_r = c + 1 < COLS && {
                    let dr2 = self.grid[Self::idx(r, c + 1)];
                    dr2 < d.saturating_sub(2)
                };
                match (try_l, try_r) {
                    (true, true) => {
                        let pick_left = rng(&mut self.seed) % 2 == 0;
                        if pick_left {
                            let dl = self.grid[Self::idx(r, c - 1)];
                            let total = d as u16 + dl as u16;
                            self.next[Self::idx(r, c - 1)] = ((total + 1) / 2) as u8;
                            self.next[Self::idx(r, c)]     = (total / 2) as u8;
                        } else {
                            let dr2 = self.grid[Self::idx(r, c + 1)];
                            let total = d as u16 + dr2 as u16;
                            self.next[Self::idx(r, c + 1)] = ((total + 1) / 2) as u8;
                            self.next[Self::idx(r, c)]     = (total / 2) as u8;
                        }
                    }
                    (true, false) => {
                        let dl = self.grid[Self::idx(r, c - 1)];
                        let total = d as u16 + dl as u16;
                        self.next[Self::idx(r, c - 1)] = ((total + 1) / 2) as u8;
                        self.next[Self::idx(r, c)]     = (total / 2) as u8;
                    }
                    (false, true) => {
                        let dr2 = self.grid[Self::idx(r, c + 1)];
                        let total = d as u16 + dr2 as u16;
                        self.next[Self::idx(r, c + 1)] = ((total + 1) / 2) as u8;
                        self.next[Self::idx(r, c)]     = (total / 2) as u8;
                    }
                    (false, false) => {}
                }
            }
        }

        std::mem::swap(&mut self.grid, &mut self.next);
        self.tick += 1;
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, GY, C_HEADER);
        fill(0, H - 24, W, 24, C_HEADER);

        text(12, 8, C_TEXT, "Fluid Simulation");
        text(160, 8, C_HINT, "←→↑↓=cursor  Space=pour  C=clear  Q=quit");

        // Draw grid with RLE per row
        for r in 0..ROWS {
            let y = GY + r as i32 * CELL;
            let mut c = 0;
            while c < COLS {
                let d = self.grid[r * COLS + c];
                let color = density_color(d);
                let mut end = c + 1;
                while end < COLS && density_color(self.grid[r * COLS + end]) == color {
                    end += 1;
                }
                fill(c as i32 * CELL, y, (end - c) as i32 * CELL, CELL, color);
                c = end;
            }
        }

        // Cursor outline (3×3 brush area)
        let cr = self.cur_r as i32;
        let cc = self.cur_c as i32;
        let x0 = ((cc - 1).max(0)) * CELL;
        let y0 = GY + ((cr - 1).max(0)) * CELL;
        let x1 = ((cc + 2).min(COLS as i32)) * CELL;
        let y1 = GY + ((cr + 2).min(ROWS as i32)) * CELL;
        fill(x0, y0, x1 - x0, 1, C_SEL);
        fill(x0, y1 - 1, x1 - x0, 1, C_SEL);
        fill(x0, y0, 1, y1 - y0, C_SEL);
        fill(x1 - 1, y0, 1, y1 - y0, C_SEL);

        text(12, H - 18, C_HINT, "Fluid flows downward and spreads sideways under gravity");
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                self.step();
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " "       => { self.pour(); }
            "c" | "C" => { for d in self.grid.iter_mut() { *d = 0; } }
            "\x1b[A"  => { if self.cur_r > 0 { self.cur_r -= 1; } }
            "\x1b[B"  => { if self.cur_r + 1 < ROWS { self.cur_r += 1; } }
            "\x1b[C"  => { if self.cur_c + 1 < COLS { self.cur_c += 1; } }
            "\x1b[D"  => { if self.cur_c > 0 { self.cur_c -= 1; } }
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
