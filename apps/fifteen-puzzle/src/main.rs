// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 640;
const H: u32 = 640;
const HEADER_H: u32 = 48;
const SIZE: usize = 4;
const CELL: u32 = 140;
const GRID_X: u32 = 40;  // (640 - 4*140) / 2
const GRID_Y: u32 = 64;  // 48 + (640 - 48 - 560) / 2

const C_BG: u32     = 0x0D1117FF;
const C_CARD: u32   = 0x161B22FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_SEL_BG: u32 = 0x1F4068FF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn text_l(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},l,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

struct Game {
    board:  [u8; 16],
    blank:  usize,
    moves:  u32,
    solved: bool,
}

impl Game {
    fn new() -> Self {
        let mut g = Game { board: [0u8; 16], blank: 15, moves: 0, solved: false };
        g.reset_and_shuffle();
        g
    }

    fn reset_and_shuffle(&mut self) {
        for i in 0..15u8 { self.board[i as usize] = i + 1; }
        self.board[15] = 0;
        self.blank = 15;
        self.moves = 0;
        self.solved = false;

        let mut seed: u64 = 0xDEADBEEFCAFEBABE;
        let mut last: i32 = 0;
        for _ in 0..200 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let row = (self.blank / SIZE) as i32;
            let col = (self.blank % SIZE) as i32;
            let mut cands = [(0i32, 0i32); 4];
            let mut nc = 0usize;
            for &(dr, dc) in &[(-1i32, 0i32), (1i32, 0i32), (0i32, -1i32), (0i32, 1i32)] {
                let nr = row + dr;
                let nco = col + dc;
                if nr >= 0 && nr < SIZE as i32 && nco >= 0 && nco < SIZE as i32
                    && (dr * SIZE as i32 + dc) != -last
                {
                    cands[nc] = (dr, dc);
                    nc += 1;
                }
            }
            if nc == 0 { continue; }
            let (dr, dc) = cands[(seed >> 32) as usize % nc];
            let nb = ((row + dr) * SIZE as i32 + (col + dc)) as usize;
            self.board.swap(self.blank, nb);
            last = dr * SIZE as i32 + dc;
            self.blank = nb;
        }
    }

    fn slide(&mut self, dr: i32, dc: i32) {
        let row = (self.blank / SIZE) as i32;
        let col = (self.blank % SIZE) as i32;
        let nr = row + dr;
        let nc = col + dc;
        if nr < 0 || nr >= SIZE as i32 || nc < 0 || nc >= SIZE as i32 { return; }
        let new_blank = (nr * SIZE as i32 + nc) as usize;
        self.board.swap(self.blank, new_blank);
        self.blank = new_blank;
        self.moves += 1;
        self.solved = (0..15usize).all(|i| self.board[i] == i as u8 + 1) && self.board[15] == 0;
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_TEXT, "15 Puzzle");
    text(120, 16, C_ORANGE, &format!("Moves: {}", g.moves));
    text(300, 16, C_HINT, "Arrows: slide tile  R: restart  Esc: quit");

    for r in 0..SIZE {
        for c in 0..SIZE {
            let idx = r * SIZE + c;
            let px = GRID_X + c as u32 * CELL;
            let py = GRID_Y + r as u32 * CELL;
            let val = g.board[idx];

            if val == 0 {
                fill(px, py, CELL, CELL, C_BG);
            } else {
                let is_correct = val == (r * SIZE + c + 1) as u8;
                let adj_blank = {
                    let br = (g.blank / SIZE) as i32;
                    let bc = (g.blank % SIZE) as i32;
                    ((r as i32 - br).abs() + (c as i32 - bc).abs()) == 1
                };

                let tile_bg = if g.solved {
                    0x1A3A1AFF
                } else if adj_blank {
                    C_SEL_BG
                } else {
                    C_CARD
                };
                let tile_bc = if g.solved {
                    C_GREEN
                } else if adj_blank {
                    C_SEL
                } else {
                    C_BORDER
                };
                let tile_fg = if g.solved {
                    C_GREEN
                } else if is_correct {
                    C_GREEN
                } else {
                    C_TEXT
                };

                fill(px + 4, py + 4, CELL - 8, CELL - 8, tile_bg);
                border(px + 4, py + 4, CELL - 8, CELL - 8, tile_bc);

                // Center number — 'l' font ~16px wide, 32px tall
                let s = val.to_string();
                let tw = s.len() as u32 * 16;
                let tx = px + (CELL - tw) / 2;
                let ty = py + (CELL - 32) / 2;
                text_l(tx, ty, tile_fg, &s);
            }
        }
    }

    if g.solved {
        let msg = format!("Solved! {} moves", g.moves);
        let mw = msg.len() as u32 * 8 + 40;
        let bx = (W - mw) / 2;
        let by = H / 2 - 24;
        fill(bx, by, mw, 48, C_HEADER);
        border(bx, by, mw, 48, C_GREEN);
        text(bx + 20, by + 16, C_GREEN, &msg);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise fifteen-puzzle");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => { game.slide(-1,  0); }
            "\x1b[B" => { game.slide( 1,  0); }
            "\x1b[D" => { game.slide( 0, -1); }
            "\x1b[C" => { game.slide( 0,  1); }
            "r" | "R" => { game.reset_and_shuffle(); }
            _ => {}
        }
        draw(&game);
    }
}
