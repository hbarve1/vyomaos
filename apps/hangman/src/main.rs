use std::io::{self, BufRead, Write};

const W: u32 = 880;
const H: u32 = 700;
const HEADER_H: u32 = 48;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_WALL: u32   = 0x8B949EFF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const WORDS: [&str; 40] = [
    "algorithm", "compiler", "function", "variable", "keyboard",
    "monitor",   "network",  "database", "terminal", "software",
    "hardware",  "program",  "memory",   "storage",  "process",
    "thread",    "socket",   "pointer",  "integer",  "boolean",
    "library",   "module",   "package",  "runtime",  "syntax",
    "interface", "protocol", "webhook",  "pipeline", "iterator",
    "closure",   "pattern",  "abstract", "concrete", "generic",
    "template",  "virtual",  "static",   "dynamic",  "recursive",
];

const MAX_WRONG: usize = 6;

// Gallows anchor
const GX: u32 = 120;
const GY: u32 = HEADER_H + 40;
const GH: u32 = 280; // pole height
const GW: u32 = 160; // beam width
const GD: u32 = 6;   // thickness

struct Game {
    word:    Vec<char>,
    guessed: [bool; 26], // indexed by letter - 'a'
    wrong:   usize,
    won:     bool,
    lost:    bool,
    seed:    u64,
    wins:    u32,
    losses:  u32,
}

impl Game {
    fn new(seed: u64) -> Self {
        let mut s = seed;
        s = lcg(s);
        let idx = (s >> 33) as usize % WORDS.len();
        Game {
            word: WORDS[idx].chars().collect(),
            guessed: [false; 26],
            wrong: 0,
            won: false,
            lost: false,
            seed: s,
            wins: 0,
            losses: 0,
        }
    }

    fn new_game(&mut self) {
        let wins = self.wins;
        let losses = self.losses;
        let old_seed = self.seed;
        *self = Game::new(old_seed);
        self.wins = wins;
        self.losses = losses;
    }

    fn guess(&mut self, c: char) {
        if self.won || self.lost { return; }
        let i = (c as u8 - b'a') as usize;
        if self.guessed[i] { return; }
        self.guessed[i] = true;
        if !self.word.contains(&c) {
            self.wrong += 1;
            if self.wrong >= MAX_WRONG {
                self.lost = true;
                self.losses += 1;
            }
        } else if self.all_revealed() {
            self.won = true;
            self.wins += 1;
        }
    }

    fn all_revealed(&self) -> bool {
        self.word.iter().all(|&c| {
            let i = (c as u8 - b'a') as usize;
            self.guessed[i]
        })
    }

    fn hint(&mut self) {
        if self.won || self.lost { return; }
        // Find first unrevealed letter and reveal it, counting as wrong guess
        for &c in &self.word {
            let i = (c as u8 - b'a') as usize;
            if !self.guessed[i] {
                self.guessed[i] = true;
                self.wrong += 1;
                if self.wrong >= MAX_WRONG {
                    self.lost = true;
                    self.losses += 1;
                } else if self.all_revealed() {
                    self.won = true;
                    self.wins += 1;
                }
                break;
            }
        }
    }
}

fn draw_gallows(wrong: usize) {
    // Base
    fill(GX - 10, GY + GH, GW + 20, GD, C_WALL);
    // Vertical pole
    fill(GX, GY, GD, GH, C_WALL);
    // Horizontal beam
    fill(GX, GY, GW, GD, C_WALL);
    // Rope
    if wrong >= 1 {
        fill(GX + GW - GD / 2, GY + GD, GD, 28, C_WALL);
    }

    let hx = GX + GW - 8; // head center x
    let hy = GY + GD + 28; // head top y
    // Head (approximated as a 28×28 square with border)
    if wrong >= 1 {
        border(hx - 14, hy, 28, 28, C_WALL);
    }
    // Body
    if wrong >= 2 {
        fill(hx - 3, hy + 28, GD, 50, C_WALL);
    }
    // Left arm
    if wrong >= 3 {
        fill(hx - 30, hy + 38, 30, GD, C_WALL);
    }
    // Right arm
    if wrong >= 4 {
        fill(hx + 3, hy + 38, 30, GD, C_WALL);
    }
    // Left leg
    if wrong >= 5 {
        fill(hx - 24, hy + 78, GD, 44, C_WALL);
        fill(hx - 44, hy + 118, 24, GD, C_WALL);
    }
    // Right leg
    if wrong >= 6 {
        fill(hx + 3, hy + 78, GD, 44, C_WALL);
        fill(hx + 3, hy + 118, 24, GD, C_WALL);
    }
}

fn draw(game: &Game) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Hangman");
    text(140, 16, C_HINT, &format!("Wins: {}  Losses: {}  Wrong: {}/{}", game.wins, game.losses, game.wrong, MAX_WRONG));
    text(500, 16, C_HINT, "a-z:guess  ?:hint  N:new game");

    draw_gallows(game.wrong);

    // Word display (blanks and letters)
    let word_y = HEADER_H + 340;
    let cell_w: u32 = 36;
    let total = game.word.len() as u32 * cell_w;
    let wx = (W - total) / 2;
    for (i, &c) in game.word.iter().enumerate() {
        let cx = wx + i as u32 * cell_w;
        let idx = (c as u8 - b'a') as usize;
        let revealed = game.guessed[idx] || game.lost;
        // Underscore line
        fill(cx + 2, word_y + 28, cell_w - 8, 2, C_WALL);
        if revealed {
            let color = if game.guessed[idx] { C_GREEN } else { C_RED };
            text_l(cx + 6, word_y, color, &c.to_string());
        }
    }

    // Keyboard layout
    let kbd_y = word_y + 60;
    let rows = ["qwertyuiop", "asdfghjkl", "zxcvbnm"];
    let row_offsets: [u32; 3] = [0, 18, 36];
    for (ri, row) in rows.iter().enumerate() {
        let row_x = (W - row.len() as u32 * 36) / 2 + row_offsets[ri];
        for (ci, c) in row.chars().enumerate() {
            let kx = row_x + ci as u32 * 36;
            let ky = kbd_y + ri as u32 * 40;
            let idx = (c as u8 - b'a') as usize;
            let color = if !game.guessed[idx] {
                C_HINT
            } else if game.word.contains(&c) {
                C_GREEN
            } else {
                C_RED
            };
            text_l(kx, ky, color, &c.to_string());
        }
    }

    // Win overlay
    if game.won {
        let bx = (W - 320) / 2;
        let by = HEADER_H + 100;
        fill(bx, by, 320, 100, C_HEADER);
        border(bx, by, 320, 100, C_GREEN);
        text(bx + 60, by + 16, C_GREEN, "You Won!");
        let word_str: String = game.word.iter().collect();
        text(bx + 24, by + 44, C_HINT, &format!("\"{}\"  N=new game", word_str));
    }

    // Loss overlay
    if game.lost {
        let bx = (W - 340) / 2;
        let by = HEADER_H + 100;
        fill(bx, by, 340, 100, C_HEADER);
        border(bx, by, 340, 100, C_RED);
        text(bx + 60, by + 16, C_RED, "Game Over!");
        let word_str: String = game.word.iter().collect();
        text(bx + 24, by + 44, C_HINT, &format!("Word: \"{}\"  N=new game", word_str));
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let seed: u64 = 0xFEDCBA9876543210;
    let mut game = Game::new(seed);

    println!("@supervisor: raise hangman");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "n" | "N" => { game.new_game(); }
            "?" => { game.hint(); }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b.is_ascii_lowercase() {
                    game.guess(b as char);
                } else if b.is_ascii_uppercase() {
                    game.guess((b + 32) as char);
                }
            }
            _ => {}
        }
        draw(&game);
    }
}
