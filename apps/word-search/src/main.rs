// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GRID: usize = 15;
const CELL: i32 = 28;
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

const WORDS: &[&str] = &[
    "RUST", "WASM", "LINUX", "KERNEL", "MEMORY",
    "THREAD", "SOCKET", "CURSOR", "WINDOW", "PIXEL",
    "FRAME", "QUEUE", "STACK", "CACHE", "BLOCK",
    "ARRAY", "TRAIT", "SCOPE", "MACRO", "ASYNC",
];

const DIRS: [(i32, i32); 8] = [
    (1, 0), (-1, 0), (0, 1), (0, -1),
    (1, 1), (-1, 1), (1, -1), (-1, -1),
];

#[derive(Clone, Copy)]
struct Placement { x: usize, y: usize, dx: i32, dy: i32 }

struct App {
    grid: [[u8; GRID]; GRID],
    highlights: [[u32; GRID]; GRID],
    word_indices: [usize; 10],
    found: [bool; 10],
    cx: usize,
    cy: usize,
    sel_start: Option<(usize, usize)>,
    seed: u64,
    found_count: usize,
}

impl App {
    fn new(seed: u64) -> Self {
        let mut app = App {
            grid: [[0u8; GRID]; GRID],
            highlights: [[0u32; GRID]; GRID],
            word_indices: [0usize; 10],
            found: [false; 10],
            cx: 7, cy: 7,
            sel_start: None,
            seed,
            found_count: 0,
        };
        app.generate();
        app
    }

    fn generate(&mut self) {
        self.grid = [[0u8; GRID]; GRID];
        self.highlights = [[0u32; GRID]; GRID];
        self.found = [false; 10];
        self.found_count = 0;
        self.sel_start = None;

        let mut indices = [0usize; 20];
        for i in 0..20 { indices[i] = i; }
        for i in (1..20usize).rev() {
            self.seed = lcg(self.seed);
            let j = (self.seed % (i as u64 + 1)) as usize;
            indices.swap(i, j);
        }
        for i in 0..10 { self.word_indices[i] = indices[i]; }

        for i in 0..10 {
            let word = WORDS[self.word_indices[i]].as_bytes().to_vec();
            self.try_place(&word);
        }

        for r in 0..GRID {
            for c in 0..GRID {
                if self.grid[r][c] == 0 {
                    self.seed = lcg(self.seed);
                    self.grid[r][c] = b'A' + (self.seed % 26) as u8;
                }
            }
        }
    }

    fn try_place(&mut self, word: &[u8]) {
        for _ in 0..300 {
            self.seed = lcg(self.seed);
            let (dx, dy) = DIRS[(self.seed % 8) as usize];
            self.seed = lcg(self.seed);
            let sx = (self.seed % GRID as u64) as usize;
            self.seed = lcg(self.seed);
            let sy = (self.seed % GRID as u64) as usize;

            let len = word.len() as i32;
            let ex = sx as i32 + dx * (len - 1);
            let ey = sy as i32 + dy * (len - 1);
            if ex < 0 || ex >= GRID as i32 || ey < 0 || ey >= GRID as i32 { continue; }

            let mut ok = true;
            for i in 0..word.len() {
                let x = (sx as i32 + dx * i as i32) as usize;
                let y = (sy as i32 + dy * i as i32) as usize;
                if self.grid[y][x] != 0 && self.grid[y][x] != word[i] { ok = false; break; }
            }
            if ok {
                for i in 0..word.len() {
                    let x = (sx as i32 + dx * i as i32) as usize;
                    let y = (sy as i32 + dy * i as i32) as usize;
                    self.grid[y][x] = word[i];
                }
                return;
            }
        }
    }

    fn selection_cells(&self) -> Vec<(usize, usize)> {
        let Some((sx, sy)) = self.sel_start else { return vec![]; };
        let dx = self.cx as i32 - sx as i32;
        let dy = self.cy as i32 - sy as i32;

        let (step_x, step_y, len) = if dx == 0 && dy == 0 {
            (0i32, 0i32, 0i32)
        } else if dx == 0 {
            (0, dy.signum(), dy.abs())
        } else if dy == 0 {
            (dx.signum(), 0, dx.abs())
        } else if dx.abs() == dy.abs() {
            (dx.signum(), dy.signum(), dx.abs())
        } else {
            return vec![];
        };

        (0..=len).map(|i| {
            ((sx as i32 + step_x * i) as usize,
             (sy as i32 + step_y * i) as usize)
        }).collect()
    }

    fn confirm_selection(&mut self) {
        if self.sel_start.is_some() {
            let cells = self.selection_cells();
            if cells.len() > 1 {
                let selected: String = cells.iter().map(|&(c, r)| self.grid[r][c] as char).collect();
                let reversed: String = selected.chars().rev().collect();
                for i in 0..10 {
                    if self.found[i] { continue; }
                    let word = WORDS[self.word_indices[i]];
                    if selected == word || reversed == word {
                        self.found[i] = true;
                        self.found_count += 1;
                        for &(c, r) in &cells {
                            self.highlights[r][c] = 0x1A4D2EFF;
                        }
                        break;
                    }
                }
            }
            self.sel_start = None;
        } else {
            self.sel_start = Some((self.cx, self.cy));
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Word Search");
        text(120, 8, C_HINT, "Arrows:move  Enter:select  Esc:cancel  N:new  Q:quit");

        let gw = GRID as i32 * CELL;
        let gh = GRID as i32 * CELL;
        fill(GX, GY, gw, gh, C_BORDER);

        let sel_cells = self.selection_cells();

        for r in 0..GRID {
            for c in 0..GRID {
                let px = GX + c as i32 * CELL + 1;
                let py = GY + r as i32 * CELL + 1;
                let cs = CELL - 2;

                let is_cursor = c == self.cx && r == self.cy;
                let is_sel = sel_cells.iter().any(|&(sc, sr)| sc == c && sr == r);
                let hl = self.highlights[r][c];

                let bg = if is_cursor { C_SEL }
                         else if is_sel { 0x1F3A5FFF }
                         else if hl != 0 { hl }
                         else { C_CARD };
                fill(px, py, cs, cs, bg);

                let letter = &(self.grid[r][c] as char).to_string();
                let tc = if is_cursor || hl != 0 { 0xFFFFFFFF } else { C_TEXT };
                text(px + 9, py + 6, tc, letter);
            }
        }

        // Right panel — word list
        let rpx = GX + gw + 24;
        let rpy = GY;
        text(rpx, rpy, C_ORANGE, "Words to Find:");
        for i in 0..10 {
            let wy = rpy + 24 + i as i32 * 38;
            let word = WORDS[self.word_indices[i]];
            let (tc, marker) = if self.found[i] { (C_GREEN, "[X]") } else { (C_HINT, "[ ]") };
            text(rpx, wy, tc, &format!("{} {}", marker, word));
        }

        let count_y = rpy + 24 + 10 * 38 + 16;
        text(rpx, count_y, C_HINT, &format!("Found: {}/10", self.found_count));

        if self.found_count == 10 {
            let bx = W / 2 - 120;
            let by = H / 2 - 36;
            fill(bx, by, 240, 72, C_HEADER);
            border(bx, by, 240, 72, C_ORANGE);
            text(bx + 24, by + 12, C_ORANGE, "All words found!");
            text(bx + 36, by + 40, C_HINT, "N = new puzzle");
        }

        // Selection hint in right panel
        if let Some((sx, sy)) = self.sel_start {
            let hy = count_y + 32;
            text(rpx, hy, C_SEL, &format!("From ({},{})", sx, sy));
            text(rpx, hy + 18, C_HINT, "Move + Enter");
        }

        fill(0, H - 24, W, 24, C_HEADER);
        let status = if self.sel_start.is_some() {
            format!("Selecting — move to end of word, Enter to confirm, Esc to cancel")
        } else {
            format!("Cursor ({},{}) — Enter to start selection", self.cx, self.cy)
        };
        text(12, H - 18, C_HINT, &status);

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[A" => { if self.cy > 0 { self.cy -= 1; } }
            "\x1b[B" => { if self.cy < GRID - 1 { self.cy += 1; } }
            "\x1b[D" => { if self.cx > 0 { self.cx -= 1; } }
            "\x1b[C" => { if self.cx < GRID - 1 { self.cx += 1; } }
            ""       => self.confirm_selection(),
            "\x1b"   => { self.sel_start = None; }
            "n" | "N" => { self.seed = lcg(self.seed); self.generate(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new(0xD1CE_C0DE_BEEF_CAFEu64);
    app.draw();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
