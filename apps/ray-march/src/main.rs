use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GY: i32 = 24;
const COLS: usize = 80;
const ROWS: usize = 57;
const CELL: i32 = 12;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;

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

fn rgba(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
}

#[derive(Clone, Copy)]
struct V3 { x: f32, y: f32, z: f32 }

impl V3 {
    fn new(x: f32, y: f32, z: f32) -> Self { V3 { x, y, z } }
    fn add(self, o: V3) -> V3 { V3::new(self.x+o.x, self.y+o.y, self.z+o.z) }
    fn sub(self, o: V3) -> V3 { V3::new(self.x-o.x, self.y-o.y, self.z-o.z) }
    fn mul(self, t: f32) -> V3 { V3::new(self.x*t, self.y*t, self.z*t) }
    fn dot(self, o: V3) -> f32 { self.x*o.x + self.y*o.y + self.z*o.z }
    fn cross(self, o: V3) -> V3 {
        V3::new(self.y*o.z-self.z*o.y, self.z*o.x-self.x*o.z, self.x*o.y-self.y*o.x)
    }
    fn len(self) -> f32 { self.dot(self).sqrt() }
    fn norm(self) -> V3 { let l = self.len(); if l < 1e-9 { self } else { self.mul(1.0/l) } }
    fn abs(self) -> V3 { V3::new(self.x.abs(), self.y.abs(), self.z.abs()) }
    fn max_v(self, o: V3) -> V3 { V3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z)) }
    fn max_c(self) -> f32 { self.x.max(self.y).max(self.z) }
}

fn sdf_sphere(p: V3, c: V3, r: f32) -> f32 { p.sub(c).len() - r }

fn sdf_box(p: V3, c: V3, h: V3) -> f32 {
    let q = p.sub(c).abs().sub(h);
    q.max_v(V3::new(0.0, 0.0, 0.0)).len() + q.max_c().min(0.0)
}

fn scene(p: V3) -> (f32, u8) {
    let ds = sdf_sphere(p, V3::new(0.45, 0.0, 0.6), 0.48);
    let db = sdf_box(p, V3::new(-0.55, -0.08, 0.5), V3::new(0.32, 0.28, 0.32));
    let dp = p.y - (-0.5);
    let d = ds.min(db).min(dp);
    let mat = if ds < db && ds < dp { 0 }
              else if db < dp       { 1 }
              else                  { 2 };
    (d, mat)
}

fn march(ro: V3, rd: V3) -> Option<(f32, u8)> {
    let mut t = 0.02f32;
    for _ in 0..48 {
        let p = ro.add(rd.mul(t));
        let (d, mat) = scene(p);
        if d < 0.002 { return Some((t, mat)); }
        if t > 12.0 { break; }
        t += d.max(0.01);
    }
    None
}

fn scene_normal(p: V3) -> V3 {
    let e = 0.003f32;
    V3::new(
        scene(V3::new(p.x+e, p.y, p.z)).0 - scene(V3::new(p.x-e, p.y, p.z)).0,
        scene(V3::new(p.x, p.y+e, p.z)).0 - scene(V3::new(p.x, p.y-e, p.z)).0,
        scene(V3::new(p.x, p.y, p.z+e)).0 - scene(V3::new(p.x, p.y, p.z-e)).0,
    ).norm()
}

fn soft_shadow(ro: V3, rd: V3) -> f32 {
    let mut res = 1.0f32;
    let mut t = 0.03f32;
    for _ in 0..24 {
        let h = scene(ro.add(rd.mul(t))).0;
        if h < 0.001 { return 0.0; }
        res = res.min(5.0 * h / t);
        t += h.max(0.03);
        if t > 4.0 { break; }
    }
    res.clamp(0.0, 1.0)
}

fn sky_color(rd: V3) -> u32 {
    let t = (rd.y * 0.5 + 0.5).clamp(0.0, 1.0);
    rgba(
        (0x0D + ((0x2A - 0x0D) as f32 * t) as i32) as u8,
        (0x11 + ((0x3A - 0x11) as f32 * t) as i32) as u8,
        (0x17 + ((0x6A - 0x17) as f32 * t) as i32) as u8,
    )
}

fn surface_color(mat: u8, p: V3, bright: f32) -> u32 {
    let (r0, g0, b0): (f32, f32, f32) = match mat {
        0 => (255.0, 115.0, 50.0),
        1 => (70.0, 140.0, 255.0),
        _ => {
            let c = ((p.x.floor() as i32) + (p.z.floor() as i32)) & 1;
            if c == 0 { (185.0, 185.0, 185.0) } else { (65.0, 65.0, 65.0) }
        }
    };
    rgba(
        (r0 * bright).min(255.0) as u8,
        (g0 * bright).min(255.0) as u8,
        (b0 * bright).min(255.0) as u8,
    )
}

struct App {
    angle:  f32,
    paused: bool,
}

impl App {
    fn new() -> Self { App { angle: 0.0, paused: false } }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(12, 4, C_TEXT, "Ray Marcher — SDF sphere+box+checkerboard");
        text(480, 4, C_HINT, &format!("Space=pause  Q=quit  angle={:.0}°", self.angle.to_degrees()));

        let angle = self.angle;
        let eye = V3::new(angle.sin() * 2.8, 0.9, angle.cos() * 2.8);
        let target = V3::new(0.0, -0.1, 0.0);
        let fwd = target.sub(eye).norm();
        let right = fwd.cross(V3::new(0.0, 1.0, 0.0)).norm();
        let up = right.cross(fwd);
        let light = V3::new(0.55, 0.75, -0.36).norm();

        for j in 0..ROWS {
            for i in 0..COLS {
                let ux = ((i as f32 + 0.5) / COLS as f32 * 2.0 - 1.0) * 1.2;
                let uy = -((j as f32 + 0.5) / ROWS as f32 * 2.0 - 1.0)
                    * 1.2 * (ROWS as f32 / COLS as f32);
                let rd = fwd.add(right.mul(ux)).add(up.mul(uy)).norm();
                let color = match march(eye, rd) {
                    None => sky_color(rd),
                    Some((t, mat)) => {
                        let p = eye.add(rd.mul(t));
                        let n = scene_normal(p);
                        let diff = n.dot(light).max(0.0);
                        let shad = soft_shadow(p.add(n.mul(0.01)), light);
                        let bright = (0.12 + diff * shad * 0.88).min(1.0);
                        surface_color(mat, p, bright)
                    }
                };
                fill(i as i32 * CELL, GY + j as i32 * CELL, CELL, CELL, color);
            }
        }
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "REPLY:pong" => {
                if !self.paused {
                    self.angle = (self.angle + std::f32::consts::PI / 180.0)
                        .rem_euclid(2.0 * std::f32::consts::PI);
                }
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            " "          => { self.paused = !self.paused; }
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
