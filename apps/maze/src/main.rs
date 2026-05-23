use std::io::{self, BufRead, Write};

const W: u32 = 840;
const H: u32 = 760;
const HEADER_H: u32 = 48;

const COLS: usize = 25;
const ROWS: usize = 25;
const CELL: u32 = 28;
const MAZE_X: u32 = (W - COLS as u32 * CELL) / 2;
const MAZE_Y: u32 = 64;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_WALL: u32   = 0x8B949EFF;
const C_TRAIL: u32  = 0x1A2A3AFF;
const C_PLAYER: u32 = 0x58A6FFFF;
const C_END: u32    = 0x3FB950FF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

// walls stored as 4 bools per cell: N=0 E=1 S=2 W=3
// flat index: r * COLS + c
struct Maze {
    walls:   Vec<[bool; 4]>, // ROWS*COLS entries
    visited: Vec<bool>,
    trail:   Vec<bool>,
    px:      usize,
    py:      usize,
    solved:  bool,
    seed:    u64,
    steps:   u32,
}

impl Maze {
    fn new(seed: u64) -> Self {
        let n = ROWS * COLS;
        let mut m = Maze {
            walls:   vec![[true; 4]; n],
            visited: vec![false; n],
            trail:   vec![false; n],
            px: 0,
            py: 0,
            solved: false,
            seed,
            steps: 0,
        };
        m.generate();
        m.trail[0] = true;
        m
    }

    fn idx(r: usize, c: usize) -> usize { r * COLS + c }

    fn generate(&mut self) {
        let mut stack: Vec<(usize, usize)> = Vec::with_capacity(ROWS * COLS);
        self.visited[0] = true;
        stack.push((0, 0));

        while let Some(&(r, c)) = stack.last() {
            let mut nb = [(0usize, 0usize, 0usize); 4];
            let mut nn = 0;
            if r > 0        && !self.visited[Self::idx(r-1, c)] { nb[nn] = (r-1, c, 0); nn += 1; }
            if c+1 < COLS   && !self.visited[Self::idx(r, c+1)] { nb[nn] = (r, c+1, 1); nn += 1; }
            if r+1 < ROWS   && !self.visited[Self::idx(r+1, c)] { nb[nn] = (r+1, c, 2); nn += 1; }
            if c > 0        && !self.visited[Self::idx(r, c-1)] { nb[nn] = (r, c-1, 3); nn += 1; }

            if nn == 0 {
                stack.pop();
            } else {
                self.seed = lcg(self.seed);
                let (nr, nc, dir) = nb[(self.seed >> 33) as usize % nn];
                self.walls[Self::idx(r, c)][dir] = false;
                let opp = [2usize, 3, 0, 1][dir];
                self.walls[Self::idx(nr, nc)][opp] = false;
                self.visited[Self::idx(nr, nc)] = true;
                stack.push((nr, nc));
            }
        }
    }

    fn try_move(&mut self, dr: i32, dc: i32) {
        if self.solved { return; }
        let dir = match (dr, dc) { (-1,0)=>0, (0,1)=>1, (1,0)=>2, (0,-1)=>3, _=>return };
        if self.walls[Self::idx(self.py, self.px)][dir] { return; }
        let nr = (self.py as i32 + dr) as usize;
        let nc = (self.px as i32 + dc) as usize;
        self.py = nr; self.px = nc;
        self.trail[Self::idx(nr, nc)] = true;
        self.steps += 1;
        if nr == ROWS-1 && nc == COLS-1 { self.solved = true; }
    }

    fn reset(&mut self) {
        self.px = 0; self.py = 0;
        self.trail = vec![false; ROWS*COLS];
        self.trail[0] = true;
        self.solved = false; self.steps = 0;
    }
}

fn draw(m: &Maze) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H-1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Maze");
    text(100, 16, C_HINT, &format!("Steps: {}  25×25", m.steps));
    text(380, 16, C_HINT, "Arrows:move  R:reset  N:new maze");

    fill(MAZE_X-1, MAZE_Y-1, COLS as u32*CELL+2, ROWS as u32*CELL+2, C_WALL);
    fill(MAZE_X,   MAZE_Y,   COLS as u32*CELL,   ROWS as u32*CELL,   C_BG);

    for r in 0..ROWS {
        for c in 0..COLS {
            let cx = MAZE_X + c as u32 * CELL;
            let cy = MAZE_Y + r as u32 * CELL;
            let i  = Maze::idx(r, c);

            if m.trail[i] { fill(cx+1, cy+1, CELL-2, CELL-2, C_TRAIL); }

            let w = &m.walls[i];
            if w[0] { fill(cx, cy,          CELL, 2,    C_WALL); }
            if w[1] { fill(cx+CELL-2, cy,   2,    CELL, C_WALL); }
            if w[2] { fill(cx, cy+CELL-2,   CELL, 2,    C_WALL); }
            if w[3] { fill(cx, cy,          2,    CELL, C_WALL); }
        }
    }

    // End
    let ex = MAZE_X + (COLS-1) as u32 * CELL;
    let ey = MAZE_Y + (ROWS-1) as u32 * CELL;
    fill(ex+4, ey+4, CELL-8, CELL-8, C_END);

    // Player
    fill(MAZE_X + m.px as u32*CELL+4, MAZE_Y + m.py as u32*CELL+4, CELL-8, CELL-8, C_PLAYER);

    if m.solved {
        let bx = (W-280)/2;
        let by = (H-80)/2;
        fill(bx, by, 280, 80, C_HEADER);
        border(bx, by, 280, 80, C_GREEN);
        text(bx+60, by+16, C_GREEN, "Maze Solved!");
        text(bx+28, by+44, C_HINT, &format!("{} steps  N=new maze", m.steps));
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut seed: u64 = 0xABCE1234ABCD5678;
    let mut maze = Maze::new(seed);

    println!("@supervisor: raise maze");
    let _ = io::stdout().flush();
    draw(&maze);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => { maze.try_move(-1, 0); }
            "\x1b[B" => { maze.try_move(1, 0); }
            "\x1b[C" => { maze.try_move(0, 1); }
            "\x1b[D" => { maze.try_move(0, -1); }
            "r" | "R" => { maze.reset(); }
            "n" | "N" => { seed = lcg(seed); maze = Maze::new(seed); }
            _ => {}
        }
        draw(&maze);
    }
}
