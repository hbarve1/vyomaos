use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const MP: i32 = 256;
const G: i64 = 2000;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_SEL: u32    = 0x58A6FFFF;

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
    if n <= 0 { return 1; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x { x = y; y = (x + n / x) / 2; }
    x.max(1)
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const COLORS: [u32; 8] = [
    0xFFA657FF, 0x58A6FFFF, 0x3FB950FF, 0xC77DFFFF,
    0xFF7B72FF, 0x00CECBFF, 0xFFD93DFF, 0xFF6392FF,
];

struct Planet {
    x: i32, y: i32,
    vx: i32, vy: i32,
    mass: i32,
    color: u32,
    trail: Vec<(i16, i16)>,
}

impl Planet {
    fn px(&self) -> i32 { self.x / MP }
    fn py(&self) -> i32 { self.y / MP }
    fn radius(&self) -> i32 { self.mass * 2 + 5 }
}

fn circle(cx: i32, cy: i32, r: i32, c: u32) {
    for dy in -r..=r {
        let dx2 = r * r - dy * dy;
        if dx2 < 0 { continue; }
        let dx = isqrt(dx2 as i64) as i32;
        fill(cx - dx, cy + dy, dx * 2 + 1, 1, c);
    }
}

fn init_planets() -> Vec<Planet> {
    let cx = W / 2;
    let cy = H / 2;
    vec![
        Planet { x: cx*MP, y: cy*MP, vx: 0, vy: 0, mass: 8, color: COLORS[0], trail: Vec::new() },
        Planet { x: (cx+150)*MP, y: cy*MP, vx: 0, vy: 2600, mass: 3, color: COLORS[1], trail: Vec::new() },
        Planet { x: cx*MP, y: (cy-120)*MP, vx: -2900, vy: 0, mass: 2, color: COLORS[2], trail: Vec::new() },
        Planet { x: (cx-170)*MP, y: cy*MP, vx: 0, vy: -2400, mass: 5, color: COLORS[3], trail: Vec::new() },
        Planet { x: cx*MP, y: (cy+130)*MP, vx: 2800, vy: 0, mass: 4, color: COLORS[4], trail: Vec::new() },
    ]
}

struct App {
    planets: Vec<Planet>,
    paused: bool,
    seed: u64,
}

impl App {
    fn new() -> Self { App { planets: init_planets(), paused: false, seed: 0xC0DE_BABE_DEAD_BEEFu64 } }

    fn step(&mut self) {
        let n = self.planets.len();
        let mut dvx = vec![0i32; n];
        let mut dvy = vec![0i32; n];

        for i in 0..n {
            for j in (i + 1)..n {
                let dx = (self.planets[j].x - self.planets[i].x) / MP;
                let dy = (self.planets[j].y - self.planets[i].y) / MP;
                let dist_sq = (dx * dx + dy * dy) as i64;
                if dist_sq < 400 { continue; }
                let dist = isqrt(dist_sq);
                let denom = dist_sq * dist;
                let fj = G * self.planets[j].mass as i64 * MP as i64;
                let fi = G * self.planets[i].mass as i64 * MP as i64;
                dvx[i] += (fj * dx as i64 / denom) as i32;
                dvy[i] += (fj * dy as i64 / denom) as i32;
                dvx[j] -= (fi * dx as i64 / denom) as i32;
                dvy[j] -= (fi * dy as i64 / denom) as i32;
            }
        }

        for i in 0..n {
            self.planets[i].vx += dvx[i];
            self.planets[i].vy += dvy[i];
            self.planets[i].x += self.planets[i].vx;
            self.planets[i].y += self.planets[i].vy;

            let r = self.planets[i].radius();
            let px = self.planets[i].px();
            let py = self.planets[i].py();
            if px < r { self.planets[i].x = r * MP; self.planets[i].vx = self.planets[i].vx.abs(); }
            if px > W - r { self.planets[i].x = (W - r) * MP; self.planets[i].vx = -self.planets[i].vx.abs(); }
            if py < 36 + r { self.planets[i].y = (36 + r) * MP; self.planets[i].vy = self.planets[i].vy.abs(); }
            if py > H - 28 - r { self.planets[i].y = (H - 28 - r) * MP; self.planets[i].vy = -self.planets[i].vy.abs(); }

            let (px, py) = (self.planets[i].px() as i16, self.planets[i].py() as i16);
            self.planets[i].trail.push((px, py));
            if self.planets[i].trail.len() > 40 { self.planets[i].trail.remove(0); }
        }
    }

    fn add_planet(&mut self) {
        if self.planets.len() >= 8 { return; }
        self.seed = lcg(self.seed);
        let px = 40 + (self.seed % (W as u64 - 80)) as i32;
        self.seed = lcg(self.seed);
        let py = 60 + (self.seed % 580) as i32;
        self.seed = lcg(self.seed);
        let vx = (self.seed % 6000) as i32 - 3000;
        self.seed = lcg(self.seed);
        let vy = (self.seed % 6000) as i32 - 3000;
        self.seed = lcg(self.seed);
        let mass = 1 + (self.seed % 6) as i32;
        let color = COLORS[self.planets.len() % 8];
        self.planets.push(Planet { x: px * MP, y: py * MP, vx, vy, mass, color, trail: Vec::new() });
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Gravity Simulator");
        text(170, 8, C_HINT, &format!("A=add ({}/8)  R=reset  Space=pause  Q=quit", self.planets.len()));

        for planet in &self.planets {
            let n = planet.trail.len();
            for (k, &(tx, ty)) in planet.trail.iter().enumerate() {
                let s = if k + 8 >= n { 2 } else { 1 };
                fill(tx as i32, ty as i32, s, s, planet.color);
            }
        }

        for planet in &self.planets {
            circle(planet.px(), planet.py(), planet.radius(), planet.color);
            let lx = planet.px() + planet.radius() + 2;
            let ly = planet.py() - 8;
            text(lx, ly, planet.color, &format!("m{}", planet.mass));
        }

        if self.paused {
            fill(W / 2 - 50, H / 2 - 16, 100, 32, C_HEADER);
            text(W / 2 - 36, H / 2 - 8, C_SEL, "[PAUSED]");
        }

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, "N-body gravity: G=2000  bounce on edges  trail=40 steps");
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                if !self.paused { self.step(); }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " " => { self.paused = !self.paused; }
            "a" | "A" => { self.add_planet(); }
            "r" | "R" => { self.planets = init_planets(); }
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
