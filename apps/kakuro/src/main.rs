// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const CELL_W: i32 = 72;
const CELL_H: i32 = 72;
const OX: i32 = 48;
const OY: i32 = 48;
const GRID: usize = 8;

const C_BG:       u32 = 0x0D1117FF;
const C_HEADER:   u32 = 0x21262DFF;
const C_BLOCKED:  u32 = 0x161B22FF;
const C_CLUE:     u32 = 0x21262DFF;
const C_ENTRY:    u32 = 0xD0D7DDFF;
const C_ENTRY_TXT:u32 = 0x0D1117FF;
const C_CLUE_TXT: u32 = 0xE6EDF3FF;
const C_BORDER:   u32 = 0x30363DFF;
const C_TEXT:     u32 = 0xE6EDF3FF;
const C_HINT:     u32 = 0x6E7681FF;
const C_SEL:      u32 = 0x58A6FFFF;
const C_ERROR:    u32 = 0xFF4444FF;
const C_CORRECT:  u32 = 0x3FB950FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

#[derive(Clone, Copy)]
enum CK { Blocked, Clue(u8, u8), Entry }  // Clue(h_sum, v_sum), 0 = no clue that dir

#[derive(Clone)]
struct Puzzle {
    cells: [[CK; GRID]; GRID],
    sol:   [[u8;  GRID]; GRID],
}

fn puzzle1() -> Puzzle {
    // 3 rows × 2 cols of entries; V1=6,V2=7; H1=3,H2=4,H3=6
    // Solution: (r,c) -> digit
    //   (1,1)=1,(1,2)=2; (2,1)=3,(2,2)=1; (3,1)=2,(3,2)=4
    use CK::*;
    let mut c = [[Blocked; GRID]; GRID];
    c[0][1] = Clue(0, 6);
    c[0][2] = Clue(0, 7);
    c[1][0] = Clue(3, 0); c[1][1] = Entry; c[1][2] = Entry;
    c[2][0] = Clue(4, 0); c[2][1] = Entry; c[2][2] = Entry;
    c[3][0] = Clue(6, 0); c[3][1] = Entry; c[3][2] = Entry;
    let mut s = [[0u8; GRID]; GRID];
    s[1][1]=1; s[1][2]=2; s[2][1]=3; s[2][2]=1; s[3][1]=2; s[3][2]=4;
    Puzzle { cells: c, sol: s }
}

fn puzzle2() -> Puzzle {
    // 3 rows × 3 cols of entries; V1=9,V2=8,V3=10; H1=10,H2=11,H3=6
    // Solution:
    //   (1,1)=2,(1,2)=5,(1,3)=3; (2,1)=4,(2,2)=1,(2,3)=6; (3,1)=3,(3,2)=2,(3,3)=1
    use CK::*;
    let mut c = [[Blocked; GRID]; GRID];
    c[0][1] = Clue(0, 9); c[0][2] = Clue(0, 8); c[0][3] = Clue(0, 10);
    c[1][0] = Clue(10,0); c[1][1]=Entry; c[1][2]=Entry; c[1][3]=Entry;
    c[2][0] = Clue(11,0); c[2][1]=Entry; c[2][2]=Entry; c[2][3]=Entry;
    c[3][0] = Clue(6, 0); c[3][1]=Entry; c[3][2]=Entry; c[3][3]=Entry;
    let mut s = [[0u8; GRID]; GRID];
    s[1][1]=2; s[1][2]=5; s[1][3]=3;
    s[2][1]=4; s[2][2]=1; s[2][3]=6;
    s[3][1]=3; s[3][2]=2; s[3][3]=1;
    Puzzle { cells: c, sol: s }
}

fn puzzle3() -> Puzzle {
    // Two separate blocks
    // Left: rows 1-2, cols 1-2; V1=10,V2=6; H1=13,H2=3
    //   (1,1)=9,(1,2)=4; (2,1)=1,(2,2)=2
    // Right: rows 1-3, cols 4-5; V3=7,V4=8; H3=4,H4=5,H5=6
    //   (1,4)=1,(1,5)=3; (2,4)=4,(2,5)=1; (3,4)=2,(3,5)=4
    use CK::*;
    let mut c = [[Blocked; GRID]; GRID];
    // Left block
    c[0][1] = Clue(0,10); c[0][2] = Clue(0,6);
    c[1][0] = Clue(13,0); c[1][1]=Entry; c[1][2]=Entry;
    c[2][0] = Clue(3, 0); c[2][1]=Entry; c[2][2]=Entry;
    // Right block
    c[0][4] = Clue(0,7); c[0][5] = Clue(0,8);
    c[1][3] = Clue(4,0); c[1][4]=Entry; c[1][5]=Entry;
    c[2][3] = Clue(5,0); c[2][4]=Entry; c[2][5]=Entry;
    c[3][3] = Clue(6,0); c[3][4]=Entry; c[3][5]=Entry;
    let mut s = [[0u8; GRID]; GRID];
    s[1][1]=9; s[1][2]=4; s[2][1]=1; s[2][2]=2;
    s[1][4]=1; s[1][5]=3; s[2][4]=4; s[2][5]=1; s[3][4]=2; s[3][5]=4;
    Puzzle { cells: c, sol: s }
}

fn check_errors(p: &Puzzle, vals: &[[u8; GRID]; GRID]) -> [[bool; GRID]; GRID] {
    let mut err = [[false; GRID]; GRID];

    // Horizontal runs
    for r in 0..GRID {
        let mut c = 0;
        while c < GRID {
            if let CK::Clue(h, _) = p.cells[r][c] {
                if h > 0 {
                    let mut run: Vec<(usize, usize)> = Vec::new();
                    let mut nc = c + 1;
                    while nc < GRID { if let CK::Entry=p.cells[r][nc] { run.push((r,nc)); nc+=1; } else { break; } }
                    let mut seen = 0u16;
                    let mut dup = false;
                    let mut sum = 0u32;
                    for &(er, ec) in &run {
                        let v = vals[er][ec];
                        if v > 0 {
                            sum += v as u32;
                            if seen & (1<<v) != 0 { dup = true; }
                            seen |= 1 << v;
                        }
                    }
                    let all = run.iter().all(|&(er,ec)| vals[er][ec] > 0);
                    if dup || (all && sum != h as u32) {
                        for &(er,ec) in &run { if vals[er][ec] > 0 { err[er][ec] = true; } }
                    }
                }
            }
            c += 1;
        }
    }

    // Vertical runs
    for c in 0..GRID {
        let mut r = 0;
        while r < GRID {
            if let CK::Clue(_, v) = p.cells[r][c] {
                if v > 0 {
                    let mut run: Vec<(usize, usize)> = Vec::new();
                    let mut nr = r + 1;
                    while nr < GRID { if let CK::Entry=p.cells[nr][c] { run.push((nr,c)); nr+=1; } else { break; } }
                    let mut seen = 0u16;
                    let mut dup = false;
                    let mut sum = 0u32;
                    for &(er,ec) in &run {
                        let dg = vals[er][ec];
                        if dg > 0 {
                            sum += dg as u32;
                            if seen & (1<<dg) != 0 { dup = true; }
                            seen |= 1 << dg;
                        }
                    }
                    let all = run.iter().all(|&(er,ec)| vals[er][ec] > 0);
                    if dup || (all && sum != v as u32) {
                        for &(er,ec) in &run { if vals[er][ec] > 0 { err[er][ec] = true; } }
                    }
                }
            }
            r += 1;
        }
    }
    err
}

fn is_solved(p: &Puzzle, vals: &[[u8; GRID]; GRID]) -> bool {
    for r in 0..GRID {
        for c in 0..GRID {
            if let CK::Entry = p.cells[r][c] {
                if vals[r][c] != p.sol[r][c] { return false; }
            }
        }
    }
    true
}

struct App {
    puzzles: [Puzzle; 3],
    idx:     usize,
    vals:    [[u8; GRID]; GRID],
    cur_r:   usize,
    cur_c:   usize,
    errors:  [[bool; GRID]; GRID],
    checked: bool,
    solved:  bool,
}

impl App {
    fn new() -> Self {
        let puzzles = [puzzle1(), puzzle2(), puzzle3()];
        let (cur_r, cur_c) = Self::first_entry_in(&puzzles[0]);
        App { puzzles, idx: 0, vals: [[0; GRID]; GRID],
              cur_r, cur_c, errors: [[false; GRID]; GRID], checked: false, solved: false }
    }

    fn first_entry_in(p: &Puzzle) -> (usize, usize) {
        for r in 0..GRID { for c in 0..GRID { if let CK::Entry = p.cells[r][c] { return (r,c); } } }
        (0, 0)
    }

    fn puzzle(&self) -> &Puzzle { &self.puzzles[self.idx] }

    fn move_cursor(&mut self, dr: i32, dc: i32) {
        let mut r = self.cur_r as i32;
        let mut c = self.cur_c as i32;
        loop {
            r += dr; c += dc;
            if r < 0 || r >= GRID as i32 || c < 0 || c >= GRID as i32 { return; }
            if let CK::Entry = self.puzzle().cells[r as usize][c as usize] {
                self.cur_r = r as usize;
                self.cur_c = c as usize;
                return;
            }
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        fill(0, H - 28, W, 28, C_HEADER);

        text(12, 8, C_TEXT, &format!("Kakuro — Puzzle {}/3", self.idx + 1));
        text(180, 8, C_HINT, "Arrow=move  1-9=fill  Bksp=clear  C=check  N=next  Q=quit");

        let p = self.puzzle();

        // Draw grid cells
        for r in 0..GRID {
            for c in 0..GRID {
                let x = OX + c as i32 * CELL_W;
                let y = OY + r as i32 * CELL_H;
                match p.cells[r][c] {
                    CK::Blocked => {
                        fill(x, y, CELL_W, CELL_H, C_BLOCKED);
                    }
                    CK::Clue(h, v) => {
                        fill(x, y, CELL_W, CELL_H, C_CLUE);
                        // Dividing diagonal via fill
                        fill(x + CELL_W/2, y, CELL_W/2, CELL_H/2, 0x30363DFF);
                        fill(x, y + CELL_H/2, CELL_W/2, CELL_H/2, 0x30363DFF);
                        if h > 0 { text(x + CELL_W/2 + 4, y + 4, C_CLUE_TXT, &h.to_string()); }
                        if v > 0 { text(x + 4, y + CELL_H/2 + 4, C_CLUE_TXT, &v.to_string()); }
                    }
                    CK::Entry => {
                        let is_cur = r == self.cur_r && c == self.cur_c;
                        let is_err = self.checked && self.errors[r][c];
                        let bg = if is_err { C_ERROR } else if is_cur { C_SEL } else { C_ENTRY };
                        fill(x, y, CELL_W, CELL_H, bg);
                        let v = self.vals[r][c];
                        if v > 0 {
                            let tc = if is_cur { 0x0D1117FF } else if is_err { C_ENTRY } else { C_ENTRY_TXT };
                            text(x + CELL_W/2 - 4, y + CELL_H/2 - 8, tc, &v.to_string());
                        }
                    }
                }
                // Cell border
                fill(x, y, CELL_W, 1, C_BORDER);
                fill(x, y, 1, CELL_H, C_BORDER);
            }
        }
        // Right/bottom border
        fill(OX + GRID as i32 * CELL_W, OY, 1, GRID as i32 * CELL_H, C_BORDER);
        fill(OX, OY + GRID as i32 * CELL_H, GRID as i32 * CELL_W + 1, 1, C_BORDER);

        // Status footer
        let status = if self.solved {
            "Solved!".to_string()
        } else if self.checked {
            let n_err: usize = self.errors.iter().flat_map(|row| row.iter()).filter(|&&e| e).count();
            if n_err == 0 { "No errors found".to_string() } else { format!("{} error(s) highlighted", n_err) }
        } else {
            "Press C to check".to_string()
        };
        let sc = if self.solved { C_CORRECT } else if self.checked { C_HINT } else { C_HINT };
        text(12, H - 20, sc, &status);
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[A" => self.move_cursor(-1, 0),
            "\x1b[B" => self.move_cursor(1, 0),
            "\x1b[C" => self.move_cursor(0, 1),
            "\x1b[D" => self.move_cursor(0, -1),
            "\x7f"   => {
                if let CK::Entry = self.puzzle().cells[self.cur_r][self.cur_c] {
                    self.vals[self.cur_r][self.cur_c] = 0;
                    self.checked = false;
                }
            }
            "c" | "C" => {
                self.errors = check_errors(self.puzzle(), &self.vals);
                self.checked = true;
                self.solved = is_solved(self.puzzle(), &self.vals);
            }
            "n" | "N" => {
                self.idx = (self.idx + 1) % 3;
                self.vals = [[0; GRID]; GRID];
                self.errors = [[false; GRID]; GRID];
                self.checked = false;
                self.solved = false;
                let (r, c) = Self::first_entry_in(self.puzzle());
                self.cur_r = r; self.cur_c = c;
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            s if s.len() == 1 => {
                if let Some(d) = s.chars().next().and_then(|ch| ch.to_digit(10)) {
                    if d >= 1 && d <= 9 {
                        if let CK::Entry = self.puzzle().cells[self.cur_r][self.cur_c] {
                            self.vals[self.cur_r][self.cur_c] = d as u8;
                            self.checked = false;
                        }
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
