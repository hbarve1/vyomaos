use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

const COLS: usize = 80;
const ROWS: usize = 50;
const CELL: i32   = 10;
const GX: i32     = 80;
const GY: i32     = 52;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_DEAD: u32   = 0x161B22FF;

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

// Age-based color: younger = bright green, older = dimmer
fn age_color(age: u8) -> u32 {
    let a = age.min(20) as u32;
    let g = 255 - a * 8;
    let r = a * 5;
    let b = a * 3;
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

struct App {
    grid: [[u8; COLS]; ROWS],   // 0=dead, >0=age
    next: [[u8; COLS]; ROWS],
    gen:  u64,
    seed: u64,
    paused: bool,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            grid: [[0u8; COLS]; ROWS],
            next: [[0u8; COLS]; ROWS],
            gen:  0,
            seed: 0xDEADBEEF_CAFEF00D,
            paused: false,
        };
        a.randomize();
        a
    }

    fn randomize(&mut self) {
        self.gen = 0;
        for r in 0..ROWS {
            for c in 0..COLS {
                self.seed = lcg(self.seed);
                self.grid[r][c] = if self.seed >> 63 == 1 { 1 } else { 0 };
            }
        }
    }

    fn neighbors(&self, r: usize, c: usize) -> u8 {
        let mut n = 0u8;
        for dr in [-1i32, 0, 1] {
            for dc in [-1i32, 0, 1] {
                if dr == 0 && dc == 0 { continue; }
                let nr = (r as i32 + dr).rem_euclid(ROWS as i32) as usize;
                let nc = (c as i32 + dc).rem_euclid(COLS as i32) as usize;
                if self.grid[nr][nc] > 0 { n += 1; }
            }
        }
        n
    }

    fn step(&mut self) {
        for r in 0..ROWS {
            for c in 0..COLS {
                let n = self.neighbors(r, c);
                let alive = self.grid[r][c] > 0;
                self.next[r][c] = if alive {
                    if n == 2 || n == 3 { self.grid[r][c].saturating_add(1) } else { 0 }
                } else {
                    if n == 3 { 1 } else { 0 }
                };
            }
        }
        for r in 0..ROWS {
            self.grid[r] = self.next[r];
        }
        self.gen += 1;
    }

    fn live_count(&self) -> usize {
        self.grid.iter().flat_map(|r| r.iter()).filter(|&&c| c > 0).count()
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Conway's Game of Life");
        text(260, 8, C_HINT, "P:pause/play  S:step  R:random  Q:quit");

        // Grid background
        fill(GX - 2, GY - 2, COLS as i32 * CELL + 4, ROWS as i32 * CELL + 4, C_CARD);
        border(GX - 2, GY - 2, COLS as i32 * CELL + 4, ROWS as i32 * CELL + 4, C_BORDER);

        // Draw cells
        for r in 0..ROWS {
            for c in 0..COLS {
                let x = GX + c as i32 * CELL;
                let y = GY + r as i32 * CELL;
                let age = self.grid[r][c];
                if age > 0 {
                    fill(x + 1, y + 1, CELL - 2, CELL - 2, age_color(age));
                }
            }
        }

        // Stats panel on left
        let px = 4i32;
        text(px, GY + 10, C_ORANGE, "Gen:");
        text(px, GY + 26, C_SEL, &format!("{}", self.gen));
        text(px, GY + 50, C_ORANGE, "Live:");
        let live = self.live_count();
        text(px, GY + 66, C_GREEN, &format!("{}", live));
        text(px, GY + 90, C_ORANGE, "Total:");
        text(px, GY + 106, C_HINT, &format!("{}", COLS * ROWS));

        // Density bar
        let density = live * 100 / (COLS * ROWS);
        text(px, GY + 130, C_ORANGE, "Fill%");
        fill(px, GY + 146, 68, 8, C_BORDER);
        fill(px, GY + 146, 68 * density as i32 / 100, 8, C_GREEN);

        let status = if self.paused { "PAUSED" } else { "RUNNING" };
        let sc = if self.paused { C_ORANGE } else { C_GREEN };
        text(px, GY + 170, sc, status);

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!("Gen {} | {} alive | {}% density", self.gen, live, density));
        flush();
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" {
            if !self.paused { self.step(); }
            self.draw();
            return;
        }
        match line {
            "p" | "P" => { self.paused = !self.paused; self.draw(); }
            "s" | "S" => { self.step(); self.draw(); }
            "r" | "R" => { self.randomize(); self.draw(); }
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
