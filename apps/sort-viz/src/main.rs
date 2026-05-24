use std::io::{self, BufRead, Write};

const W: i32    = 960;
const H: i32    = 720;
const GY: i32   = 24;
const BARS: usize = 80;
const BAR_W: i32  = W / BARS as i32;
const BAR_BASE: i32 = H - 16;
const BAR_MAX_H: i32 = H - GY - 36;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;

const ALGO_NAMES: [&str; 5] = [
    "Bubble Sort", "Insertion Sort", "Selection Sort", "Merge Sort", "Quick Sort",
];

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

fn gen_bubble(v: &[u8]) -> Vec<(usize, usize, bool)> {
    let mut v = v.to_vec();
    let n = v.len();
    let mut ops = Vec::new();
    for i in 0..n {
        for j in 0..n - 1 - i {
            let sw = v[j] > v[j + 1];
            ops.push((j, j + 1, sw));
            if sw { v.swap(j, j + 1); }
        }
    }
    ops
}

fn gen_insertion(v: &[u8]) -> Vec<(usize, usize, bool)> {
    let mut v = v.to_vec();
    let n = v.len();
    let mut ops = Vec::new();
    for i in 1..n {
        let mut j = i;
        while j > 0 {
            let sw = v[j - 1] > v[j];
            ops.push((j - 1, j, sw));
            if sw { v.swap(j - 1, j); j -= 1; } else { break; }
        }
    }
    ops
}

fn gen_selection(v: &[u8]) -> Vec<(usize, usize, bool)> {
    let mut v = v.to_vec();
    let n = v.len();
    let mut ops = Vec::new();
    for i in 0..n - 1 {
        let mut mi = i;
        for j in i + 1..n {
            let less = v[j] < v[mi];
            ops.push((j, mi, false));
            if less { mi = j; }
        }
        if mi != i {
            ops.push((i, mi, true));
            v.swap(i, mi);
        }
    }
    ops
}

fn gen_merge(v: &[u8]) -> Vec<(usize, usize, bool)> {
    let mut v = v.to_vec();
    let n = v.len();
    let mut ops = Vec::new();
    let mut width = 1usize;
    while width < n {
        let mut lo = 0usize;
        while lo < n {
            let mid = (lo + width).min(n - 1);
            let hi  = (lo + 2 * width - 1).min(n - 1);
            let mut i = mid + 1;
            while i <= hi {
                let mut j = i;
                while j > lo {
                    let sw = v[j - 1] > v[j];
                    ops.push((j - 1, j, sw));
                    if sw { v.swap(j - 1, j); j -= 1; } else { break; }
                }
                i += 1;
            }
            lo += 2 * width;
        }
        width *= 2;
    }
    ops
}

fn gen_quick(v: &[u8]) -> Vec<(usize, usize, bool)> {
    let mut v = v.to_vec();
    let n = v.len();
    let mut ops = Vec::new();
    if n <= 1 { return ops; }
    let mut stack: Vec<(usize, usize)> = vec![(0, n - 1)];
    while let Some((lo, hi)) = stack.pop() {
        if lo >= hi { continue; }
        let pivot = v[hi];
        let mut i = lo;
        for j in lo..hi {
            ops.push((j, hi, false));
            if v[j] <= pivot {
                if i != j {
                    ops.push((i, j, true));
                    v.swap(i, j);
                }
                i += 1;
            }
        }
        if i != hi {
            ops.push((i, hi, true));
            v.swap(i, hi);
        }
        let p = i;
        if p > lo { stack.push((lo, p - 1)); }
        if p < hi { stack.push((p + 1, hi)); }
    }
    ops
}

fn gen_ops(algo: usize, vals: &[u8]) -> Vec<(usize, usize, bool)> {
    match algo {
        0 => gen_bubble(vals),
        1 => gen_insertion(vals),
        2 => gen_selection(vals),
        3 => gen_merge(vals),
        _ => gen_quick(vals),
    }
}

struct App {
    vals: Vec<u8>,
    orig: Vec<u8>,
    ops:  Vec<(usize, usize, bool)>,
    step: usize,
    hi:   (usize, usize),
    algo: usize,
    cmps: usize,
    swps: usize,
    rng:  u64,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            vals: vec![0; BARS],
            orig: vec![0; BARS],
            ops:  Vec::new(),
            step: 0,
            hi:   (0, 0),
            algo: 0,
            cmps: 0,
            swps: 0,
            rng:  0xBEEF_CAFE_1234_ABCD,
        };
        app.new_shuffle();
        app
    }

    fn new_shuffle(&mut self) {
        let mut vals: Vec<u8> = (1..=BARS as u8).collect();
        for i in (1..BARS).rev() {
            self.rng = lcg(self.rng);
            let j = (self.rng % (i as u64 + 1)) as usize;
            vals.swap(i, j);
        }
        self.orig = vals;
        self.reset();
    }

    fn reset(&mut self) {
        self.vals = self.orig.clone();
        self.ops  = gen_ops(self.algo, &self.orig);
        self.step = 0;
        self.hi   = (0, 0);
        self.cmps = 0;
        self.swps = 0;
    }

    fn tick(&mut self) {
        if self.step < self.ops.len() {
            let (a, b, sw) = self.ops[self.step];
            self.hi   = (a, b);
            self.cmps += 1;
            if sw {
                self.vals.swap(a, b);
                self.swps += 1;
            }
            self.step += 1;
        }
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        let done = self.step >= self.ops.len();
        let status = format!(
            "{}{}  Ops: {}/{}  Swaps: {}  Tab=algo  Space=shuffle  Q=quit",
            ALGO_NAMES[self.algo],
            if done { " [SORTED]" } else { "" },
            self.step, self.ops.len(), self.swps,
        );
        text(8, 4, C_TEXT, &status);

        fill(0, GY, W, H - GY, C_BG);

        let sorted_vals: Vec<u8> = (1..=BARS as u8).collect();
        for (i, &v) in self.vals.iter().enumerate() {
            let bar_h = (v as i32 * BAR_MAX_H / BARS as i32).max(2);
            let col = if i == self.hi.0 || i == self.hi.1 {
                C_ORANGE
            } else if self.vals[i] == sorted_vals[i] && self.step > 0 {
                C_GREEN
            } else {
                C_SEL
            };
            fill(i as i32 * BAR_W, BAR_BASE - bar_h, BAR_W - 1, bar_h, col);
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                self.tick();
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            "\t" => {
                self.algo = (self.algo + 1) % 5;
                self.reset();
            }
            " " => {
                self.new_shuffle();
            }
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
