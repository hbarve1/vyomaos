use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GY: i32 = 32;
const COLS: usize = 160;
const ROWS: usize = 112;
const CELL: i32 = 6;

const C_EMPTY:  u32 = 0x0D1117FF;
const C_SAND:   u32 = 0xE2B96FFF;
const C_WATER:  u32 = 0x2B65ECFF;
const C_STONE:  u32 = 0x8B949EFF;
const C_FIRE:   u32 = 0xFF6B35FF;
const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_SEL:    u32 = 0x58A6FFFF;

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

#[derive(Clone, Copy, PartialEq)]
enum Mat { Empty, Sand, Water, Stone, Fire }

impl Mat {
    fn color(self) -> u32 {
        match self {
            Mat::Empty => C_EMPTY,
            Mat::Sand  => C_SAND,
            Mat::Water => C_WATER,
            Mat::Stone => C_STONE,
            Mat::Fire  => C_FIRE,
        }
    }
    fn name(self) -> &'static str {
        match self { Mat::Empty=>"Empty", Mat::Sand=>"Sand", Mat::Water=>"Water", Mat::Stone=>"Stone", Mat::Fire=>"Fire" }
    }
}

#[derive(Clone, Copy)]
struct Cell { mat: Mat, age: u8 }

impl Cell {
    fn new(mat: Mat) -> Self { Cell { mat, age: 0 } }
    fn empty() -> Self { Cell { mat: Mat::Empty, age: 0 } }
}

fn rng(s: &mut u64) -> u64 {
    *s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    *s >> 33
}

struct App {
    grid:  Vec<Cell>,  // ROWS * COLS flat
    cur_r: usize,
    cur_c: usize,
    sel:   Mat,
    hold:  bool,
    seed:  u64,
    tick:  u64,
}

impl App {
    fn new() -> Self {
        App {
            grid:  vec![Cell::empty(); ROWS * COLS],
            cur_r: ROWS / 2,
            cur_c: COLS / 2,
            sel:   Mat::Sand,
            hold:  false,
            seed:  0xDEADBEEF,
            tick:  0,
        }
    }

    fn get(&self, r: usize, c: usize) -> Cell { self.grid[r * COLS + c] }
    fn set(&mut self, r: usize, c: usize, cell: Cell) { self.grid[r * COLS + c] = cell; }

    fn paint(&mut self) {
        let r = self.cur_r as i32;
        let c = self.cur_c as i32;
        let sel = self.sel;
        for dr in -1i32..=1 {
            for dc in -1i32..=1 {
                let nr = r + dr;
                let nc = c + dc;
                if nr >= 0 && nr < ROWS as i32 && nc >= 0 && nc < COLS as i32 {
                    self.set(nr as usize, nc as usize, Cell::new(sel));
                }
            }
        }
    }

    fn step(&mut self) {
        let mut moved = vec![false; ROWS * COLS];
        let left_to_right = self.tick % 2 == 0;

        for r in (0..ROWS).rev() {
            for ci in 0..COLS {
                let c = if left_to_right { ci } else { COLS - 1 - ci };
                if moved[r * COLS + c] { continue; }
                let cell = self.get(r, c);
                match cell.mat {
                    Mat::Sand => {
                        if r + 1 < ROWS && self.get(r+1, c).mat == Mat::Empty && !moved[(r+1)*COLS+c] {
                            self.set(r+1, c, cell);
                            self.set(r, c, Cell::empty());
                            moved[(r+1)*COLS+c] = true;
                        } else {
                            let can_l = c > 0 && r+1 < ROWS
                                && self.get(r+1,c-1).mat == Mat::Empty && !moved[(r+1)*COLS+c-1];
                            let can_r = c+1 < COLS && r+1 < ROWS
                                && self.get(r+1,c+1).mat == Mat::Empty && !moved[(r+1)*COLS+c+1];
                            let target = match (can_l, can_r) {
                                (true, true)   => if rng(&mut self.seed)%2==0 { Some(c-1) } else { Some(c+1) },
                                (true, false)  => Some(c-1),
                                (false, true)  => Some(c+1),
                                (false, false) => None,
                            };
                            if let Some(tc) = target {
                                self.set(r+1, tc, cell);
                                self.set(r, c, Cell::empty());
                                moved[(r+1)*COLS+tc] = true;
                            }
                        }
                    }
                    Mat::Water => {
                        if r+1 < ROWS && self.get(r+1,c).mat == Mat::Empty && !moved[(r+1)*COLS+c] {
                            self.set(r+1, c, cell);
                            self.set(r, c, Cell::empty());
                            moved[(r+1)*COLS+c] = true;
                        } else {
                            let can_l = c > 0 && self.get(r,c-1).mat == Mat::Empty && !moved[r*COLS+c-1];
                            let can_r = c+1 < COLS && self.get(r,c+1).mat == Mat::Empty && !moved[r*COLS+c+1];
                            let target = match (can_l, can_r) {
                                (true, true)   => if rng(&mut self.seed)%2==0 { Some(c-1) } else { Some(c+1) },
                                (true, false)  => Some(c-1),
                                (false, true)  => Some(c+1),
                                (false, false) => None,
                            };
                            if let Some(tc) = target {
                                self.set(r, tc, cell);
                                self.set(r, c, Cell::empty());
                                moved[r*COLS+tc] = true;
                            }
                        }
                    }
                    Mat::Fire => {
                        let new_age = cell.age.saturating_add(1);
                        if new_age >= 8 {
                            self.set(r, c, Cell::empty());
                        } else {
                            self.grid[r*COLS+c].age = new_age;
                            for (nr, nc) in [
                                (r.wrapping_sub(1), c), (r+1, c),
                                (r, c.wrapping_sub(1)), (r, c+1),
                            ] {
                                if nr < ROWS && nc < COLS && self.get(nr,nc).mat == Mat::Empty {
                                    if rng(&mut self.seed) % 20 == 0 {
                                        self.set(nr, nc, Cell::new(Mat::Fire));
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        self.tick += 1;
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, GY, C_HEADER);
        fill(0, H - 28, W, 28, C_HEADER);

        text(12, 8, C_TEXT, "Sand Simulation");
        text(152, 8, C_HINT, &format!(
            "1=Empty  2=Sand  3=Water  4=Stone  5=Fire  Space=paint  H=hold({})  C=clear  Q=quit",
            if self.hold { "ON" } else { "off" }
        ));

        // Draw grid with RLE per row
        for r in 0..ROWS {
            let y = GY + r as i32 * CELL;
            let mut c = 0;
            while c < COLS {
                let mat = self.get(r, c).mat;
                let color = mat.color();
                let mut end = c + 1;
                while end < COLS && self.get(r, end).mat == mat { end += 1; }
                fill(c as i32 * CELL, y, (end - c) as i32 * CELL, CELL, color);
                c = end;
            }
        }

        // Cursor outline (3×3 brush area)
        let cr = self.cur_r as i32;
        let cc = self.cur_c as i32;
        let x0 = ((cc - 1).max(0)) * CELL;
        let y0 = GY + ((cr - 1).max(0)) * CELL;
        let x1 = ((cc + 2).min(COLS as i32)) * CELL;
        let y1 = GY + ((cr + 2).min(ROWS as i32)) * CELL;
        fill(x0, y0, x1 - x0, 1, C_SEL);
        fill(x0, y1 - 1, x1 - x0, 1, C_SEL);
        fill(x0, y0, 1, y1 - y0, C_SEL);
        fill(x1 - 1, y0, 1, y1 - y0, C_SEL);

        // Material palette in status bar
        let mats = [Mat::Empty, Mat::Sand, Mat::Water, Mat::Stone, Mat::Fire];
        let key_labels = ["1", "2", "3", "4", "5"];
        for (i, &m) in mats.iter().enumerate() {
            let bx = 12 + i as i32 * 110;
            let by = H - 24;
            let selected = m == self.sel;
            if selected { fill(bx - 2, by - 2, 108, 24, C_SEL); }
            fill(bx, by, 16, 16, m.color());
            let tc = if selected { 0x0D1117FF } else { C_TEXT };
            text(bx + 20, by + 2, tc, &format!("{} {}", key_labels[i], m.name()));
        }
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                self.step();
                if self.hold { self.paint(); }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " "          => { self.paint(); }
            "h" | "H"    => { self.hold = !self.hold; }
            "c" | "C"    => { for cell in self.grid.iter_mut() { *cell = Cell::empty(); } }
            "1"          => { self.sel = Mat::Empty; }
            "2"          => { self.sel = Mat::Sand; }
            "3"          => { self.sel = Mat::Water; }
            "4"          => { self.sel = Mat::Stone; }
            "5"          => { self.sel = Mat::Fire; }
            "\x1b[A"     => { if self.cur_r > 0 { self.cur_r -= 1; } }
            "\x1b[B"     => { if self.cur_r + 1 < ROWS { self.cur_r += 1; } }
            "\x1b[C"     => { if self.cur_c + 1 < COLS { self.cur_c += 1; } }
            "\x1b[D"     => { if self.cur_c > 0 { self.cur_c -= 1; } }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
