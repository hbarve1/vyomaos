use std::io::{self, BufRead, Write};

const W: i32 = 640;
const H: i32 = 680;

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

// Cell: 0=empty, 1=X(human), 2=O(AI)
type Board = [u8; 9];

const WINS: [[usize; 3]; 8] = [
    [0,1,2],[3,4,5],[6,7,8], // rows
    [0,3,6],[1,4,7],[2,5,8], // cols
    [0,4,8],[2,4,6],         // diags
];

fn winner(b: &Board) -> u8 {
    for &[a,c,d] in &WINS {
        if b[a] != 0 && b[a] == b[c] && b[c] == b[d] { return b[a]; }
    }
    0
}

fn is_full(b: &Board) -> bool { b.iter().all(|&c| c != 0) }

// Minimax — returns score for current player (maximiser = O=2)
fn minimax(b: &mut Board, is_max: bool, depth: i32) -> i32 {
    let w = winner(b);
    if w == 2 { return 10 - depth; }
    if w == 1 { return depth - 10; }
    if is_full(b) { return 0; }
    if is_max {
        let mut best = i32::MIN;
        for i in 0..9 {
            if b[i] == 0 {
                b[i] = 2;
                best = best.max(minimax(b, false, depth + 1));
                b[i] = 0;
            }
        }
        best
    } else {
        let mut best = i32::MAX;
        for i in 0..9 {
            if b[i] == 0 {
                b[i] = 1;
                best = best.min(minimax(b, true, depth + 1));
                b[i] = 0;
            }
        }
        best
    }
}

fn ai_move(b: &mut Board) -> usize {
    let mut best_score = i32::MIN;
    let mut best_idx = 0;
    for i in 0..9 {
        if b[i] == 0 {
            b[i] = 2;
            let s = minimax(b, false, 0);
            b[i] = 0;
            if s > best_score { best_score = s; best_idx = i; }
        }
    }
    best_idx
}

const CELL: i32 = 130;
const GX: i32   = (W - CELL * 3) / 2;
const GY: i32   = 80;

struct App {
    board:  Board,
    cursor: usize,
    wins:   u32,
    draws:  u32,
    losses: u32,
    over:   bool,
    result: &'static str,
}

impl App {
    fn new() -> Self { App { board: [0;9], cursor: 4, wins: 0, draws: 0, losses: 0, over: false, result: "" } }

    fn new_game(&mut self) { self.board = [0;9]; self.cursor = 4; self.over = false; self.result = ""; }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Tic Tac Toe");
        text(160, 8, C_HINT, "Arrows:move  Enter:place  R:new game  Q:quit");

        // Score row
        text(GX, 48, C_GREEN,  &format!("W:{}", self.wins));
        text(GX + 60, 48, C_HINT, &format!("D:{}", self.draws));
        text(GX + 120, 48, C_RED, &format!("L:{}", self.losses));
        text(GX + 220, 48, C_HINT, "You=X  AI=O");

        // Grid
        for r in 0..3usize {
            for c in 0..3usize {
                let i = r * 3 + c;
                let x = GX + c as i32 * CELL;
                let y = GY + r as i32 * CELL;
                let sel = !self.over && i == self.cursor;
                let bg = if sel { 0x1A2040FF } else { C_CARD };
                fill(x, y, CELL - 4, CELL - 4, bg);
                border(x, y, CELL - 4, CELL - 4, if sel { C_SEL } else { C_BORDER });

                match self.board[i] {
                    1 => {
                        // Draw X as two thick diagonal lines using fill
                        for k in 0..CELL-24 {
                            fill(x + 12 + k, y + 12 + k,       4, 4, 0xFF7B72FF);
                            fill(x + CELL - 20 - k, y + 12 + k, 4, 4, 0xFF7B72FF);
                        }
                    }
                    2 => {
                        // Draw O as a thick ring using fill
                        let cx2 = x + (CELL - 4) / 2;
                        let cy2 = y + (CELL - 4) / 2;
                        let r_out = (CELL - 24) / 2;
                        let r_in  = r_out - 10;
                        for dy in -r_out..=r_out {
                            for dx in -r_out..=r_out {
                                let d2 = dx*dx + dy*dy;
                                if d2 <= r_out*r_out && d2 >= r_in*r_in {
                                    fill(cx2 + dx, cy2 + dy, 2, 2, 0x58A6FFFF);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        // Result overlay
        if self.over {
            fill(GX - 10, GY + CELL + 20, CELL * 3 + 24, 60, C_CARD);
            border(GX - 10, GY + CELL + 20, CELL * 3 + 24, 60,
                   if self.result == "You win!" { C_GREEN } else if self.result == "You lose." { C_RED } else { C_ORANGE });
            let tc = if self.result == "You win!" { C_GREEN } else if self.result == "You lose." { C_RED } else { C_ORANGE };
            text(GX + 50, GY + CELL + 42, tc, self.result);
            text(GX + 50, GY + CELL + 62, C_HINT, "Press R for new game");
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        let status = if self.over { self.result } else { "Your turn (X)" };
        text(12, H - 18, C_HINT, status);
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "r" | "R" => { self.new_game(); self.draw(); return; }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        if self.over { self.draw(); return; }

        match line {
            "\x1b[A" => { if self.cursor >= 3 { self.cursor -= 3; } }
            "\x1b[B" => { if self.cursor < 6   { self.cursor += 3; } }
            "\x1b[D" => { if self.cursor % 3 > 0 { self.cursor -= 1; } }
            "\x1b[C" => { if self.cursor % 3 < 2 { self.cursor += 1; } }
            "\r" | "" => {
                if self.board[self.cursor] == 0 {
                    self.board[self.cursor] = 1;
                    let w = winner(&self.board);
                    if w == 1 { self.wins += 1; self.result = "You win!"; self.over = true; }
                    else if is_full(&self.board) { self.draws += 1; self.result = "Draw!"; self.over = true; }
                    else {
                        let ai = ai_move(&mut self.board);
                        self.board[ai] = 2;
                        let w2 = winner(&self.board);
                        if w2 == 2 { self.losses += 1; self.result = "You lose."; self.over = true; }
                        else if is_full(&self.board) { self.draws += 1; self.result = "Draw!"; self.over = true; }
                    }
                }
            }
            _ => {}
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
