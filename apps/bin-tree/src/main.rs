use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const N: usize = 31;
const RADIUS: i32 = 22;
const LEAF_SPACING: i32 = 48;
const LEAF_LEFT: i32 = 120; // (960 - 15*48) / 2

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_CARD:   u32 = 0x161B22FF;

const LEVEL_COLORS: [u32; 5] = [C_ORANGE, C_SEL, C_GREEN, 0x8B949EFF, C_BORDER];
const C_ACTIVE: u32 = 0xFFD700FF; // gold

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

fn isqrt(n: i64) -> i64 {
    if n <= 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x
}

fn level_of(k: usize) -> usize {
    let mut n = k;
    let mut l = 0;
    while n > 1 { n >>= 1; l += 1; }
    l
}

fn node_pos(k: usize) -> (i32, i32) {
    let level = level_of(k);
    let first = 1usize << level;
    let j = (k - first) as i32;
    let lspan = 1i32 << (4 - level); // leaves per subtree
    let x = LEAF_LEFT + j * lspan * LEAF_SPACING + (lspan - 1) * LEAF_SPACING / 2;
    let y = 80 + level as i32 * 100;
    (x, y)
}

fn circle(cx: i32, cy: i32, r: i32, c: u32) {
    for dy in -r..=r {
        let dx = isqrt((r * r - dy * dy) as i64) as i32;
        fill(cx - dx, cy + dy, 2 * dx + 1, 1, c);
    }
}

fn draw_line(x1: i32, y1: i32, x2: i32, y2: i32, c: u32) {
    let dx = x2 - x1;
    let dy = y2 - y1;
    let steps = dx.abs().max(dy.abs());
    if steps == 0 { return; }
    for t in 0..=steps {
        let x = x1 + dx * t / steps;
        let y = y1 + dy * t / steps;
        fill(x, y, 1, 1, c);
    }
}

fn inorder(result: &mut Vec<usize>, node: usize) {
    let mut stack = Vec::new();
    let mut curr = node;
    loop {
        while curr <= N { stack.push(curr); curr *= 2; }
        if stack.is_empty() { break; }
        curr = stack.pop().unwrap();
        result.push(curr);
        curr = curr * 2 + 1;
    }
}

fn preorder(result: &mut Vec<usize>, root: usize) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node > N { continue; }
        result.push(node);
        stack.push(node * 2 + 1);
        stack.push(node * 2);
    }
}

fn postorder(result: &mut Vec<usize>, root: usize) {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node > N { continue; }
        result.push(node);
        stack.push(node * 2);
        stack.push(node * 2 + 1);
    }
    result.reverse();
}

#[derive(Clone, Copy)]
enum Trav { InOrder, PreOrder, PostOrder, Bfs }

impl Trav {
    fn name(self) -> &'static str {
        match self { Trav::InOrder => "In-order", Trav::PreOrder => "Pre-order", Trav::PostOrder => "Post-order", Trav::Bfs => "BFS" }
    }
    fn next(self) -> Self {
        match self { Trav::InOrder => Trav::PreOrder, Trav::PreOrder => Trav::PostOrder, Trav::PostOrder => Trav::Bfs, Trav::Bfs => Trav::InOrder }
    }
}

struct App {
    travs:    [Vec<usize>; 4],
    mode:     Trav,
    step_idx: usize,
    paused:   bool,
}

impl App {
    fn new() -> Self {
        let mut io_ = Vec::new(); inorder(&mut io_, 1);
        let mut pre = Vec::new(); preorder(&mut pre, 1);
        let mut post = Vec::new(); postorder(&mut post, 1);
        let bfs: Vec<usize> = (1..=N).collect();
        App { travs: [io_, pre, post, bfs], mode: Trav::InOrder, step_idx: 0, paused: false }
    }

    fn current_trav(&self) -> &Vec<usize> {
        &self.travs[self.mode as usize]
    }

    fn current_node(&self) -> usize {
        let t = self.current_trav();
        if t.is_empty() { 1 } else { t[self.step_idx % t.len()] }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        fill(0, H - 24, W, 24, C_HEADER);

        let cur = self.current_node();
        let total = self.current_trav().len();
        text(12, 8, C_TEXT, "Binary Tree Visualizer");
        text(240, 8, C_HINT, &format!(
            "{}  Step {}/{}  Tab=mode  R=reset  Space={}  Q=quit",
            self.mode.name(), self.step_idx + 1, total,
            if self.paused { "PAUSED" } else { "pause" }
        ));

        // Draw edges first
        for k in 2..=N {
            let (cx, cy) = node_pos(k);
            let (px, py) = node_pos(k / 2);
            draw_line(px, py, cx, cy, C_BORDER);
        }

        // Draw nodes
        for k in 1..=N {
            let (cx, cy) = node_pos(k);
            let level = level_of(k);
            let is_active = k == cur;

            if is_active {
                circle(cx, cy, RADIUS + 2, C_ACTIVE);
                circle(cx, cy, RADIUS, LEVEL_COLORS[level]);
            } else {
                circle(cx, cy, RADIUS, LEVEL_COLORS[level]);
            }

            // Node number label
            let label = k.to_string();
            let lx = cx - if k >= 10 { 8 } else { 4 };
            let lc = if level < 3 { C_CARD } else { C_TEXT };
            text(lx, cy - 6, lc, &label);
        }

        // Traversal strip at bottom
        let trav = self.current_trav();
        let tx0 = (W - N as i32 * 29 + 5) / 2;
        let ty = H - 76;
        for (step, &node) in trav.iter().enumerate() {
            let bx = tx0 + step as i32 * 29;
            let is_cur = step == self.step_idx % N;
            let level = level_of(node);
            let bg = if is_cur { C_ACTIVE } else { LEVEL_COLORS[level] };
            fill(bx, ty, 24, 18, bg);
            let tc = if is_cur || level < 3 { C_CARD } else { C_TEXT };
            let lx = bx + if node >= 10 { 2 } else { 6 };
            text(lx, ty + 4, tc, &node.to_string());
        }

        text(12, H - 18, C_HINT, &format!("Active: node {}  ({})", cur, self.mode.name()));
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                if !self.paused {
                    self.step_idx = (self.step_idx + 1) % N;
                }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " " => { self.paused = !self.paused; }
            "\t" => {
                self.mode = self.mode.next();
                self.step_idx = 0;
            }
            "r" | "R" => { self.step_idx = 0; }
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
