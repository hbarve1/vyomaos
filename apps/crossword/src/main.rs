use std::io::{self, BufRead, Write};

const W: i32 = 800;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_CARD: u32   = 0x161B22FF;
const C_WHITE: u32  = 0xFFFFFFFF;
const C_BLACK: u32  = 0x000000FF;

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

// 5x5 grid. '#' = black cell, letter = solution, ' ' = empty white cell
// Solution:
//   CRANE
//   H   I
//   ANTED  (A R T E D)
//   R   G
//   TREND
const SOLUTION: [[u8; 5]; 5] = [
    [b'C', b'R', b'A', b'N', b'E'],
    [b'H', b'#', b'R', b'#', b'I'],
    [b'A', b'R', b'T', b'E', b'D'],
    [b'R', b'#', b'E', b'#', b'G'],
    [b'S', b'O', b'N', b'E', b'T'],
];

const ACROSS: [&str; 5] = [
    "1A. A large bird (5)",
    "3A. Wagered (5)",  // ARTED? no — ANTED
    "5A. Poem form (6)",
    "",
    "",
];

const DOWN: [&str; 5] = [
    "1D. Pursuit (5)",  // CHARS
    "2D. Armed conflict (3)",  // ART?
    "3D. Musical interval (5)",// ARTES
    "4D. Number (5)",
    "",
];

// Better crossword:
// PLANT
// I # A #
// NOTES
// E # E #
// SCENE
// Across: PLANT(1), NOTES(3), SCENE(5)
// Down: PIE(1D col0), TONES(2D col2),

// Let's use a well-defined 5x5 crossword:
// BLAND
// L # I #
// ATONE
// N # E #
// KNEEL

const SOL: [[u8; 5]; 5] = [
    [b'B', b'L', b'A', b'N', b'D'],
    [b'L', b'#', b'I', b'#', b'E'],
    [b'A', b'T', b'O', b'N', b'E'],
    [b'N', b'#', b'N', b'#', b'L'],
    [b'K', b'N', b'E', b'E', b'L'],
];

const CLUES_ACROSS: [&str; 5] = [
    "1A: Dull, unexciting (5)",
    "3A: Repent, make amends (5)",
    "5A: Kneel down (5)",
    "",
    "",
];
const CLUES_DOWN: [&str; 5] = [
    "1D: Flat, monotonous (5)",
    "2D: Unite in harmony (5)",
    "3D: Anoint solemnly (5)",
    "4D: Heavenly messenger (5)",
    "5D: Handle or grip (5)",
];

const CELL: i32 = 60;
const GAP: i32  = 2;
const GX: i32   = 40;
const GY: i32   = 56;

struct App {
    user:   [[u8; 5]; 5],
    state:  [[u8; 5]; 5], // 0=empty 1=correct 2=wrong 3=revealed
    cx: usize,
    cy: usize,
    checked: bool,
    score: u32,
}

impl App {
    fn new() -> Self {
        App {
            user:    [[b' '; 5]; 5],
            state:   [[0u8; 5]; 5],
            cx: 0, cy: 0,
            checked: false,
            score: 0,
        }
    }

    fn is_black(row: usize, col: usize) -> bool {
        SOL[row][col] == b'#'
    }

    fn move_cursor(&mut self, dr: i32, dc: i32) {
        let mut r = self.cy as i32 + dr;
        let mut c = self.cx as i32 + dc;
        // skip black cells
        for _ in 0..25 {
            if r < 0 { r = 4; } else if r > 4 { r = 0; }
            if c < 0 { c = 4; } else if c > 4 { c = 0; }
            if !Self::is_black(r as usize, c as usize) { break; }
            r += dr; c += dc;
        }
        self.cy = r.clamp(0, 4) as usize;
        self.cx = c.clamp(0, 4) as usize;
    }

    fn check(&mut self) {
        self.checked = true;
        let mut correct = 0u32;
        for r in 0..5 {
            for c in 0..5 {
                if Self::is_black(r, c) { continue; }
                if self.user[r][c].to_ascii_uppercase() == SOL[r][c] {
                    self.state[r][c] = 1;
                    correct += 1;
                } else if self.user[r][c] != b' ' {
                    self.state[r][c] = 2;
                } else {
                    self.state[r][c] = 0;
                }
            }
        }
        self.score = correct;
    }

    fn reveal_one(&mut self) {
        for r in 0..5 {
            for c in 0..5 {
                if Self::is_black(r, c) { continue; }
                if self.user[r][c].to_ascii_uppercase() != SOL[r][c] {
                    self.user[r][c] = SOL[r][c];
                    self.state[r][c] = 3;
                    self.checked = false;
                    return;
                }
            }
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Crossword");
        text(180, 8, C_HINT, "Arrows:move  Letter:type  C:check  R:reveal  Bksp:clear  Q:quit");

        // Grid
        for r in 0..5usize {
            for c in 0..5usize {
                let x = GX + c as i32 * (CELL + GAP);
                let y = GY + r as i32 * (CELL + GAP);
                if Self::is_black(r, c) {
                    fill(x, y, CELL, CELL, C_BLACK);
                } else {
                    let bg = if r == self.cy && c == self.cx {
                        C_SEL
                    } else {
                        match self.state[r][c] {
                            1 => 0x1A3D1FFF, // dark green
                            2 => 0x3D1A1AFF, // dark red
                            3 => 0x1A2A3DFF, // dark blue (revealed)
                            _ => C_WHITE,
                        }
                    };
                    fill(x, y, CELL, CELL, bg);
                    border(x, y, CELL, CELL, C_BORDER);

                    let ch = self.user[r][c];
                    if ch != b' ' {
                        let tc = if r == self.cy && c == self.cx {
                            C_BLACK
                        } else {
                            match self.state[r][c] {
                                1 => C_GREEN,
                                2 => C_RED,
                                3 => C_SEL,
                                _ => C_BLACK,
                            }
                        };
                        let s = [ch];
                        text(x + 20, y + 18, tc, std::str::from_utf8(&s).unwrap_or("?"));
                    }
                }
            }
        }

        // Clues panel
        let px = GX + 5 * (CELL + GAP) + 20;
        fill(px, 32, W - px - 8, H - 40, C_CARD);
        border(px, 32, W - px - 8, H - 40, C_BORDER);

        text(px + 8, 40, C_ORANGE, "ACROSS");
        for (i, clue) in CLUES_ACROSS.iter().enumerate() {
            if !clue.is_empty() {
                text(px + 8, 58 + i as i32 * 16, C_HINT, clue);
            }
        }

        text(px + 8, 110, C_ORANGE, "DOWN");
        for (i, clue) in CLUES_DOWN.iter().enumerate() {
            if !clue.is_empty() {
                text(px + 8, 128 + i as i32 * 16, C_HINT, clue);
            }
        }

        if self.checked {
            let filled: u32 = (0..5).flat_map(|r| (0..5usize).map(move |c| (r, c)))
                .filter(|&(r, c)| !Self::is_black(r, c) && self.user[r][c] != b' ')
                .count() as u32;
            text(px + 8, 220, C_TEXT, &format!("Correct: {}/20", self.score));
            if self.score == 20 { text(px + 8, 238, C_GREEN, "SOLVED!"); }
            let _ = filled;
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        let row = self.cy; let col = self.cx;
        text(12, H - 18, C_HINT, &format!("Cursor ({},{})  C=check  R=reveal", row + 1, col + 1));

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[A" => self.move_cursor(-1, 0),
            "\x1b[B" => self.move_cursor(1, 0),
            "\x1b[C" => self.move_cursor(0, 1),
            "\x1b[D" => self.move_cursor(0, -1),
            "c" | "C" => self.check(),
            "r" | "R" => self.reveal_one(),
            "\x7f" => {
                if !Self::is_black(self.cy, self.cx) {
                    self.user[self.cy][self.cx] = b' ';
                    self.state[self.cy][self.cx] = 0;
                    self.checked = false;
                }
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {
                if line.len() == 1 {
                    let b = line.as_bytes()[0];
                    if b.is_ascii_alphabetic() && !Self::is_black(self.cy, self.cx) {
                        self.user[self.cy][self.cx] = b.to_ascii_uppercase();
                        self.state[self.cy][self.cx] = 0;
                        self.checked = false;
                        // advance cursor right, then down
                        let nc = self.cx + 1;
                        if nc < 5 && !Self::is_black(self.cy, nc) {
                            self.cx = nc;
                        }
                    }
                }
            }
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
