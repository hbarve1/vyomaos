use std::io::{self, BufRead, Write};

const W: u32 = 600;
const H: u32 = 700;
const HEADER_H: u32 = 48;
const CELL: u32 = 120;
const GAP: u32 = 8;
const GRID_X: u32 = 60;  // (600 - 4*120 - 3*8) / 2
const GRID_Y: u32 = 68;  // 48 + 20

const C_BG: u32     = 0x0D1117FF;
const C_GRID_BG: u32 = 0x161B22FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_RED: u32    = 0xFF7B72FF;

fn tile_color(v: u32) -> u32 {
    match v {
        2    => 0x2A2D3EFF,
        4    => 0x363B52FF,
        8    => 0xFF6B35FF,
        16   => 0xFF4500FF,
        32   => 0xE83232FF,
        64   => 0xCC1111FF,
        128  => 0xFAD02CFF,
        256  => 0xF5A623FF,
        512  => 0x56B4D3FF,
        1024 => 0x3FB950FF,
        2048 => 0x58A6FFFF,
        _    => 0xD2A8FFFF,
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

fn slide_seq(seq: [u32; 4], score: &mut u32) -> ([u32; 4], bool) {
    let orig = seq;
    let mut buf = [0u32; 4];
    let mut j = 0;
    for &v in &seq { if v != 0 { buf[j] = v; j += 1; } }
    let mut out = [0u32; 4];
    let mut oi = 0;
    let mut bi = 0;
    while bi < 4 && buf[bi] != 0 {
        if bi + 1 < 4 && buf[bi] == buf[bi + 1] && buf[bi + 1] != 0 {
            out[oi] = buf[bi] * 2;
            *score += out[oi];
            bi += 2;
        } else {
            out[oi] = buf[bi];
            bi += 1;
        }
        oi += 1;
    }
    (out, out != orig)
}

struct Game {
    board: [[u32; 4]; 4],
    score: u32,
    best:  u32,
    won:   bool,
    over:  bool,
    rng:   u64,
}

impl Game {
    fn new() -> Self {
        let mut g = Game {
            board: [[0u32; 4]; 4],
            score: 0, best: 0,
            won: false, over: false,
            rng: 0xFABFABFABFABFAB0,
        };
        g.spawn(); g.spawn();
        g
    }

    fn lcg(&mut self) -> u64 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.rng >> 33
    }

    fn spawn(&mut self) {
        let mut cells = [(0usize, 0usize); 16];
        let mut n = 0;
        for r in 0..4 { for c in 0..4 { if self.board[r][c] == 0 { cells[n] = (r, c); n += 1; } } }
        if n == 0 { return; }
        let idx = self.lcg() as usize % n;
        let (r, c) = cells[idx];
        self.board[r][c] = if self.lcg() % 10 == 0 { 4 } else { 2 };
    }

    fn has_moves(&self) -> bool {
        for r in 0..4 { for c in 0..4 {
            if self.board[r][c] == 0 { return true; }
            if c + 1 < 4 && self.board[r][c] == self.board[r][c+1] { return true; }
            if r + 1 < 4 && self.board[r][c] == self.board[r+1][c] { return true; }
        }}
        false
    }

    fn check_win(&mut self) {
        if !self.won {
            for r in 0..4 { for c in 0..4 { if self.board[r][c] >= 2048 { self.won = true; } } }
        }
    }

    fn slide_left(&mut self) -> bool {
        let mut any = false;
        for r in 0..4 {
            let (row, changed) = slide_seq(self.board[r], &mut self.score);
            if changed { self.board[r] = row; any = true; }
        }
        any
    }

    fn slide_right(&mut self) -> bool {
        let mut any = false;
        for r in 0..4 {
            let mut rev = self.board[r]; rev.reverse();
            let (mut row, changed) = slide_seq(rev, &mut self.score);
            if changed { row.reverse(); self.board[r] = row; any = true; }
        }
        any
    }

    fn slide_up(&mut self) -> bool {
        let mut any = false;
        for c in 0..4 {
            let col = [self.board[0][c], self.board[1][c], self.board[2][c], self.board[3][c]];
            let (new_col, changed) = slide_seq(col, &mut self.score);
            if changed { for r in 0..4 { self.board[r][c] = new_col[r]; } any = true; }
        }
        any
    }

    fn slide_down(&mut self) -> bool {
        let mut any = false;
        for c in 0..4 {
            let col = [self.board[3][c], self.board[2][c], self.board[1][c], self.board[0][c]];
            let (new_col, changed) = slide_seq(col, &mut self.score);
            if changed { for r in 0..4 { self.board[3-r][c] = new_col[r]; } any = true; }
        }
        any
    }

    fn make_move(&mut self, moved: bool) {
        if moved {
            if self.score > self.best { self.best = self.score; }
            self.spawn();
            self.check_win();
            if !self.has_moves() { self.over = true; }
        }
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "2048");
    text(100, 16, C_HINT, &format!("Score: {}  Best: {}", g.score, g.best));
    text(380, 16, C_HINT, "Arrows: slide  R: restart");

    // Grid background
    let grid_w = 4 * CELL + 3 * GAP;
    let grid_h = 4 * CELL + 3 * GAP;
    fill(GRID_X - GAP, GRID_Y - GAP, grid_w + 2 * GAP, grid_h + 2 * GAP, C_GRID_BG);
    border(GRID_X - GAP, GRID_Y - GAP, grid_w + 2 * GAP, grid_h + 2 * GAP, C_BORDER);

    // Tiles
    for r in 0..4 {
        for c in 0..4 {
            let px = GRID_X + c as u32 * (CELL + GAP);
            let py = GRID_Y + r as u32 * (CELL + GAP);
            let val = g.board[r][c];

            if val == 0 {
                fill(px, py, CELL, CELL, 0x0A0D12FF);
            } else {
                let color = tile_color(val);
                fill(px, py, CELL, CELL, color);
                fill(px, py, CELL, 3, 0xFFFFFF28);

                let s = val.to_string();
                // Center number: 'l' font ~16px wide, 32px tall
                let tw = s.len() as u32 * 16;
                let tx = px + (CELL - tw.min(CELL)) / 2;
                let ty = py + (CELL - 32) / 2;
                let fg = if val <= 4 { C_HINT } else { C_TEXT };
                text_l(tx, ty, fg, &s);
            }
        }
    }

    // Won overlay (non-blocking — game continues)
    if g.won && !g.over {
        let msg = "You reached 2048! Keep going!";
        let mw = msg.len() as u32 * 8 + 20;
        fill((W - mw) / 2, H - 40, mw, 28, C_HEADER);
        border((W - mw) / 2, H - 40, mw, 28, C_GREEN);
        text((W - mw) / 2 + 10, H - 30, C_GREEN, msg);
    }

    // Game over overlay
    if g.over {
        let bx = (W - 240) / 2;
        let by = (H - 64) / 2;
        fill(bx, by, 240, 64, C_HEADER);
        border(bx, by, 240, 64, C_RED);
        text(bx + 32, by + 10, C_RED, "No more moves!");
        text(bx + 20, by + 32, C_HINT, &format!("Score: {}  R to restart", g.score));
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise 2048");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[D" => { let m = game.slide_left();  game.make_move(m); }
            "\x1b[C" => { let m = game.slide_right(); game.make_move(m); }
            "\x1b[A" => { let m = game.slide_up();    game.make_move(m); }
            "\x1b[B" => { let m = game.slide_down();  game.make_move(m); }
            "r" | "R" => {
                let best = game.best.max(game.score);
                game = Game::new();
                game.best = best;
            }
            _ => {}
        }
        draw(&game);
    }
}
