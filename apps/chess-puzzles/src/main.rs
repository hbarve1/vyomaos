use std::io::{self, BufRead, Write};

const W: i32 = 760;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_CARD: u32   = 0x161B22FF;
const C_SEL: u32    = 0x58A6FFFF;

const SQ: i32 = 56;
const BX: i32 = 32;
const BY: i32 = 56;
const C_LIGHT: u32 = 0x4A6741FF;
const C_DARK:  u32 = 0x2D3B2AFF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c);
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s);
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

struct Puzzle {
    name:          &'static str,
    depth:         u8,
    white_to_move: bool,
    // 64 chars: row0=rank8, row7=rank1; col0=file_a, col7=file_h
    // Uppercase=white (K Q R B N P), lowercase=black, .=empty
    board:         &'static str,
    sol:           (usize, usize, usize, usize), // (from_row, from_col, to_row, to_col)
    hint:          &'static str,
}

static PUZZLES: &[Puzzle] = &[
    Puzzle { name: "Back Rank", depth: 1, white_to_move: true,
      board: "......k......ppp.....................................PPP....R.K.",
      sol: (7,4,0,4), hint: "Rook to e8!" },
    Puzzle { name: "Ladder Mate", depth: 1, white_to_move: true,
      board: "k........R.......R......................................K.......",
      sol: (1,1,1,0), hint: "Rook slides to a7!" },
    Puzzle { name: "Smothered I", depth: 1, white_to_move: true,
      board: ".......k......pp...........................................K..R.",
      sol: (7,6,0,6), hint: "Rook to g8!" },
    Puzzle { name: "Queen March", depth: 1, white_to_move: true,
      board: "......k......ppp............................Q...............K...",
      sol: (5,4,0,4), hint: "Queen to e8!" },
    Puzzle { name: "Queen Box", depth: 1, white_to_move: true,
      board: ".......k......pp....................Q.......................K...",
      sol: (4,4,0,4), hint: "Queen to e8!" },
    Puzzle { name: "Q Diagonal", depth: 1, white_to_move: true,
      board: "kn......p..............................................Q.......K",
      sol: (6,7,0,1), hint: "Queen to b8!" },
    Puzzle { name: "Promotion", depth: 1, white_to_move: true,
      board: ".......k......Pp............................................K..R",
      sol: (1,6,0,6), hint: "Promote the pawn!" },
    Puzzle { name: "Double Rook", depth: 1, white_to_move: true,
      board: "k.......pp..............................RR..................K...",
      sol: (5,1,0,1), hint: "Rook to b8!" },
    Puzzle { name: "Q Corridor", depth: 1, white_to_move: true,
      board: "kn...............................................Q......K.......",
      sol: (6,1,1,1), hint: "Queen to b7!" },
    Puzzle { name: "Rook Pair", depth: 1, white_to_move: true,
      board: "k........R......R.............................................K.",
      sol: (1,1,0,1), hint: "Rook to b8!" },
    Puzzle { name: "Rook Check", depth: 2, white_to_move: true,
      board: "......k......ppp.............................................R.K",
      sol: (7,5,0,5), hint: "Rook checks, then mate!" },
    Puzzle { name: "Knight Fork", depth: 2, white_to_move: true,
      board: "....k.......p..............N................................K...",
      sol: (3,3,2,5), hint: "Knight to f6!" },
    Puzzle { name: "Rook Lift", depth: 2, white_to_move: true,
      board: "......k......ppp............R...............................K...",
      sol: (3,4,0,4), hint: "Rook lifts to e8!" },
    Puzzle { name: "Q Sacrifice", depth: 1, white_to_move: true,
      board: ".....rk......ppp............................................Q.K.",
      sol: (7,4,0,4), hint: "Queen to e8!" },
    Puzzle { name: "Pin Win", depth: 2, white_to_move: true,
      board: "....k.......p..............Q..................................K.",
      sol: (3,3,0,3), hint: "Queen to d8!" },
    Puzzle { name: "Fork Attack", depth: 2, white_to_move: true,
      board: "r...k..r....p...............N...............................K...",
      sol: (3,4,2,2), hint: "Knight to c6, forking!" },
    Puzzle { name: "Back Rank II", depth: 1, white_to_move: true,
      board: ".....rk......ppp............................................R.K.",
      sol: (7,4,0,4), hint: "Rook to e8!" },
    Puzzle { name: "Queens Coup", depth: 1, white_to_move: true,
      board: ".......k......p..........................Q..................K...",
      sol: (5,1,0,6), hint: "Queen slices to g8!" },
    Puzzle { name: "Rook Queen", depth: 1, white_to_move: true,
      board: ".......k......pp............Q...............................K..R",
      sol: (3,4,0,4), hint: "Queen to e8!" },
    Puzzle { name: "Final Boss", depth: 2, white_to_move: true,
      board: "r...k..r.pppppppp...............................PPPPPPPPRNBQKBNR",
      sol: (7,3,1,3), hint: "Queen to d7!" },
];

fn is_white(p: u8) -> bool { p.is_ascii_uppercase() && p != b'.' }
fn is_black(p: u8) -> bool { p.is_ascii_lowercase() }

fn board_arr(puzzle: &Puzzle) -> [u8; 64] {
    let mut arr = [b'.'; 64];
    for (i, &b) in puzzle.board.as_bytes().iter().take(64).enumerate() {
        arr[i] = b;
    }
    arr
}

#[derive(PartialEq)]
enum State { Play, Solved, Wrong }

struct App {
    current:    usize,
    board:      [u8; 64],
    cursor:     (usize, usize),
    selected:   Option<(usize, usize)>,
    state:      State,
    solved_cnt: u32,
}

impl App {
    fn new() -> Self {
        App { current: 0, board: board_arr(&PUZZLES[0]),
              cursor: (4,4), selected: None, state: State::Play, solved_cnt: 0 }
    }

    fn load(&mut self) {
        self.board = board_arr(&PUZZLES[self.current]);
        self.cursor = (4,4);
        self.selected = None;
        self.state = State::Play;
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 30, C_HEADER);
        let p = &PUZZLES[self.current];
        text(12, 8, C_TEXT, &format!("Chess Puzzles  [{}/{}]  {}", self.current+1, PUZZLES.len(), p.name));
        let ds = if p.depth==1 {"Mate-in-1 [*]"} else {"Mate-in-2 [**]"};
        let ss = if p.white_to_move {"White"} else {"Black"};
        text(W-240, 8, C_ORANGE, &format!("{}  {} to move", ds, ss));

        for row in 0..8usize {
            for col in 0..8usize {
                let sx = BX + col as i32 * SQ;
                let sy = BY + row as i32 * SQ;
                let light = (row+col) % 2 == 0;
                let sq_c = if self.cursor == (row,col) {
                    0xFFA65799u32
                } else if self.selected == Some((row,col)) {
                    0x58A6FF99u32
                } else if light { C_LIGHT } else { C_DARK };
                fill(sx, sy, SQ, SQ, sq_c);

                let pc = self.board[row*8+col];
                if pc != b'.' {
                    let ps = (pc as char).to_string();
                    let pc_col = if is_white(pc) { 0xF0F0F0FFu32 } else { 0xDD4444FFu32 };
                    text(sx+SQ/2-4, sy+SQ/2-8, pc_col, &ps);
                }
                if self.cursor == (row,col) { border(sx,sy,SQ,SQ,C_ORANGE); }
                else if self.selected == Some((row,col)) { border(sx,sy,SQ,SQ,C_SEL); }
            }
        }

        for col in 0..8usize {
            let fc = (b'a' + col as u8) as char;
            text(BX + col as i32*SQ + SQ/2-4, BY+8*SQ+4, C_HINT, &fc.to_string());
        }
        for row in 0..8usize {
            text(BX-14, BY+row as i32*SQ+SQ/2-8, C_HINT, &(8-row).to_string());
        }

        let px = BX+8*SQ+16;
        let pw = W-px-8;
        fill(px, BY, pw, 8*SQ, C_CARD);
        border(px, BY, pw, 8*SQ, C_BORDER);
        text(px+8, BY+8,  C_HINT,   "PUZZLE");
        text(px+8, BY+24, C_ORANGE, p.name);
        text(px+8, BY+44, C_YELLOW, ds);
        text(px+8, BY+60, C_SEL,    &format!("{} to move", ss));
        text(px+8, BY+88, C_HINT,   "HINT:");
        let h = p.hint;
        let mid = if h.len()>18 { h[..18].rfind(' ').unwrap_or(18) } else { h.len() };
        text(px+8, BY+104, C_TEXT, &h[..mid]);
        if mid < h.len() { text(px+8, BY+120, C_TEXT, &h[mid+1..]); }
        text(px+8, BY+148, C_HINT, &format!("Solved: {}", self.solved_cnt));
        text(px+8, BY+176, C_HINT, "Arrows:move");
        text(px+8, BY+192, C_HINT, "Enter:select");
        text(px+8, BY+208, C_HINT, "R:reset");
        text(px+8, BY+224, C_HINT, "<>:puzzles");

        match self.state {
            State::Solved => {
                fill(55, 350, 400, 70, C_CARD);
                border(55, 350, 400, 70, C_GREEN);
                text(115, 368, C_GREEN, "CORRECT! Excellent!");
                text(95, 392, C_HINT, "Press > for the next puzzle");
            }
            State::Wrong => {
                fill(55, 350, 400, 70, C_CARD);
                border(55, 350, 400, 70, C_RED);
                text(135, 368, C_RED, "Incorrect move.");
                text(95, 392, C_HINT, "Press R to reset and retry.");
            }
            State::Play => {}
        }

        let mode = if self.selected.is_some() {"SELECT DEST"} else {"SELECT PIECE"};
        text(12, H-18, C_HINT, &format!("{}  |  <>=puzzles  R=reset", mode));
        flush();
    }

    fn try_move(&mut self, tr: usize, tc: usize) {
        if let Some((fr,fc)) = self.selected {
            let sol = PUZZLES[self.current].sol;
            if (fr,fc,tr,tc) == sol {
                let fi = fr*8+fc;
                let ti = tr*8+tc;
                self.board[ti] = self.board[fi];
                self.board[fi] = b'.';
                self.state = State::Solved;
                self.solved_cnt += 1;
            } else {
                self.state = State::Wrong;
            }
            self.selected = None;
        }
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[A" => { if self.cursor.0>0 { self.cursor.0-=1; } self.draw(); }
            "\x1b[B" => { if self.cursor.0<7 { self.cursor.0+=1; } self.draw(); }
            "\x1b[D" => {
                if self.selected.is_none() {
                    if self.current>0 { self.current-=1; self.load(); }
                } else if self.cursor.1>0 { self.cursor.1-=1; }
                self.draw();
            }
            "\x1b[C" => {
                if self.selected.is_none() {
                    if self.current+1<PUZZLES.len() { self.current+=1; self.load(); }
                } else if self.cursor.1<7 { self.cursor.1+=1; }
                self.draw();
            }
            "" => {
                if self.state!=State::Play { return; }
                let (row,col) = self.cursor;
                if self.selected.is_some() {
                    self.try_move(row,col);
                } else {
                    let pc = self.board[row*8+col];
                    let p = &PUZZLES[self.current];
                    let valid = if p.white_to_move { is_white(pc) } else { is_black(pc) };
                    if valid { self.selected = Some((row,col)); }
                }
                self.draw();
            }
            "r"|"R" => { self.load(); self.draw(); }
            "q"|"Q"|"\x03" => std::process::exit(0),
            _ => {}
        }
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    let _ = io::stdout().flush();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l)=>l, Err(_)=>break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
