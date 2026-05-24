// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 680;
const H: u32 = 720;
const HEADER_H: u32 = 48;
const CELL: u32 = 72;
const BOARD_X: u32 = (W - 8 * CELL) / 2; // 44
const BOARD_Y: u32 = HEADER_H + 16;

const C_BG: u32       = 0x0D1117FF;
const C_HEADER: u32   = 0x21262DFF;
const C_BORDER: u32   = 0x30363DFF;
const C_TEXT: u32     = 0xE6EDF3FF;
const C_HINT: u32     = 0x6E7681FF;
const C_GREEN: u32    = 0x3FB950FF;
const C_ORANGE: u32   = 0xFFA657FF;
const C_RED: u32      = 0xFF7B72FF;
const C_SEL: u32      = 0x58A6FFFF;
const C_SEL_BG: u32   = 0x1F4068FF;
const C_LIGHT: u32    = 0x2D333BFF;
const C_DARK: u32     = 0x1C2128FF;
const C_VALID: u32    = 0x1A3A1AFF;

// Piece values: 1=K,2=Q,3=R,4=B,5=N,6=P; positive=white, negative=black, 0=empty
const WK: i8 = 1; const WQ: i8 = 2; const WR: i8 = 3;
const WB: i8 = 4; const WN: i8 = 5; const WP: i8 = 6;
const BK: i8 = -1; const BQ: i8 = -2; const BR: i8 = -3;
const BB: i8 = -4; const BN: i8 = -5; const BP: i8 = -6;

fn piece_char(p: i8) -> &'static str {
    match p {
        1 => "K", 2 => "Q", 3 => "R", 4 => "B", 5 => "N", 6 => "P",
        -1 => "k", -2 => "q", -3 => "r", -4 => "b", -5 => "n", -6 => "p",
        _ => " ",
    }
}

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

#[rustfmt::skip]
const START: [[i8; 8]; 8] = [
    [BR, BN, BB, BQ, BK, BB, BN, BR],
    [BP, BP, BP, BP, BP, BP, BP, BP],
    [ 0,  0,  0,  0,  0,  0,  0,  0],
    [ 0,  0,  0,  0,  0,  0,  0,  0],
    [ 0,  0,  0,  0,  0,  0,  0,  0],
    [ 0,  0,  0,  0,  0,  0,  0,  0],
    [WP, WP, WP, WP, WP, WP, WP, WP],
    [WR, WN, WB, WQ, WK, WB, WN, WR],
];

struct Game {
    board:    [[i8; 8]; 8],
    cursor:   (usize, usize),
    selected: Option<(usize, usize)>,
    valid:    [[bool; 8]; 8],
    white_turn: bool,
    status:   u8, // 0=playing, 1=white wins, 2=black wins
}

impl Game {
    fn new() -> Self {
        Game {
            board: START,
            cursor: (7, 4),
            selected: None,
            valid: [[false; 8]; 8],
            white_turn: true,
            status: 0,
        }
    }

    fn is_white(p: i8) -> bool { p > 0 }
    fn is_black(p: i8) -> bool { p < 0 }

    fn compute_valid(&mut self, r: usize, c: usize) {
        self.valid = [[false; 8]; 8];
        let p = self.board[r][c];
        if p == 0 { return; }
        let white = Self::is_white(p);

        let in_bounds = |r: i32, c: i32| r >= 0 && r < 8 && c >= 0 && c < 8;
        let can_land = |board: &[[i8; 8]; 8], tr: i32, tc: i32, white: bool| -> bool {
            if !in_bounds(tr, tc) { return false; }
            let t = board[tr as usize][tc as usize];
            if white { t <= 0 } else { t >= 0 }
        };
        let friendly = |board: &[[i8; 8]; 8], tr: i32, tc: i32, white: bool| -> bool {
            if !in_bounds(tr, tc) { return false; }
            let t = board[tr as usize][tc as usize];
            if white { t > 0 } else { t < 0 }
        };

        let mark = |valid: &mut [[bool; 8]; 8], tr: i32, tc: i32| {
            if in_bounds(tr, tc) { valid[tr as usize][tc as usize] = true; }
        };

        let slide = |valid: &mut [[bool; 8]; 8], board: &[[i8; 8]; 8], dr: i32, dc: i32, white: bool| {
            let mut tr = r as i32 + dr;
            let mut tc = c as i32 + dc;
            while in_bounds(tr, tc) {
                let t = board[tr as usize][tc as usize];
                if friendly(board, tr, tc, white) { break; }
                valid[tr as usize][tc as usize] = true;
                if t != 0 { break; }
                tr += dr; tc += dc;
            }
        };

        let ri = r as i32;
        let ci = c as i32;

        match p.abs() {
            1 => { // King
                for dr in -1i32..=1 {
                    for dc in -1i32..=1 {
                        if dr == 0 && dc == 0 { continue; }
                        if can_land(&self.board, ri + dr, ci + dc, white) {
                            mark(&mut self.valid, ri + dr, ci + dc);
                        }
                    }
                }
            }
            2 => { // Queen
                for &(dr, dc) in &[(-1,0),(1,0),(0,-1),(0,1),(-1,-1),(-1,1),(1,-1),(1,1)] {
                    slide(&mut self.valid, &self.board, dr, dc, white);
                }
            }
            3 => { // Rook
                for &(dr, dc) in &[(-1,0),(1,0),(0,-1),(0,1)] {
                    slide(&mut self.valid, &self.board, dr, dc, white);
                }
            }
            4 => { // Bishop
                for &(dr, dc) in &[(-1,-1),(-1,1),(1,-1),(1,1)] {
                    slide(&mut self.valid, &self.board, dr, dc, white);
                }
            }
            5 => { // Knight
                for &(dr, dc) in &[(-2,-1),(-2,1),(-1,-2),(-1,2),(1,-2),(1,2),(2,-1),(2,1)] {
                    if can_land(&self.board, ri + dr, ci + dc, white) {
                        mark(&mut self.valid, ri + dr, ci + dc);
                    }
                }
            }
            6 => { // Pawn
                let dir: i32 = if white { -1 } else { 1 };
                let start_row: i32 = if white { 6 } else { 1 };
                // Forward
                let nr = ri + dir;
                if in_bounds(nr, ci) && self.board[nr as usize][ci as usize] == 0 {
                    mark(&mut self.valid, nr, ci);
                    // Double move from start
                    if ri == start_row && self.board[(nr + dir) as usize][ci as usize] == 0 {
                        mark(&mut self.valid, nr + dir, ci);
                    }
                }
                // Captures
                for dc in [-1i32, 1] {
                    if in_bounds(nr, ci + dc) {
                        let t = self.board[nr as usize][(ci + dc) as usize];
                        if (white && Self::is_black(t)) || (!white && Self::is_white(t)) {
                            mark(&mut self.valid, nr, ci + dc);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn make_move(&mut self, fr: usize, fc: usize, tr: usize, tc: usize) {
        let p = self.board[fr][fc];
        let captured = self.board[tr][tc];
        self.board[tr][tc] = p;
        self.board[fr][fc] = 0;

        // Check if king captured
        if captured.abs() == 1 {
            if captured > 0 { self.status = 2; } // black wins
            else            { self.status = 1; } // white wins
        }

        // Pawn promotion
        if p == WP && tr == 0 { self.board[tr][tc] = WQ; }
        if p == BP && tr == 7 { self.board[tr][tc] = BQ; }

        self.white_turn = !self.white_turn;
        self.selected = None;
        self.valid = [[false; 8]; 8];
    }

    fn enter(&mut self) {
        if self.status != 0 { return; }
        let (cr, cc) = self.cursor;
        if let Some((sr, sc)) = self.selected {
            if self.valid[cr][cc] {
                self.make_move(sr, sc, cr, cc);
                return;
            }
        }
        // Select piece
        let p = self.board[cr][cc];
        if (self.white_turn && p > 0) || (!self.white_turn && p < 0) {
            self.selected = Some((cr, cc));
            self.compute_valid(cr, cc);
        } else {
            self.selected = None;
            self.valid = [[false; 8]; 8];
        }
    }

    fn move_cursor(&mut self, dr: i32, dc: i32) {
        let nr = (self.cursor.0 as i32 + dr).clamp(0, 7) as usize;
        let nc = (self.cursor.1 as i32 + dc).clamp(0, 7) as usize;
        self.cursor = (nr, nc);
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Chess");
    let turn_str = if g.status == 0 {
        if g.white_turn { "White's turn" } else { "Black's turn" }
    } else if g.status == 1 { "White wins!" } else { "Black wins!" };
    let turn_col = if g.status == 1 { C_TEXT } else if g.status == 2 { C_ORANGE } else { C_HINT };
    text(120, 16, turn_col, turn_str);
    text(360, 16, C_HINT, "Arrows:move  Enter:select/move  R:restart");

    // Rank/file labels
    let files = ["a","b","c","d","e","f","g","h"];
    for i in 0..8 {
        text(BOARD_X + i as u32 * CELL + 28, BOARD_Y + 8 * CELL + 4, C_HINT, files[i]);
        let rank = (8 - i).to_string();
        text(BOARD_X - 14, BOARD_Y + i as u32 * CELL + 26, C_HINT, &rank);
    }

    for r in 0..8usize {
        for c in 0..8usize {
            let px = BOARD_X + c as u32 * CELL;
            let py = BOARD_Y + r as u32 * CELL;
            let light = (r + c) % 2 == 0;

            let bg = if Some((r, c)) == g.selected {
                C_SEL_BG
            } else if g.valid[r][c] {
                C_VALID
            } else if light {
                C_LIGHT
            } else {
                C_DARK
            };
            fill(px, py, CELL, CELL, bg);

            if (r, c) == g.cursor {
                border(px, py, CELL, CELL, C_SEL);
            }

            if g.valid[r][c] && g.board[r][c] == 0 {
                // Draw dot for valid empty square
                fill(px + CELL / 2 - 6, py + CELL / 2 - 6, 12, 12, C_GREEN);
            }

            let p = g.board[r][c];
            if p != 0 {
                let fg = if p > 0 { C_TEXT } else { C_ORANGE };
                let ch = piece_char(p).to_uppercase().to_string();
                let tx = px + (CELL - 16) / 2;
                let ty = py + (CELL - 32) / 2;
                text_l(tx, ty, fg, &ch);
            }
        }
    }

    border(BOARD_X, BOARD_Y, 8 * CELL, 8 * CELL, C_BORDER);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise chess");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => { game.move_cursor(-1, 0); }
            "\x1b[B" => { game.move_cursor(1, 0); }
            "\x1b[C" => { game.move_cursor(0, 1); }
            "\x1b[D" => { game.move_cursor(0, -1); }
            "\r" | "" => { game.enter(); }
            "r" | "R" => { game = Game::new(); }
            _ => {}
        }
        draw(&game);
    }
}
