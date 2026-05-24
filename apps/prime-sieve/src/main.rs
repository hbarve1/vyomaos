// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const MAX: usize = 200;
const COLS: usize = 20;
const CELL_W: i32 = 38;
const CELL_H: i32 = 34;
const GX: i32 = 20;
const GY: i32 = 52;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_CARD: u32   = 0x161B22FF;
const C_RED: u32    = 0xFF7B72FF;

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

struct App {
    composite: [bool; MAX + 2],
    current_p: usize,
    mult: usize,
    done: bool,
    paused: bool,
    speed: u32,
    tick: u32,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            composite: [false; MAX + 2],
            current_p: 2,
            mult: 4,
            done: false,
            paused: false,
            speed: 2,
            tick: 0,
        };
        a.composite[0] = true;
        a.composite[1] = true;
        a
    }

    fn reset(&mut self) {
        self.composite = [false; MAX + 2];
        self.composite[0] = true;
        self.composite[1] = true;
        self.current_p = 2;
        self.mult = 4;
        self.done = false;
        self.tick = 0;
    }

    fn step(&mut self) {
        if self.done { return; }

        if self.mult > MAX {
            let mut next = self.current_p + 1;
            while next <= MAX && self.composite[next] { next += 1; }
            if next > MAX || next * next > MAX {
                self.done = true;
                return;
            }
            self.current_p = next;
            self.mult = self.current_p * 2;
        }

        if self.mult <= MAX {
            self.composite[self.mult] = true;
            self.mult += self.current_p;
        }
    }

    fn prime_count(&self) -> usize {
        (2..=MAX).filter(|&n| !self.composite[n]).count()
    }

    fn composite_count(&self) -> usize {
        (2..=MAX).filter(|&n| self.composite[n]).count()
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Prime Sieve");
        text(100, 8, C_HINT, "Space=pause  R=reset  +=faster  -=slower  Q=quit");

        let gw = COLS as i32 * CELL_W;
        let rows = (MAX + COLS - 1) / COLS;
        let gh = rows as i32 * CELL_H;

        // Grid background (creates cell borders via 1px gaps)
        fill(GX, GY, gw, gh, C_BORDER);

        for num in 2..=MAX {
            let i = num - 2;
            let c = (i % COLS) as i32;
            let r = (i / COLS) as i32;
            let px = GX + c * CELL_W + 1;
            let py = GY + r * CELL_H + 1;
            let cs = CELL_W - 2;
            let ch = CELL_H - 2;

            let is_cp = num == self.current_p;
            let is_comp = self.composite[num];

            let bg = if is_cp {
                C_ORANGE
            } else if is_comp {
                0x2D0A0AFF
            } else if self.done {
                0x0F2D1AFF
            } else {
                C_CARD
            };
            fill(px, py, cs, ch, bg);

            let tc = if is_cp {
                0x000000FF
            } else if is_comp {
                0x5A2020FF
            } else if self.done {
                C_GREEN
            } else {
                C_TEXT
            };

            let tx = px + if num < 10 { 14 } else if num < 100 { 10 } else { 6 };
            text(tx, py + 9, tc, &num.to_string());
        }

        border(GX, GY, gw, gh, C_BORDER);

        // Stats panel
        let sy = GY + gh + 12;
        let sw = W - 40;
        fill(20, sy, sw, 180, C_CARD);
        border(20, sy, sw, 180, C_BORDER);

        let pc = self.prime_count();
        let cc = self.composite_count();

        if self.done {
            text(32, sy + 12, C_ORANGE, "Sieve complete!");
            text(200, sy + 12, C_GREEN, &format!("{} primes found in 2..={}", pc, MAX));
        } else {
            let state = if self.paused { "[PAUSED]" } else { "[RUNNING]" };
            let sc = if self.paused { C_SEL } else { C_GREEN };
            text(32, sy + 12, C_ORANGE, &format!("Current factor: {}", self.current_p));
            text(32 + 180, sy + 12, sc, state);
        }

        text(32, sy + 34, C_GREEN, &format!("Primes: {}", pc));
        text(200, sy + 34, C_RED, &format!("Composites: {}", cc));
        text(380, sy + 34, C_HINT, &format!("Remaining: {}", MAX - 1 - pc - cc));
        text(32, sy + 54, C_HINT, &format!("Speed: {} (1=fast 16=slow)  Next multiple: {}", self.speed, if self.done { 0 } else { self.mult }));

        // Prime list (confirmed primes so far)
        let confirmed: Vec<usize> = (2..=MAX)
            .filter(|&n| !self.composite[n] && (n <= self.current_p || self.done))
            .collect();
        let list_str = confirmed.iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" ");
        text(32, sy + 76, C_HINT, "Primes:");
        let bytes = list_str.as_bytes();
        let cpl = 100usize;
        for i in 0..3 {
            let s = i * cpl;
            if s >= bytes.len() { break; }
            let e = (s + cpl).min(bytes.len());
            if let Ok(line) = std::str::from_utf8(&bytes[s..e]) {
                text(96, sy + 76 + i as i32 * 18, C_GREEN, line);
            }
        }

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, "Orange = current prime  Dark red = composite  Green = prime (done)");
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                self.tick = self.tick.wrapping_add(1);
                if !self.paused && !self.done && self.tick % self.speed == 0 {
                    self.step();
                }
                if !self.done {
                    println!("@supervisor: ping");
                    let _ = io::stdout().flush();
                }
            }
            " " => { self.paused = !self.paused; }
            "r" | "R" => {
                self.reset();
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            "+" | "=" => { if self.speed > 1 { self.speed -= 1; } }
            "-" => { if self.speed < 16 { self.speed += 1; } }
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
