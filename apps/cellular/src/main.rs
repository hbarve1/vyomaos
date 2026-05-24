use std::collections::VecDeque;
use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const NCOLS: usize = 120;
const CELL_W: i32 = 8;
const CELL_H: i32 = 8;
const GY: i32 = 32;
const NROWS: usize = 83; // (720 - 32 - 24) / 8

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
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

type Row = [bool; NCOLS];

fn next_row(prev: &Row, rule: u8) -> Row {
    let mut next = [false; NCOLS];
    for i in 0..NCOLS {
        let l = if i == 0 { false } else { prev[i - 1] } as u8;
        let c = prev[i] as u8;
        let r = if i == NCOLS - 1 { false } else { prev[i + 1] } as u8;
        let pattern = (l << 2) | (c << 1) | r;
        next[i] = (rule >> pattern) & 1 == 1;
    }
    next
}

fn init_row() -> Row {
    let mut row = [false; NCOLS];
    row[NCOLS / 2] = true;
    row
}

struct App {
    rows: VecDeque<Row>,
    rule: u8,
    paused: bool,
}

impl App {
    fn new() -> Self {
        let mut rows = VecDeque::with_capacity(NROWS + 1);
        rows.push_back(init_row());
        App { rows, rule: 110, paused: false }
    }

    fn step(&mut self) {
        let last = *self.rows.back().unwrap();
        let next = next_row(&last, self.rule);
        if self.rows.len() >= NROWS { self.rows.pop_front(); }
        self.rows.push_back(next);
    }

    fn reset(&mut self) {
        self.rows.clear();
        self.rows.push_back(init_row());
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        fill(0, H - 24, W, 24, C_HEADER);

        let bin: String = (0..8u8).rev()
            .map(|i| if (self.rule >> i) & 1 == 1 { '1' } else { '0' })
            .collect();
        text(12, 8, C_TEXT, &format!("Cellular Automaton — Rule {}", self.rule));
        text(292, 8, C_HINT, &format!(
            "{}  +/-=rule  R=reset  Space={}  Q=quit",
            bin, if self.paused { "PAUSED" } else { "pause" }
        ));

        // Grid
        for (r, row) in self.rows.iter().enumerate() {
            let py = GY + r as i32 * CELL_H;
            for (c, &alive) in row.iter().enumerate() {
                if alive {
                    fill(c as i32 * CELL_W, py, CELL_W, CELL_H, C_SEL);
                }
            }
        }

        // Footer: rule bit squares (bit 7 = pattern 111 on left, bit 0 = pattern 000 on right)
        text(12, H - 18, C_HINT, &format!("Rule {:08b}", self.rule));
        let bx0 = W - 8 * 28 - 10;
        for i in 0..8u8 {
            let bit = 7 - i;
            let bx = bx0 + i as i32 * 28;
            let active = (self.rule >> bit) & 1 == 1;
            fill(bx, H - 22, 24, 16, if active { C_SEL } else { C_BORDER });
            text(bx + 8, H - 21, if active { C_BG } else { C_HINT }, &bit.to_string());
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                if !self.paused { self.step(); }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " " => { self.paused = !self.paused; }
            "+" | "=" => { self.rule = self.rule.wrapping_add(1); self.reset(); }
            "-" => { self.rule = self.rule.wrapping_sub(1); self.reset(); }
            "r" | "R" => { self.reset(); }
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
