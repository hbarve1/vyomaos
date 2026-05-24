// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};

const W: u32 = 760;
const H: u32 = 680;
const HEADER_H: u32 = 48;
const ROWS: usize = 16;
const COLS: usize = 16;
const CELL: u32 = 36;
const MINES_COUNT: usize = 96;
const GRID_X: u32 = 92;  // (760 - 16*36) / 2
const GRID_Y: u32 = 76;  // 48 + (680 - 48 - 576) / 2

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
const C_RED: u32    = 0xFF7B72FF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

#[derive(PartialEq, Clone, Copy)]
enum State { Waiting, Playing, Won, Lost }

struct Game {
    mines:      [[bool; COLS]; ROWS],
    revealed:   [[bool; COLS]; ROWS],
    flagged:    [[bool; COLS]; ROWS],
    cursor:     (usize, usize),
    state:      State,
    flags:      usize,
    safe_count: usize,
}

impl Game {
    fn new() -> Self {
        Game {
            mines:      [[false; COLS]; ROWS],
            revealed:   [[false; COLS]; ROWS],
            flagged:    [[false; COLS]; ROWS],
            cursor:     (ROWS / 2, COLS / 2),
            state:      State::Waiting,
            flags:      0,
            safe_count: 0,
        }
    }

    fn place_mines(&mut self, avoid_r: usize, avoid_c: usize) {
        let mut seed: u64 = (avoid_r as u64)
            .wrapping_mul(7919)
            .wrapping_add(avoid_c as u64)
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.mines = [[false; COLS]; ROWS];
        let mut placed = 0;
        while placed < MINES_COUNT {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let idx = (seed >> 33) as usize % (ROWS * COLS);
            let r = idx / COLS;
            let c = idx % COLS;
            let dr = (r as i32 - avoid_r as i32).abs();
            let dc = (c as i32 - avoid_c as i32).abs();
            if !self.mines[r][c] && (dr > 1 || dc > 1) {
                self.mines[r][c] = true;
                placed += 1;
            }
        }
    }

    fn adj_count(&self, r: usize, c: usize) -> u8 {
        let mut n = 0u8;
        for dr in -1i32..=1 {
            for dc in -1i32..=1 {
                if dr == 0 && dc == 0 { continue; }
                let nr = r as i32 + dr;
                let nc = c as i32 + dc;
                if nr >= 0 && nr < ROWS as i32 && nc >= 0 && nc < COLS as i32 {
                    if self.mines[nr as usize][nc as usize] { n += 1; }
                }
            }
        }
        n
    }

    fn reveal_bfs(&mut self, start_r: usize, start_c: usize) {
        let mut queue: VecDeque<(usize, usize)> = VecDeque::new();
        queue.push_back((start_r, start_c));
        while let Some((r, c)) = queue.pop_front() {
            if self.revealed[r][c] || self.flagged[r][c] || self.mines[r][c] { continue; }
            self.revealed[r][c] = true;
            self.safe_count += 1;
            if self.adj_count(r, c) == 0 {
                for dr in -1i32..=1 {
                    for dc in -1i32..=1 {
                        if dr == 0 && dc == 0 { continue; }
                        let nr = r as i32 + dr;
                        let nc = c as i32 + dc;
                        if nr >= 0 && nr < ROWS as i32 && nc >= 0 && nc < COLS as i32 {
                            let (nr, nc) = (nr as usize, nc as usize);
                            if !self.revealed[nr][nc] { queue.push_back((nr, nc)); }
                        }
                    }
                }
            }
        }
    }

    fn reveal(&mut self, r: usize, c: usize) {
        if self.revealed[r][c] || self.flagged[r][c] { return; }
        if self.state == State::Waiting {
            self.place_mines(r, c);
            self.state = State::Playing;
        }
        if self.state != State::Playing { return; }
        if self.mines[r][c] {
            self.revealed[r][c] = true;
            self.state = State::Lost;
            return;
        }
        self.reveal_bfs(r, c);
        if self.safe_count == ROWS * COLS - MINES_COUNT {
            self.state = State::Won;
        }
    }

    fn toggle_flag(&mut self, r: usize, c: usize) {
        if self.revealed[r][c] { return; }
        if self.flagged[r][c] {
            self.flagged[r][c] = false;
            self.flags -= 1;
        } else {
            self.flagged[r][c] = true;
            self.flags += 1;
        }
    }
}

fn num_color(n: u8) -> u32 {
    match n {
        1 => 0x58A6FFFF,
        2 => 0x3FB950FF,
        3 => 0xFF7B72FF,
        4 => 0xFFA657FF,
        5 => 0xD2A8FFFF,
        6 => 0x79C0FFFF,
        7 => 0xE6EDF3FF,
        _ => 0x8B949EFF,
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_TEXT, "Minesweeper");
    let mines_left = MINES_COUNT as i32 - g.flags as i32;
    text(160, 16, C_ORANGE, &format!("Mines: {}  Flags: {}", mines_left, g.flags));
    text(380, 16, C_HINT, "Space: reveal  F: flag  R: restart  Esc: quit");

    let show_mines = g.state == State::Lost;
    for r in 0..ROWS {
        for c in 0..COLS {
            let px = GRID_X + c as u32 * CELL;
            let py = GRID_Y + r as u32 * CELL;
            let is_cur = g.cursor == (r, c);

            if g.revealed[r][c] {
                if g.mines[r][c] {
                    fill(px, py, CELL, CELL, 0x3D1A1AFF);
                    border(px, py, CELL, CELL, C_RED);
                    text(px + 13, py + 12, C_RED, "X");
                } else {
                    fill(px, py, CELL, CELL, C_BG);
                    border(px, py, CELL, CELL, 0x21262DFF);
                    let n = g.adj_count(r, c);
                    if n > 0 {
                        text(px + 13, py + 12, num_color(n), &n.to_string());
                    }
                }
            } else if g.flagged[r][c] {
                fill(px, py, CELL, CELL, 0x1A1A0DFF);
                border(px, py, CELL, CELL, C_ORANGE);
                text(px + 13, py + 12, C_ORANGE, "F");
            } else if show_mines && g.mines[r][c] {
                fill(px, py, CELL, CELL, 0x2D1A1AFF);
                border(px, py, CELL, CELL, 0x8B2020FF);
                text(px + 13, py + 12, 0xCC4444FF, "*");
            } else {
                fill(px, py, CELL, CELL, if is_cur { C_SEL_BG } else { C_CARD });
                border(px, py, CELL, CELL, if is_cur { C_SEL } else { C_BORDER });
            }
        }
    }

    // Status bar
    fill(0, H - 24, W, 24, C_HEADER);
    fill(0, H - 24, W, 1, C_BORDER);
    let status = match g.state {
        State::Waiting => "Navigate with arrows, then Space/Enter to reveal first cell",
        State::Playing => "Arrows: navigate  Space/Enter: reveal  F: flag/unflag",
        State::Won     => "You Win! All mines found. Press R to play again.",
        State::Lost    => "Game Over! Mines revealed. Press R to play again.",
    };
    text(16, H - 16, C_HINT, status);

    // Win / Loss overlay
    match g.state {
        State::Won => {
            let bx = (W - 180) / 2;
            let by = (H - 60) / 2;
            fill(bx, by, 180, 60, C_HEADER);
            border(bx, by, 180, 60, C_GREEN);
            text(bx + 40, by + 10, C_GREEN, "You Win!");
            text(bx + 16, by + 30, C_HINT, "Press R to restart");
        }
        State::Lost => {
            let bx = (W - 200) / 2;
            let by = (H - 60) / 2;
            fill(bx, by, 200, 60, C_HEADER);
            border(bx, by, 200, 60, C_RED);
            text(bx + 32, by + 10, C_RED, "Game Over!");
            text(bx + 24, by + 30, C_HINT, "Press R to restart");
        }
        _ => {}
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise minesweeper");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        let (r, c) = game.cursor;
        match raw.as_str() {
            "\x1b" | "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => { if r > 0 { game.cursor.0 -= 1; } }
            "\x1b[B" => { if r < ROWS - 1 { game.cursor.0 += 1; } }
            "\x1b[D" => { if c > 0 { game.cursor.1 -= 1; } }
            "\x1b[C" => { if c < COLS - 1 { game.cursor.1 += 1; } }
            " " | "" | "\r" => {
                if game.state == State::Waiting || game.state == State::Playing {
                    game.reveal(r, c);
                }
            }
            "f" | "F" => {
                if game.state == State::Waiting || game.state == State::Playing {
                    game.toggle_flag(r, c);
                }
            }
            "r" | "R" => { game = Game::new(); }
            _ => {}
        }
        draw(&game);
    }
}
