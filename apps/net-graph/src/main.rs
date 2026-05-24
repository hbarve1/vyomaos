use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GY: i32 = 24;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_BG:     u32 = 0x0D1117FF;
const C_BORDER: u32 = 0x30363DFF;
const C_EDGE:   u32 = 0x30363DFF;

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

fn draw_line(x0: i32, y0: i32, x1: i32, y1: i32, c: u32) {
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let steps = dx.max(dy).max(1);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = x0 + ((x1 - x0) as f32 * t) as i32;
        let y = y0 + ((y1 - y0) as f32 * t) as i32;
        fill(x, y, 2, 2, c);
    }
}

fn isqrt(n: i64) -> i64 {
    if n <= 0 { return 0; }
    let mut x = n;
    loop {
        let x1 = (x + n / x) / 2;
        if x1 >= x { return x; }
        x = x1;
    }
}

fn draw_circle(cx: i32, cy: i32, r: i32, c: u32) {
    let r2 = (r * r) as i64;
    for dy in -r..=r {
        let dx = isqrt(r2 - (dy * dy) as i64) as i32;
        fill(cx - dx, cy + dy, dx * 2 + 1, 1, c);
    }
}

fn draw_ring(cx: i32, cy: i32, r: i32, c: u32) {
    let r2 = (r * r) as i64;
    let r1 = ((r - 2) * (r - 2)) as i64;
    for dy in -r..=r {
        let dy2 = (dy * dy) as i64;
        let ox = isqrt(r2 - dy2) as i32;
        let ix = if dy2 < r1 { isqrt(r1 - dy2) as i32 } else { 0 };
        if ox > ix {
            fill(cx - ox, cy + dy, ox - ix, 1, c);
            fill(cx + ix, cy + dy, ox - ix, 1, c);
        }
    }
}

const N: usize = 12;
const NODE_R: i32 = 20;

const EDGES: [(usize, usize); 18] = [
    (0, 1), (0, 2), (0, 3),
    (1, 4), (1, 5),
    (2, 4), (2, 6),
    (3, 6), (3, 7),
    (4, 8),
    (5, 8), (5, 9),
    (6, 9), (6, 10),
    (7, 10), (7, 11),
    (8, 11),
    (9, 11),
];

const LABELS: [&str; N] = [
    "A", "B", "C", "D", "E", "F",
    "G", "H", "I", "J", "K", "L",
];

fn node_color(deg: usize) -> u32 {
    if deg <= 1 { C_HINT }
    else if deg == 2 { C_SEL }
    else if deg == 3 { C_GREEN }
    else { C_ORANGE }
}

struct App {
    px: [f32; N],
    py: [f32; N],
    vx: [f32; N],
    vy: [f32; N],
    deg: [usize; N],
}

impl App {
    fn new() -> Self {
        let mut px = [0.0f32; N];
        let mut py = [0.0f32; N];
        let cx = 480.0f32;
        let cy = (H / 2 + GY / 2) as f32;
        let r = 250.0f32;
        for i in 0..N {
            let a = (i as f32) * 2.0 * std::f32::consts::PI / N as f32;
            px[i] = cx + r * a.cos();
            py[i] = cy + r * a.sin();
        }
        let mut deg = [0usize; N];
        for &(a, b) in &EDGES {
            deg[a] += 1;
            deg[b] += 1;
        }
        App { px, py, vx: [0.0; N], vy: [0.0; N], deg }
    }

    fn step_physics(&mut self) {
        let k = 0.08f32;
        let rest = 110.0f32;
        let rep = 2000.0f32;
        let damp = 0.8f32;

        let mut fx = [0.0f32; N];
        let mut fy = [0.0f32; N];

        for &(a, b) in &EDGES {
            let dx = self.px[b] - self.px[a];
            let dy = self.py[b] - self.py[a];
            let dist = (dx * dx + dy * dy).sqrt().max(1.0);
            let force = k * (dist - rest);
            let nx = dx / dist;
            let ny = dy / dist;
            fx[a] += force * nx;
            fy[a] += force * ny;
            fx[b] -= force * nx;
            fy[b] -= force * ny;
        }

        for i in 0..N {
            for j in 0..N {
                if i == j { continue; }
                let dx = self.px[i] - self.px[j];
                let dy = self.py[i] - self.py[j];
                let dist2 = (dx * dx + dy * dy).max(1.0);
                let dist = dist2.sqrt();
                let force = rep / dist2;
                fx[i] += force * dx / dist;
                fy[i] += force * dy / dist;
            }
        }

        for i in 0..N {
            self.vx[i] = (self.vx[i] + fx[i]) * damp;
            self.vy[i] = (self.vy[i] + fy[i]) * damp;
            self.px[i] = (self.px[i] + self.vx[i]).clamp(80.0, 880.0);
            self.py[i] = (self.py[i] + self.vy[i]).clamp((GY + 40) as f32, (H - 40) as f32);
        }
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(12, 4, C_TEXT, "Network Graph — force-directed layout");
        text(600, 4, C_HINT, "Q=quit");

        fill(0, GY, W, H - GY, C_BG);

        for &(a, b) in &EDGES {
            draw_line(
                self.px[a] as i32, self.py[a] as i32,
                self.px[b] as i32, self.py[b] as i32,
                C_EDGE,
            );
        }

        for i in 0..N {
            let cx = self.px[i] as i32;
            let cy = self.py[i] as i32;
            let col = node_color(self.deg[i]);
            draw_circle(cx, cy, NODE_R, col);
            draw_ring(cx, cy, NODE_R, C_BORDER);
            text(cx - 4, cy - 6, C_TEXT, LABELS[i]);
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                self.step_physics();
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
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
