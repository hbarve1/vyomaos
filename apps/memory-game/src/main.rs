// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 760;
const H: u32 = 680;
const HEADER_H: u32 = 48;
const COLS: usize = 4;
const ROWS: usize = 4;
const CARD_W: u32 = 160;
const CARD_H: u32 = 120;
const GAP: u32 = 8;
// (760 - 4*160 - 3*8) / 2 = (760-664)/2 = 48
const GRID_X: u32 = 48;
// 48 + (680 - 48 - 4*120 - 3*8) / 2 = 48 + (680-48-504)/2 = 48+64 = 112
const GRID_Y: u32 = 112;

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

const SYMBOLS: &[&str] = &["A", "B", "C", "D", "E", "F", "G", "H"];
const SYM_COLORS: &[u32] = &[
    0xFF7B72FF, // A - red
    0x3FB950FF, // B - green
    0x58A6FFFF, // C - blue
    0xFFA657FF, // D - orange
    0xD2A8FFFF, // E - purple
    0x79C0FFFF, // F - cyan
    0xF0883EFF, // G - amber
    0xE6EDF3FF, // H - white
];

// Card states: 0=Hidden 1=FaceUp 2=Matched
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

fn shuffled_board(seed: u64) -> [u8; 16] {
    let mut board = [0u8; 16];
    for i in 0..8 { board[i] = i as u8; board[i + 8] = i as u8; }
    let mut s = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    for i in (1..16usize).rev() {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (s >> 33) as usize % (i + 1);
        board.swap(i, j);
    }
    board
}

struct Game {
    board:    [u8; 16],
    states:   [u8; 16],   // 0=hidden 1=face-up 2=matched
    cursor:   usize,
    face_up:  [usize; 2],
    fup_n:    usize,       // count of face-up (0, 1, or 2)
    moves:    u32,
    matched:  u32,
    won:      bool,
    gen:      u64,
}

impl Game {
    fn new(gen: u64) -> Self {
        Game {
            board:   shuffled_board(gen),
            states:  [0u8; 16],
            cursor:  0,
            face_up: [0usize; 2],
            fup_n:   0,
            moves:   0,
            matched: 0,
            won:     false,
            gen,
        }
    }

    fn flip(&mut self) {
        let idx = self.cursor;
        match self.states[idx] {
            2 => return,  // already matched
            1 => return,  // already face-up
            _ => {}
        }

        // If 2 unmatched cards are face-up, flip them back first
        if self.fup_n == 2 {
            self.states[self.face_up[0]] = 0;
            self.states[self.face_up[1]] = 0;
            self.fup_n = 0;
        }

        // Flip this card
        self.states[idx] = 1;
        self.face_up[self.fup_n] = idx;
        self.fup_n += 1;

        // Check pair when 2 are face-up
        if self.fup_n == 2 {
            self.moves += 1;
            let a = self.face_up[0];
            let b = self.face_up[1];
            if self.board[a] == self.board[b] {
                self.states[a] = 2;
                self.states[b] = 2;
                self.fup_n = 0;
                self.matched += 2;
                if self.matched == 16 { self.won = true; }
            }
        }
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_TEXT, "Memory Game");
    text(160, 16, C_ORANGE, &format!("Pairs: {}/8  Moves: {}", g.matched / 2, g.moves));
    text(380, 16, C_HINT, "Space/Enter: flip  Arrows: move  R: restart");

    for r in 0..ROWS {
        for c in 0..COLS {
            let idx = r * COLS + c;
            let px = GRID_X + c as u32 * (CARD_W + GAP);
            let py = GRID_Y + r as u32 * (CARD_H + GAP);
            let is_cur = idx == g.cursor;

            match g.states[idx] {
                2 => {
                    // Matched
                    fill(px, py, CARD_W, CARD_H, 0x1A3A1AFF);
                    border(px, py, CARD_W, CARD_H, C_GREEN);
                    let sym_idx = g.board[idx] as usize;
                    let tx = px + (CARD_W - 16) / 2;
                    let ty = py + (CARD_H - 32) / 2;
                    text_l(tx, ty, C_GREEN, SYMBOLS[sym_idx]);
                }
                1 => {
                    // Face-up
                    fill(px, py, CARD_W, CARD_H, C_HEADER);
                    border(px, py, CARD_W, CARD_H, C_SEL);
                    let sym_idx = g.board[idx] as usize;
                    let color = SYM_COLORS[sym_idx];
                    let tx = px + (CARD_W - 16) / 2;
                    let ty = py + (CARD_H - 32) / 2;
                    text_l(tx, ty, color, SYMBOLS[sym_idx]);
                }
                _ => {
                    // Hidden
                    let bg = if is_cur { C_SEL_BG } else { C_CARD };
                    let bc = if is_cur { C_SEL } else { C_BORDER };
                    fill(px, py, CARD_W, CARD_H, bg);
                    border(px, py, CARD_W, CARD_H, bc);
                    // Card back pattern: small center square
                    fill(px + CARD_W/2 - 6, py + CARD_H/2 - 6, 12, 12, C_BORDER);
                    fill(px + CARD_W/2 - 3, py + CARD_H/2 - 3, 6, 6, if is_cur { C_SEL } else { 0x30363DFF });
                }
            }
        }
    }

    if g.won {
        let msg = format!("All matched! {} moves", g.moves);
        let mw = msg.len() as u32 * 8 + 48;
        let bx = (W - mw) / 2;
        let by = (H - 64) / 2;
        fill(bx, by, mw, 64, C_HEADER);
        border(bx, by, mw, 64, C_GREEN);
        text(bx + 24, by + 14, C_GREEN, &msg);
        text(bx + 24, by + 36, C_HINT, "Press R to play again");
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut gen = 0u64;
    let mut game = Game::new(gen);

    println!("@supervisor: raise memory-game");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        let cur = game.cursor;
        let row = cur / COLS;
        let col = cur % COLS;

        match raw.as_str() {
            "\x1b" | "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => { if row > 0 { game.cursor -= COLS; } }
            "\x1b[B" => { if row < ROWS - 1 { game.cursor += COLS; } }
            "\x1b[D" => { if col > 0 { game.cursor -= 1; } }
            "\x1b[C" => { if col < COLS - 1 { game.cursor += 1; } }
            " " | "" | "\r" => {
                if !game.won { game.flip(); }
            }
            "r" | "R" => {
                gen += 1;
                game = Game::new(gen);
            }
            _ => {}
        }
        draw(&game);
    }
}
