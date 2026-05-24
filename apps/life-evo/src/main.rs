use std::io::{self, BufRead, Write};

const W: i32  = 960;
const H: i32  = 720;
const GY: i32 = 24;
const GW: usize = 120;
const GH: usize = 80;
const CELL: i32 = 8;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;

const RULE_NAMES:   [&str; 3] = ["Conway B3/S23", "HighLife B36/S23", "Day&Night B3678/S34678"];
const RULE_COLORS:  [u32; 3]  = [0x3FB950FF, 0xFFA657FF, 0x58A6FFFF];
const BORN:    [&[u8]; 3] = [&[3], &[3, 6], &[3, 6, 7, 8]];
const SURVIVE: [&[u8]; 3] = [&[2, 3], &[2, 3], &[3, 4, 6, 7, 8]];

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn count_neighbors(grid: &[bool], x: usize, y: usize) -> u8 {
    let mut n = 0u8;
    for dy in -1i32..=1 {
        for dx in -1i32..=1 {
            if dx == 0 && dy == 0 { continue; }
            let nx = ((x as i32 + dx + GW as i32) % GW as i32) as usize;
            let ny = ((y as i32 + dy + GH as i32) % GH as i32) as usize;
            if grid[ny * GW + nx] { n += 1; }
        }
    }
    n
}

struct App {
    cur:     Vec<bool>,
    next:    Vec<bool>,
    rule:    usize,
    running: bool,
    gen:     u64,
    rng:     u64,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            cur:     vec![false; GW * GH],
            next:    vec![false; GW * GH],
            rule:    0,
            running: false,
            gen:     0,
            rng:     0xCAFEBABE_DEADBEEF,
        };
        app.random_fill();
        app
    }

    fn random_fill(&mut self) {
        let mut rng = self.rng;
        for cell in self.cur.iter_mut() {
            rng = lcg(rng);
            *cell = (rng >> 33) % 3 == 0;
        }
        self.rng = lcg(rng);
        self.gen = 0;
    }

    fn step(&mut self) {
        let born = BORN[self.rule];
        let survive = SURVIVE[self.rule];
        for y in 0..GH {
            for x in 0..GW {
                let n = count_neighbors(&self.cur, x, y);
                let alive = self.cur[y * GW + x];
                self.next[y * GW + x] = if alive {
                    survive.contains(&n)
                } else {
                    born.contains(&n)
                };
            }
        }
        std::mem::swap(&mut self.cur, &mut self.next);
        self.gen += 1;
    }

    fn live_count(&self) -> usize {
        self.cur.iter().filter(|&&c| c).count()
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        let status = format!("{}  |  Gen: {}  |  Live: {}  |  Space=run/pause  P=rule  S=step  R=random  C=clear  Q=quit",
            RULE_NAMES[self.rule], self.gen, self.live_count());
        text(8, 4, C_TEXT, &status);

        fill(0, GY, W, H - GY, C_BG);

        let color = RULE_COLORS[self.rule];
        for y in 0..GH {
            let mut run_start: Option<usize> = None;
            for x in 0..GW {
                let alive = self.cur[y * GW + x];
                match (alive, run_start) {
                    (true, None) => { run_start = Some(x); }
                    (false, Some(s)) => {
                        fill(s as i32 * CELL, GY + y as i32 * CELL,
                             (x - s) as i32 * CELL, CELL, color);
                        run_start = None;
                    }
                    _ => {}
                }
            }
            if let Some(s) = run_start {
                fill(s as i32 * CELL, GY + y as i32 * CELL,
                     (GW - s) as i32 * CELL, CELL, color);
            }
        }

        if self.running {
            text(8, GY + 4, C_HINT, "RUNNING");
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                if self.running { self.step(); }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " "           => { self.running = !self.running; }
            "p" | "P"     => { self.rule = (self.rule + 1) % 3; self.gen = 0; }
            "s" | "S"     => { self.step(); }
            "r" | "R"     => { self.random_fill(); }
            "c" | "C"     => { self.cur.iter_mut().for_each(|c| *c = false); self.gen = 0; }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.render();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.render();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
