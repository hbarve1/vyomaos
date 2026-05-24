// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 520;
const H: u32 = 700;
const HEADER_H: u32 = 48;
const CELL_W: u32 = 72;
const CELL_H: u32 = 72;
const GAP: u32 = 8;
const GRID_X: u32 = (W - 5 * CELL_W - 4 * GAP) / 2; // 28
const GRID_Y: u32 = 64;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x538D4EFF;
const C_YELLOW: u32  = 0xB59F3BFF;
const C_ABSENT: u32  = 0x3A3F4BFF;
const C_EMPTY: u32   = 0x21262DFF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;

const WORDS: [&str; 60] = [
    "CRANE", "SLOTH", "BRAVE", "PLANT", "FROST",
    "GHOST", "LIGHT", "BROWN", "CHESS", "FLINT",
    "GUARD", "HEART", "SHIRE", "TRICK", "BLAZE",
    "CROWN", "DRIFT", "GLOOM", "PIXEL", "RISKY",
    "SHARP", "STONE", "SWAMP", "TRACK", "VAULT",
    "WHEAT", "WITCH", "WORLD", "YIELD", "YOUTH",
    "ABBEY", "BLUNT", "CIVIC", "DWARF", "ECLAT",
    "FLAIR", "GLAZE", "HOIST", "INGOT", "JOUST",
    "KNACK", "LEMON", "MONTH", "NYMPH", "ORBIT",
    "PRANK", "QUART", "RAVEN", "SCALP", "TROUT",
    "UMBRA", "VIGOR", "WRECK", "XENON", "YACHT",
    "ZINCY", "ADORN", "BRINE", "CLEFT", "DIGIT",
];

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

// Letter state: 0=unknown, 1=absent, 2=present, 3=correct
struct Game {
    target:    [u8; 5],
    guesses:   [[u8; 5]; 6],
    results:   [[u8; 5]; 6], // per cell: 0=empty, 1=absent, 2=present, 3=correct
    current:   usize,        // current guess row
    cursor:    usize,        // current col in guess
    key_state: [u8; 26],     // best state per letter A-Z
    state:     u8,           // 0=playing, 1=won, 2=lost
    word_idx:  usize,
}

impl Game {
    fn new(seed: usize) -> Self {
        let idx = seed % WORDS.len();
        let target_str = WORDS[idx];
        let mut target = [0u8; 5];
        for (i, b) in target_str.bytes().enumerate() {
            target[i] = b - b'A';
        }
        Game {
            target,
            guesses:   [[26u8; 5]; 6],
            results:   [[0u8; 5]; 6],
            current:   0,
            cursor:    0,
            key_state: [0u8; 26],
            state:     0,
            word_idx:  idx,
        }
    }

    fn type_letter(&mut self, c: u8) {
        if self.state != 0 { return; }
        if self.cursor < 5 {
            self.guesses[self.current][self.cursor] = c;
            self.cursor += 1;
        }
    }

    fn backspace(&mut self) {
        if self.state != 0 { return; }
        if self.cursor > 0 {
            self.cursor -= 1;
            self.guesses[self.current][self.cursor] = 26;
        }
    }

    fn submit(&mut self) {
        if self.state != 0 { return; }
        if self.cursor < 5 { return; }

        let guess = self.guesses[self.current];
        let target = self.target;
        let mut result = [1u8; 5]; // default: absent
        let mut target_used = [false; 5];
        let mut guess_used  = [false; 5];

        // First pass: correct
        for i in 0..5 {
            if guess[i] == target[i] {
                result[i] = 3;
                target_used[i] = true;
                guess_used[i]  = true;
            }
        }
        // Second pass: present
        for i in 0..5 {
            if guess_used[i] { continue; }
            for j in 0..5 {
                if target_used[j] { continue; }
                if guess[i] == target[j] {
                    result[i] = 2;
                    target_used[j] = true;
                    break;
                }
            }
        }

        self.results[self.current] = result;

        // Update key states (best seen)
        for i in 0..5 {
            let li = guess[i] as usize;
            if result[i] > self.key_state[li] {
                self.key_state[li] = result[i];
            }
        }

        if result == [3, 3, 3, 3, 3] {
            self.state = 1; // won
        } else {
            self.current += 1;
            self.cursor = 0;
            if self.current == 6 {
                self.state = 2; // lost
            }
        }
    }
}

fn cell_color(state: u8) -> u32 {
    match state {
        1 => C_ABSENT,
        2 => C_YELLOW,
        3 => C_GREEN,
        _ => C_EMPTY,
    }
}

const KEYBOARD_ROWS: [&str; 3] = ["QWERTYUIOP", "ASDFGHJKL", "ZXCVBNM"];
const KEY_W: u32 = 40;
const KEY_H: u32 = 44;
const KEY_GAP: u32 = 5;
const KB_Y: u32 = 570;

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Wordle");
    let wname: [u8; 5] = g.target.map(|c| c + b'A');
    let _ = std::str::from_utf8(&wname); // won't show target
    text(120, 16, C_HINT, &format!("Word #{}", g.word_idx + 1));
    text(280, 16, C_HINT, "a-z type  Enter=submit  Bsp=del  R=new");

    // Grid
    for row in 0..6 {
        for col in 0..5 {
            let px = GRID_X + col as u32 * (CELL_W + GAP);
            let py = GRID_Y + row as u32 * (CELL_H + GAP);
            let letter = g.guesses[row][col];
            let result = g.results[row][col];

            let bg = if row < g.current || (row == g.current && g.state != 0) {
                cell_color(result)
            } else {
                C_EMPTY
            };
            fill(px, py, CELL_W, CELL_H, bg);

            let border_color = if row == g.current && g.state == 0 {
                C_SEL
            } else {
                C_BORDER
            };
            border(px, py, CELL_W, CELL_H, border_color);

            if letter < 26 {
                let ch = [(letter + b'A') as char];
                let s: String = ch.iter().collect();
                let tx = px + (CELL_W - 16) / 2;
                let ty = py + (CELL_H - 32) / 2;
                text_l(tx, ty, C_TEXT, &s);
            }
        }
    }

    // Keyboard
    for (ri, row) in KEYBOARD_ROWS.iter().enumerate() {
        let row_w = row.len() as u32 * (KEY_W + KEY_GAP) - KEY_GAP;
        let row_x = (W - row_w) / 2;
        let ky = KB_Y + ri as u32 * (KEY_H + KEY_GAP);
        for (ci, ch) in row.chars().enumerate() {
            let li = (ch as u8 - b'A') as usize;
            let state = g.key_state[li];
            let kx = row_x + ci as u32 * (KEY_W + KEY_GAP);
            let bg = cell_color(state);
            fill(kx, ky, KEY_W, KEY_H, bg);
            border(kx, ky, KEY_W, KEY_H, C_BORDER);
            let s = ch.to_string();
            text(kx + 14, ky + 14, C_TEXT, &s);
        }
    }

    // Status overlay
    match g.state {
        1 => {
            let bx = (W - 240) / 2;
            let by = (H - 80) / 2;
            fill(bx, by, 240, 80, C_HEADER);
            border(bx, by, 240, 80, C_GREEN);
            text(bx + 56, by + 12, C_GREEN, "PERFECT!");
            text(bx + 32, by + 40, C_HINT, "R to play again");
        }
        2 => {
            let bx = (W - 280) / 2;
            let by = (H - 80) / 2;
            fill(bx, by, 280, 80, C_HEADER);
            border(bx, by, 280, 80, 0xFF7B72FF);
            let tw: [u8; 5] = g.target.map(|c| c + b'A');
            let word = std::str::from_utf8(&tw).unwrap_or("?????");
            text(bx + 20, by + 12, 0xFF7B72FF, &format!("The word was: {}", word));
            text(bx + 44, by + 40, C_HINT, "R to play again");
        }
        _ => {}
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut seed: usize = 0;
    let mut game = Game::new(seed);

    println!("@supervisor: raise wordle");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\r" | "" => { game.submit(); }
            "\x7f" => { game.backspace(); }
            "r" | "R" => { seed += 1; game = Game::new(seed); }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= b'a' && b <= b'z' { game.type_letter(b - b'a'); }
                else if b >= b'A' && b <= b'Z' { game.type_letter(b - b'A'); }
            }
            _ => {}
        }
        draw(&game);
    }
}
