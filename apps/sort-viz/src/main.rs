use std::io::{self, BufRead, Write};

const W: i32    = 960;
const H: i32    = 720;
const GY: i32   = 24;
const N: usize  = 80;
const BAR_W: i32 = W / N as i32;  // 12
const BAR_AREA: i32 = 636;
const BAR_BASE: i32 = GY + BAR_AREA;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_CARD:   u32 = 0x161B22FF;

const ALGO_NAMES: [&str; 5] = ["Bubble", "Insertion", "Selection", "Merge", "Quick"];

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

fn shuffle(vals: &mut [u8; N], rng: &mut u64) {
    for i in (1..N).rev() {
        *rng = lcg(*rng);
        let j = ((*rng >> 32) as usize) % (i + 1);
        vals.swap(i, j);
    }
}

fn gen_bubble(a: &mut [u8; N]) -> Vec<(u8, u8, bool)> {
    let mut mv = vec![];
    for i in 0..N - 1 {
        for j in 0..N - 1 - i {
            let sw = a[j] > a[j + 1];
            mv.push((j as u8, (j + 1) as u8, sw));
            if sw { a.swap(j, j + 1); }
        }
    }
    mv
}

fn gen_insertion(a: &mut [u8; N]) -> Vec<(u8, u8, bool)> {
    let mut mv = vec![];
    for i in 1..N {
        let mut j = i;
        while j > 0 {
            let sw = a[j - 1] > a[j];
            mv.push(((j - 1) as u8, j as u8, sw));
            if sw { a.swap(j - 1, j); j -= 1; } else { break; }
        }
    }
    mv
}

fn gen_selection(a: &mut [u8; N]) -> Vec<(u8, u8, bool)> {
    let mut mv = vec![];
    for i in 0..N - 1 {
        let mut m = i;
        for j in i + 1..N {
            mv.push((m as u8, j as u8, false));
            if a[j] < a[m] { m = j; }
        }
        if m != i { mv.push((i as u8, m as u8, true)); a.swap(i, m); }
    }
    mv
}

fn gen_merge(a: &mut [u8; N]) -> Vec<(u8, u8, bool)> {
    let mut mv = vec![];
    let mut width = 1usize;
    while width < N {
        let mut start = 0usize;
        while start < N {
            let mid = (start + width).min(N);
            let end = (start + 2 * width).min(N);
            let mut l = start;
            let mut r = mid;
            while l < r && r < end {
                if a[l] <= a[r] {
                    mv.push((l as u8, r as u8, false));
                    l += 1;
                } else {
                    let mut k = r;
                    while k > l {
                        mv.push(((k - 1) as u8, k as u8, true));
                        a.swap(k - 1, k);
                        k -= 1;
                    }
                    l += 1;
                    r += 1;
                }
            }
            start += 2 * width;
        }
        width *= 2;
    }
    mv
}

fn gen_quick(a: &mut [u8; N]) -> Vec<(u8, u8, bool)> {
    let mut mv = vec![];
    let mut stack: Vec<(usize, usize)> = vec![];
    if N > 1 { stack.push((0, N - 1)); }
    while let Some((lo, hi)) = stack.pop() {
        if lo >= hi { continue; }
        let pivot = a[hi];
        let mut i = lo;
        for j in lo..hi {
            mv.push((j as u8, hi as u8, false));
            if a[j] <= pivot {
                if i != j { mv.push((i as u8, j as u8, true)); a.swap(i, j); }
                i += 1;
            }
        }
        if i != hi { mv.push((i as u8, hi as u8, true)); a.swap(i, hi); }
        if i > lo + 1 { stack.push((lo, i - 1)); }
        if i + 1 < hi { stack.push((i + 1, hi)); }
    }
    mv
}

fn build_moves(algo: usize, vals: &[u8; N]) -> Vec<(u8, u8, bool)> {
    let mut a = *vals;
    match algo {
        0 => gen_bubble(&mut a),
        1 => gen_insertion(&mut a),
        2 => gen_selection(&mut a),
        3 => gen_merge(&mut a),
        _ => gen_quick(&mut a),
    }
}

struct App {
    vals:     [u8; N],
    algo:     usize,
    moves:    Vec<(u8, u8, bool)>,
    pos:      usize,
    hi:       [usize; 2],
    comps:    u64,
    swaps:    u64,
    rng:      u64,
    done:     bool,
}

impl App {
    fn new() -> Self {
        let mut vals: [u8; N] = std::array::from_fn(|i| (i + 1) as u8);
        let mut rng: u64 = 0xDEADBEEF_CAFEBABE;
        shuffle(&mut vals, &mut rng);
        let moves = build_moves(0, &vals);
        App { vals, algo: 0, moves, pos: 0, hi: [0, 1], comps: 0, swaps: 0, rng, done: false }
    }

    fn reset(&mut self) {
        shuffle(&mut self.vals, &mut self.rng);
        self.moves = build_moves(self.algo, &self.vals);
        self.pos = 0;
        self.comps = 0;
        self.swaps = 0;
        self.done = false;
    }

    fn tick(&mut self) {
        if self.pos >= self.moves.len() {
            self.done = true;
            return;
        }
        let (a, b, sw) = self.moves[self.pos];
        self.hi = [a as usize, b as usize];
        self.comps += 1;
        if sw { self.vals.swap(a as usize, b as usize); self.swaps += 1; }
        self.pos += 1;
        if self.pos >= self.moves.len() { self.done = true; }
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(8, 4, C_TEXT, ALGO_NAMES[self.algo]);
        let status = format!("Steps:{} Swaps:{} Comps:{}", self.pos, self.swaps, self.comps);
        text(200, 4, C_HINT, &status);
        text(620, 4, C_HINT, "Tab=algo  Space=reset  Q=quit");

        fill(0, GY, W, H - GY, C_BG);

        for i in 0..N {
            let bh = self.vals[i] as i32 * BAR_AREA / N as i32;
            let by = BAR_BASE - bh;
            let bx = i as i32 * BAR_W;
            let col = if self.done {
                C_GREEN
            } else if i == self.hi[0] || i == self.hi[1] {
                C_ORANGE
            } else {
                C_SEL
            };
            fill(bx, by, BAR_W - 1, bh, col);
        }

        // legend strip
        fill(0, BAR_BASE + 2, W, 2, C_CARD);
        for (i, name) in ALGO_NAMES.iter().enumerate() {
            let lx = 10 + i as i32 * 190;
            let col = if i == self.algo { C_ORANGE } else { C_HINT };
            text(lx, BAR_BASE + 8, col, &format!("[{}] {}", i + 1, name));
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                if !self.done { self.tick(); }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            "\t" => {
                self.algo = (self.algo + 1) % 5;
                self.reset();
            }
            " " => { self.reset(); }
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
