use std::io::{self, BufRead, Write};

const W: u32 = 640;
const H: u32 = 740;
const HEADER_H: u32 = 48;
const CELL: u32 = 60;
const GRID_X: u32 = (W - 9 * CELL) / 2; // 50
const GRID_Y: u32 = 64;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_RED: u32     = 0xFF7B72FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;
const C_GIVEN_BG: u32 = 0x161B22FF;
const C_BOX_BORDER: u32 = 0x8B949EFF;

// A classic easy Sudoku puzzle (0 = empty)
#[rustfmt::skip]
const PUZZLE: [[u8; 9]; 9] = [
    [5, 3, 0,  0, 7, 0,  0, 0, 0],
    [6, 0, 0,  1, 9, 5,  0, 0, 0],
    [0, 9, 8,  0, 0, 0,  0, 6, 0],

    [8, 0, 0,  0, 6, 0,  0, 0, 3],
    [4, 0, 0,  8, 0, 3,  0, 0, 1],
    [7, 0, 0,  0, 2, 0,  0, 0, 6],

    [0, 6, 0,  0, 0, 0,  2, 8, 0],
    [0, 0, 0,  4, 1, 9,  0, 0, 5],
    [0, 0, 0,  0, 8, 0,  0, 7, 9],
];

#[rustfmt::skip]
const SOLUTION: [[u8; 9]; 9] = [
    [5, 3, 4,  6, 7, 8,  9, 1, 2],
    [6, 7, 2,  1, 9, 5,  3, 4, 8],
    [1, 9, 8,  3, 4, 2,  5, 6, 7],

    [8, 5, 9,  7, 6, 1,  4, 2, 3],
    [4, 2, 6,  8, 5, 3,  7, 9, 1],
    [7, 1, 3,  9, 2, 4,  8, 5, 6],

    [9, 6, 1,  5, 3, 7,  2, 8, 4],
    [2, 8, 7,  4, 1, 9,  6, 3, 5],
    [3, 4, 5,  2, 8, 6,  1, 7, 9],
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

struct Game {
    board:    [[u8; 9]; 9],
    given:    [[bool; 9]; 9],
    conflict: [[bool; 9]; 9],
    cursor_r: usize,
    cursor_c: usize,
    solved:   bool,
    check_on: bool,
}

impl Game {
    fn new() -> Self {
        let mut board = [[0u8; 9]; 9];
        let mut given = [[false; 9]; 9];
        for r in 0..9 {
            for c in 0..9 {
                board[r][c] = PUZZLE[r][c];
                given[r][c] = PUZZLE[r][c] != 0;
            }
        }
        Game {
            board,
            given,
            conflict: [[false; 9]; 9],
            cursor_r: 0,
            cursor_c: 0,
            solved:   false,
            check_on: false,
        }
    }

    fn reset(&mut self) {
        for r in 0..9 {
            for c in 0..9 {
                self.board[r][c] = PUZZLE[r][c];
                self.conflict[r][c] = false;
            }
        }
        self.solved = false;
        self.check_on = false;
    }

    fn solve(&mut self) {
        for r in 0..9 {
            for c in 0..9 {
                self.board[r][c] = SOLUTION[r][c];
                self.conflict[r][c] = false;
            }
        }
        self.solved = true;
    }

    fn enter_digit(&mut self, d: u8) {
        let r = self.cursor_r;
        let c = self.cursor_c;
        if self.given[r][c] { return; }
        self.board[r][c] = d;
        if self.check_on { self.compute_conflicts(); }
        self.check_solved();
    }

    fn compute_conflicts(&mut self) {
        self.conflict = [[false; 9]; 9];
        for r in 0..9 {
            for c in 0..9 {
                let v = self.board[r][c];
                if v == 0 { continue; }
                // row
                for c2 in 0..9 {
                    if c2 != c && self.board[r][c2] == v {
                        self.conflict[r][c]  = true;
                        self.conflict[r][c2] = true;
                    }
                }
                // col
                for r2 in 0..9 {
                    if r2 != r && self.board[r2][c] == v {
                        self.conflict[r][c]  = true;
                        self.conflict[r2][c] = true;
                    }
                }
                // box
                let br = (r / 3) * 3;
                let bc = (c / 3) * 3;
                for dr in 0..3 {
                    for dc in 0..3 {
                        let r2 = br + dr;
                        let c2 = bc + dc;
                        if (r2 != r || c2 != c) && self.board[r2][c2] == v {
                            self.conflict[r][c]  = true;
                            self.conflict[r2][c2] = true;
                        }
                    }
                }
            }
        }
    }

    fn check_solved(&mut self) {
        for r in 0..9 {
            for c in 0..9 {
                if self.board[r][c] == 0 { return; }
            }
        }
        self.compute_conflicts();
        let any_conflict = self.conflict.iter().any(|row| row.iter().any(|&b| b));
        if !any_conflict { self.solved = true; }
    }

    fn move_cursor(&mut self, dr: i32, dc: i32) {
        let nr = (self.cursor_r as i32 + dr).rem_euclid(9) as usize;
        let nc = (self.cursor_c as i32 + dc).rem_euclid(9) as usize;
        self.cursor_r = nr;
        self.cursor_c = nc;
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Sudoku");
    text(120, 16, C_HINT, "Arrows:move  1-9:enter  0/Bsp:clear  c:check  r:reset  s:solve");

    // Draw cells
    let grid_w = 9 * CELL;
    let grid_h = 9 * CELL;
    fill(GRID_X, GRID_Y, grid_w, grid_h, 0x161B22FF);

    for r in 0..9usize {
        for c in 0..9usize {
            let px = GRID_X + c as u32 * CELL;
            let py = GRID_Y + r as u32 * CELL;

            let is_sel = r == g.cursor_r && c == g.cursor_c;
            let is_conflict = g.conflict[r][c];
            let is_given = g.given[r][c];

            let bg = if is_sel {
                C_SEL_BG
            } else if is_conflict {
                0x3D1A1AFF
            } else if is_given {
                C_GIVEN_BG
            } else {
                C_BG
            };
            fill(px + 1, py + 1, CELL - 2, CELL - 2, bg);

            let v = g.board[r][c];
            if v != 0 {
                let fg = if is_conflict {
                    C_RED
                } else if is_given {
                    C_TEXT
                } else {
                    C_SEL
                };
                let s = v.to_string();
                let tx = px + (CELL - 16) / 2;
                let ty = py + (CELL - 32) / 2;
                text_l(tx, ty, fg, &s);
            }
        }
    }

    // Thin cell lines
    for i in 0..=9u32 {
        let x = GRID_X + i * CELL;
        let y = GRID_Y + i * CELL;
        fill(x, GRID_Y, 1, grid_h, C_BORDER);
        fill(GRID_X, y, grid_w, 1, C_BORDER);
    }

    // Bold 3×3 box borders
    for b in 0..=3u32 {
        let x = GRID_X + b * 3 * CELL;
        let y = GRID_Y + b * 3 * CELL;
        fill(x, GRID_Y, 2, grid_h, C_BOX_BORDER);
        fill(GRID_X, y, grid_w, 2, C_BOX_BORDER);
    }

    // Outer border
    border(GRID_X, GRID_Y, grid_w, grid_h, C_BOX_BORDER);

    // Status bar
    let status_y = GRID_Y + grid_h + 16;
    if g.check_on {
        let any_conflict = g.conflict.iter().any(|row| row.iter().any(|&b| b));
        if any_conflict {
            text(GRID_X, status_y, C_RED, "Conflicts found (red cells)");
        } else {
            text(GRID_X, status_y, C_GREEN, "No conflicts detected");
        }
    }

    if g.solved {
        let bx = (W - 240) / 2;
        let by = (H - 80) / 2;
        fill(bx, by, 240, 80, C_HEADER);
        border(bx, by, 240, 80, C_GREEN);
        text(bx + 60, by + 12, C_GREEN, "Solved!");
        text(bx + 28, by + 40, C_HINT, "r=reset  s=solve again");
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise sudoku");
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
            "\x7f" | "0" => { game.enter_digit(0); }
            "c" | "C" => {
                game.check_on = true;
                game.compute_conflicts();
            }
            "r" | "R" => { game.reset(); }
            "s" | "S" => { game.solve(); }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= b'1' && b <= b'9' {
                    game.enter_digit(b - b'0');
                }
            }
            _ => {}
        }
        draw(&game);
    }
}
