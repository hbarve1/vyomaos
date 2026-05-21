use std::io::{self, BufRead, Write};

const W: u32 = 560;
const H: u32 = 860;
const HEADER_H: u32 = 40;
const GCOLS: usize = 10;
const GROWS: usize = 20;
const CELL: u32 = 36;
const GRID_X: u32 = 100; // (560 - 10*36) / 2
const GRID_Y: u32 = 50;
const PCELL: u32 = 14;   // preview cell size

const C_BG: u32     = 0x0D1117FF;
const C_GRID: u32   = 0x090D12FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_RED: u32    = 0xFF7B72FF;

const PIECE_COLORS: [u32; 7] = [
    0x58A6FFFF, // I - blue
    0xFFA657FF, // O - orange
    0xD2A8FFFF, // T - purple
    0x3FB950FF, // S - green
    0xFF7B72FF, // Z - red
    0x79C0FFFF, // J - cyan
    0xF0883EFF, // L - amber
];

const SPAWN_COL: [i32; 7] = [3, 4, 3, 3, 3, 3, 3];

// [piece][rotation][cell] = [row_offset, col_offset]
const PIECES: [[[[i8; 2]; 4]; 4]; 7] = [
    // I
    [[[0,0],[0,1],[0,2],[0,3]], [[0,1],[1,1],[2,1],[3,1]], [[0,0],[0,1],[0,2],[0,3]], [[0,1],[1,1],[2,1],[3,1]]],
    // O
    [[[0,0],[0,1],[1,0],[1,1]], [[0,0],[0,1],[1,0],[1,1]], [[0,0],[0,1],[1,0],[1,1]], [[0,0],[0,1],[1,0],[1,1]]],
    // T
    [[[0,0],[0,1],[0,2],[1,1]], [[0,1],[1,0],[1,1],[2,1]], [[0,1],[1,0],[1,1],[1,2]], [[0,0],[1,0],[1,1],[2,0]]],
    // S
    [[[0,1],[0,2],[1,0],[1,1]], [[0,0],[1,0],[1,1],[2,1]], [[0,1],[0,2],[1,0],[1,1]], [[0,0],[1,0],[1,1],[2,1]]],
    // Z
    [[[0,0],[0,1],[1,1],[1,2]], [[0,1],[1,0],[1,1],[2,0]], [[0,0],[0,1],[1,1],[1,2]], [[0,1],[1,0],[1,1],[2,0]]],
    // J
    [[[0,0],[1,0],[1,1],[1,2]], [[0,0],[0,1],[1,0],[2,0]], [[0,0],[0,1],[0,2],[1,2]], [[0,1],[1,1],[2,0],[2,1]]],
    // L
    [[[0,2],[1,0],[1,1],[1,2]], [[0,0],[1,0],[2,0],[2,1]], [[0,0],[0,1],[0,2],[1,0]], [[0,0],[0,1],[1,1],[2,1]]],
];

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
enum State { Waiting, Playing, Lost }

struct Game {
    board:    [[u32; GCOLS]; GROWS],
    cur_t:    usize, cur_r: usize, cur_row: i32, cur_col: i32,
    next_t:   usize,
    score:    u32,
    lines:    u32,
    level:    u32,
    state:    State,
    rng:      u64,
}

impl Game {
    fn new() -> Self {
        let mut rng: u64 = 0xDEADBEEF12345678;
        rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let next_t = (rng >> 33) as usize % 7;
        Game {
            board: [[0u32; GCOLS]; GROWS],
            cur_t: 0, cur_r: 0, cur_row: 0, cur_col: SPAWN_COL[0],
            next_t,
            score: 0, lines: 0, level: 0,
            state: State::Waiting,
            rng,
        }
    }

    fn lcg(&mut self) -> u64 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.rng >> 33
    }

    fn can_place(&self, pt: usize, rot: usize, row: i32, col: i32) -> bool {
        for &[dr, dc] in &PIECES[pt][rot] {
            let r = row + dr as i32;
            let c = col + dc as i32;
            if r < 0 { continue; }
            if r >= GROWS as i32 || c < 0 || c >= GCOLS as i32 { return false; }
            if self.board[r as usize][c as usize] != 0 { return false; }
        }
        true
    }

    fn lock_piece(&mut self) {
        let color = PIECE_COLORS[self.cur_t];
        for &[dr, dc] in &PIECES[self.cur_t][self.cur_r] {
            let r = (self.cur_row + dr as i32) as usize;
            let c = (self.cur_col + dc as i32) as usize;
            self.board[r][c] = color;
        }
    }

    fn clear_lines(&mut self) -> u32 {
        let mut cleared = 0u32;
        let mut write = (GROWS - 1) as i32;
        let mut read = (GROWS - 1) as i32;
        while read >= 0 {
            let full = (0..GCOLS).all(|c| self.board[read as usize][c] != 0);
            if full {
                cleared += 1;
            } else {
                if write != read {
                    self.board[write as usize] = self.board[read as usize];
                }
                write -= 1;
            }
            read -= 1;
        }
        while write >= 0 {
            self.board[write as usize] = [0u32; GCOLS];
            write -= 1;
        }
        cleared
    }

    fn spawn_next(&mut self) {
        self.cur_t = self.next_t;
        self.cur_r = 0;
        self.cur_row = 0;
        self.cur_col = SPAWN_COL[self.cur_t];
        let next = self.lcg() as usize % 7;
        self.next_t = next;
        if !self.can_place(self.cur_t, self.cur_r, self.cur_row, self.cur_col) {
            self.state = State::Lost;
        }
    }

    fn lock_and_advance(&mut self) {
        self.lock_piece();
        let cleared = self.clear_lines();
        self.score += match cleared {
            1 => 100, 2 => 300, 3 => 500, 4 => 800, _ => 0,
        } * (self.level + 1);
        self.lines += cleared;
        self.level = self.lines / 10;
        self.spawn_next();
    }

    fn drop_interval(&self) -> u32 {
        match self.level { 0=>8, 1=>7, 2=>6, 3=>5, 4=>4, 5=>3, 6=>2, _=>1 }
    }

    fn ghost_row(&self) -> i32 {
        let mut gr = self.cur_row;
        while self.can_place(self.cur_t, self.cur_r, gr + 1, self.cur_col) { gr += 1; }
        gr
    }

    fn step(&mut self) {
        if self.state != State::Playing { return; }
        let nr = self.cur_row + 1;
        if self.can_place(self.cur_t, self.cur_r, nr, self.cur_col) {
            self.cur_row = nr;
        } else {
            self.lock_and_advance();
        }
    }

    fn hard_drop(&mut self) {
        self.cur_row = self.ghost_row();
        self.score += 2;
        self.lock_and_advance();
    }

    fn rotate_cw(&mut self) {
        if self.state != State::Playing { return; }
        let nr = (self.cur_r + 1) % 4;
        // Try rotation, then wall kicks
        for dc in [0i32, 1, -1, 2, -2] {
            if self.can_place(self.cur_t, nr, self.cur_row, self.cur_col + dc) {
                self.cur_r = nr;
                self.cur_col += dc;
                return;
            }
        }
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_TEXT, "Tetris");
    text(100, 12, C_ORANGE, &format!("Score: {}  Lv: {}  Lines: {}", g.score, g.level + 1, g.lines));

    // Grid background
    fill(GRID_X, GRID_Y, GCOLS as u32 * CELL, GROWS as u32 * CELL, C_GRID);
    border(GRID_X, GRID_Y, GCOLS as u32 * CELL, GROWS as u32 * CELL, C_BORDER);

    // Subtle grid lines
    for r in 0..GROWS as u32 {
        fill(GRID_X, GRID_Y + r * CELL, GCOLS as u32 * CELL, 1, 0x15191EFF);
    }

    // Locked board cells
    for r in 0..GROWS {
        for c in 0..GCOLS {
            if g.board[r][c] != 0 {
                let px = GRID_X + c as u32 * CELL + 1;
                let py = GRID_Y + r as u32 * CELL + 1;
                fill(px, py, CELL - 2, CELL - 2, g.board[r][c]);
                fill(px, py, CELL - 2, 2, 0xFFFFFF28);
            }
        }
    }

    // Ghost piece
    if g.state == State::Playing {
        let gr = g.ghost_row();
        if gr != g.cur_row {
            for &[dr, dc] in &PIECES[g.cur_t][g.cur_r] {
                let r = gr + dr as i32;
                let c = g.cur_col + dc as i32;
                if r >= 0 && r < GROWS as i32 && c >= 0 && c < GCOLS as i32 {
                    let px = GRID_X + c as u32 * CELL + 1;
                    let py = GRID_Y + r as u32 * CELL + 1;
                    border(px, py, CELL - 2, CELL - 2, PIECE_COLORS[g.cur_t]);
                }
            }
        }

        // Current piece
        let color = PIECE_COLORS[g.cur_t];
        for &[dr, dc] in &PIECES[g.cur_t][g.cur_r] {
            let r = g.cur_row + dr as i32;
            let c = g.cur_col + dc as i32;
            if r >= 0 && r < GROWS as i32 && c >= 0 && c < GCOLS as i32 {
                let px = GRID_X + c as u32 * CELL + 1;
                let py = GRID_Y + r as u32 * CELL + 1;
                fill(px, py, CELL - 2, CELL - 2, color);
                fill(px, py, CELL - 2, 2, 0xFFFFFF38);
            }
        }
    }

    // Right panel: score + next piece
    let px = GRID_X + GCOLS as u32 * CELL + 16;
    text(px, GRID_Y, C_HINT, "Next:");
    for &[dr, dc] in &PIECES[g.next_t][0] {
        let bx = px + dc as u32 * PCELL;
        let by = GRID_Y + 20 + dr as u32 * PCELL;
        fill(bx, by, PCELL - 1, PCELL - 1, PIECE_COLORS[g.next_t]);
    }
    text(px, GRID_Y + 90, C_HINT, "Keys:");
    text(px, GRID_Y + 108, C_HINT, "< > move");
    text(px, GRID_Y + 124, C_HINT, "^ rotate");
    text(px, GRID_Y + 140, C_HINT, "v soft drop");
    text(px, GRID_Y + 156, C_HINT, "Sp hard drop");

    // Waiting overlay
    if g.state == State::Waiting {
        let gw = GCOLS as u32 * CELL;
        let gh = GROWS as u32 * CELL;
        fill(GRID_X + gw/2 - 80, GRID_Y + gh/2 - 20, 160, 40, C_HEADER);
        border(GRID_X + gw/2 - 80, GRID_Y + gh/2 - 20, 160, 40, C_BORDER);
        text(GRID_X + gw/2 - 60, GRID_Y + gh/2 - 8, C_HINT, "Space to start");
    }

    // Game over overlay
    if g.state == State::Lost {
        let gw = GCOLS as u32 * CELL;
        let gh = GROWS as u32 * CELL;
        let bx = GRID_X + gw/2 - 88;
        let by = GRID_Y + gh/2 - 32;
        fill(bx, by, 176, 64, C_HEADER);
        border(bx, by, 176, 64, C_RED);
        text(bx + 24, by + 10, C_RED, "Game Over!");
        text(bx + 16, by + 32, C_HINT, "R to restart");
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();
    let mut ticks = 0u32;

    println!("@supervisor: raise tetris");
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            if game.state == State::Playing {
                ticks += 1;
                if ticks >= game.drop_interval() {
                    ticks = 0;
                    game.step();
                    draw(&game);
                }
            }
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            " " => {
                match game.state {
                    State::Waiting => { game.state = State::Playing; game.spawn_next(); }
                    State::Playing => { game.hard_drop(); }
                    _ => {}
                }
            }
            "\x1b[A" => { game.rotate_cw(); }
            "\x1b[B" => {
                if game.state == State::Playing {
                    let nr = game.cur_row + 1;
                    if game.can_place(game.cur_t, game.cur_r, nr, game.cur_col) {
                        game.cur_row = nr;
                        game.score += 1;
                    }
                }
            }
            "\x1b[D" => {
                if game.state == State::Playing {
                    let nc = game.cur_col - 1;
                    if game.can_place(game.cur_t, game.cur_r, game.cur_row, nc) {
                        game.cur_col = nc;
                    }
                }
            }
            "\x1b[C" => {
                if game.state == State::Playing {
                    let nc = game.cur_col + 1;
                    if game.can_place(game.cur_t, game.cur_r, game.cur_row, nc) {
                        game.cur_col = nc;
                    }
                }
            }
            "r" | "R" => { game = Game::new(); ticks = 0; }
            _ => {}
        }
        draw(&game);
    }
}
