use std::io::{self, BufRead, Write};

const W: i32 = 800;
const H: i32 = 700;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_BACK: u32   = 0x1E3050FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn draw_line(x1: i32, y1: i32, x2: i32, y2: i32, c: u32) {
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    let sx: i32 = if x1 < x2 { 1 } else { -1 };
    let sy: i32 = if y1 < y2 { 1 } else { -1 };
    let mut err = dx - dy;
    let (mut x, mut y) = (x1, y1);
    loop {
        fill(x, y, 2, 2, c);
        if x == x2 && y == y2 { break; }
        let e2 = 2 * err;
        if e2 > -dy { err -= dy; x += sx; }
        if e2 <  dx { err += dx; y += sy; }
    }
}

// Cube vertices in fixed-point (×1000 = 1 unit)
const VERTS: [[i32; 3]; 8] = [
    [-1000, -1000, -1000], [1000, -1000, -1000],
    [ 1000,  1000, -1000], [-1000,  1000, -1000],
    [-1000, -1000,  1000], [1000, -1000,  1000],
    [ 1000,  1000,  1000], [-1000,  1000,  1000],
];

const EDGES: [(usize, usize); 12] = [
    (0, 1), (1, 2), (2, 3), (3, 0), // back face
    (4, 5), (5, 6), (6, 7), (7, 4), // front face
    (0, 4), (1, 5), (2, 6), (3, 7), // connecting edges
];

struct App {
    sin_t: [i32; 360],
    cos_t: [i32; 360],
    ang_x: usize,
    ang_y: usize,
    zoom:  i32,
    ticks: u64,
}

impl App {
    fn new() -> Self {
        let mut sin_t = [0i32; 360];
        let mut cos_t = [0i32; 360];
        for i in 0..360usize {
            let r = i as f64 * std::f64::consts::PI / 180.0;
            sin_t[i] = (r.sin() * 1000.0).round() as i32;
            cos_t[i] = (r.cos() * 1000.0).round() as i32;
        }
        App { sin_t, cos_t, ang_x: 20, ang_y: 30, zoom: 300, ticks: 0 }
    }

    fn rot_y(&self, v: [i32; 3]) -> [i32; 3] {
        let s = self.sin_t[self.ang_y];
        let c = self.cos_t[self.ang_y];
        [(v[0]*c - v[2]*s)/1000, v[1], (v[0]*s + v[2]*c)/1000]
    }

    fn rot_x(&self, v: [i32; 3]) -> [i32; 3] {
        let s = self.sin_t[self.ang_x];
        let c = self.cos_t[self.ang_x];
        [v[0], (v[1]*c - v[2]*s)/1000, (v[1]*s + v[2]*c)/1000]
    }

    fn project(&self, v: [i32; 3]) -> (i32, i32) {
        const D: i32 = 4000;
        let zd = (D + v[2]).max(500);
        let px = W / 2 + (v[0] * self.zoom) / zd;
        let py = H / 2 + 16 + (v[1] * self.zoom) / zd;
        (px, py)
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "3D Cube Viewer");
        text(W - 456, 8, C_HINT, "\u{2190}\u{2192}\u{2191}\u{2193}=rotate  +/-=zoom  R=reset  Q=quit");

        // Rotate and project all vertices
        let mut rotated   = [[0i32; 3]; 8];
        let mut projected = [(0i32, 0i32); 8];
        for (i, &v) in VERTS.iter().enumerate() {
            let rv = self.rot_y(v);
            let rv = self.rot_x(rv);
            rotated[i]   = rv;
            projected[i] = self.project(rv);
        }

        // Draw 12 edges with depth-based color
        for &(a, b) in &EDGES {
            let mid_z = (rotated[a][2] + rotated[b][2]) / 2;
            let c = if mid_z > 200 { C_GREEN }
                    else if mid_z > -200 { C_SEL }
                    else { C_BACK };
            let (x1, y1) = projected[a];
            let (x2, y2) = projected[b];
            draw_line(x1, y1, x2, y2, c);
        }

        // Vertex dots
        for &(px, py) in &projected {
            fill(px - 3, py - 3, 6, 6, C_ORANGE);
        }

        // Color legend
        text(12, H - 72, C_HINT,  "Depth:");
        fill(68, H - 68, 10, 10, C_GREEN);  text(82, H - 72, C_GREEN,  "Front");
        fill(140, H - 68, 10, 10, C_SEL);   text(154, H - 72, C_SEL,   "Mid");
        fill(202, H - 68, 10, 10, C_BACK);  text(216, H - 72, C_HINT,  "Back");

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("Angle X: {:3}deg  Y: {:3}deg  Zoom: {:3}  Tick: {}",
                 self.ang_x, self.ang_y, self.zoom, self.ticks));

        flush();
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" {
            self.ticks += 1;
            self.ang_y = (self.ang_y + 2) % 360;
            self.draw();
            println!("@supervisor: ping");
            return;
        }
        match line {
            "\x1b[A" => { self.ang_x = (360 + self.ang_x - 5) % 360; self.draw(); }
            "\x1b[B" => { self.ang_x = (self.ang_x + 5) % 360; self.draw(); }
            "\x1b[C" => { self.ang_y = (self.ang_y + 5) % 360; self.draw(); }
            "\x1b[D" => { self.ang_y = (360 + self.ang_y - 5) % 360; self.draw(); }
            "+" | "=" => { if self.zoom < 600 { self.zoom += 20; } self.draw(); }
            "-" | "_" => { if self.zoom > 80  { self.zoom -= 20; } self.draw(); }
            "r" | "R" => { self.ang_x = 20; self.ang_y = 30; self.zoom = 300; self.draw(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
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
        let _ = io::stdout().flush();
    }
}
