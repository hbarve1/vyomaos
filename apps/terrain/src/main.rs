use std::io::{self, BufRead, Write};

const W: i32  = 960;
const H: i32  = 720;
const GY: i32 = 24;
const GW: usize = 120;
const GH: usize = 80;
const CELL: i32 = 8;
const DS: usize = 129;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn rand01(rng: &mut u64) -> f32 {
    *rng = lcg(*rng);
    ((*rng >> 32) as f32) / (u32::MAX as f32)
}

fn rgba(r: u8, g: u8, b: u8) -> u32 {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
}

fn lerp_rgb(a: (u8, u8, u8), b: (u8, u8, u8), t: f32) -> u32 {
    let t = t.clamp(0.0, 1.0);
    rgba(
        (a.0 as f32 + (b.0 as f32 - a.0 as f32) * t) as u8,
        (a.1 as f32 + (b.1 as f32 - a.1 as f32) * t) as u8,
        (a.2 as f32 + (b.2 as f32 - a.2 as f32) * t) as u8,
    )
}

fn height_color(h: f32) -> u32 {
    if h < 0.20 {
        lerp_rgb((10, 20, 90), (30, 60, 160), h / 0.20)
    } else if h < 0.35 {
        lerp_rgb((30, 60, 160), (60, 110, 200), (h - 0.20) / 0.15)
    } else if h < 0.55 {
        lerp_rgb((34, 110, 34), (70, 150, 50), (h - 0.35) / 0.20)
    } else if h < 0.72 {
        lerp_rgb((160, 130, 70), (200, 185, 110), (h - 0.55) / 0.17)
    } else {
        lerp_rgb((215, 205, 195), (255, 255, 255), (h - 0.72) / 0.28)
    }
}

fn diamond_square(rng: &mut u64) -> Vec<f32> {
    let mut g = vec![-1.0f32; DS * DS];

    g[0]                  = rand01(rng);
    g[DS - 1]             = rand01(rng);
    g[(DS - 1) * DS]      = rand01(rng);
    g[(DS - 1) * DS + DS - 1] = rand01(rng);

    let mut size = DS - 1;
    let mut amp  = 0.9f32;

    while size >= 2 {
        let half = size / 2;

        for y in (half..DS).step_by(size) {
            for x in (half..DS).step_by(size) {
                let tl = g[(y - half) * DS + x - half];
                let tr = g[(y - half) * DS + x + half];
                let bl = g[(y + half) * DS + x - half];
                let br = g[(y + half) * DS + x + half];
                g[y * DS + x] = ((tl + tr + bl + br) / 4.0 + (rand01(rng) - 0.5) * amp)
                    .clamp(0.0, 1.0);
            }
        }

        for y in (0..DS).step_by(half) {
            for x in (0..DS).step_by(half) {
                if g[y * DS + x] >= 0.0 { continue; }
                let mut sum = 0.0f32;
                let mut cnt = 0u32;
                if y >= half       { sum += g[(y - half) * DS + x]; cnt += 1; }
                if y + half < DS   { sum += g[(y + half) * DS + x]; cnt += 1; }
                if x >= half       { sum += g[y * DS + x - half];   cnt += 1; }
                if x + half < DS   { sum += g[y * DS + x + half];   cnt += 1; }
                if cnt > 0 {
                    g[y * DS + x] = (sum / cnt as f32 + (rand01(rng) - 0.5) * amp)
                        .clamp(0.0, 1.0);
                }
            }
        }

        size = half;
        amp *= 0.58;
    }

    g
}

struct App {
    grid: Vec<f32>,
    rng:  u64,
}

impl App {
    fn new() -> Self {
        let mut rng: u64 = 0xDEAD_BEEF_1234_5678;
        let grid = diamond_square(&mut rng);
        App { grid, rng }
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(12, 4, C_TEXT, "Terrain Generator — Diamond-Square Fractal");
        text(680, 4, C_HINT, "R=regenerate  Q=quit");

        for gy in 0..GH {
            let ds_y = gy * (DS - 1) / (GH - 1);
            let mut run_col: Option<u32> = None;
            let mut run_x0 = 0usize;
            for gx in 0..GW {
                let ds_x = gx * (DS - 1) / (GW - 1);
                let h = self.grid[ds_y * DS + ds_x];
                let color = height_color(h);
                match run_col {
                    None => { run_col = Some(color); run_x0 = gx; }
                    Some(c) if c == color => {}
                    Some(c) => {
                        fill(run_x0 as i32 * CELL, GY + gy as i32 * CELL,
                             (gx - run_x0) as i32 * CELL, CELL, c);
                        run_col = Some(color); run_x0 = gx;
                    }
                }
            }
            if let Some(c) = run_col {
                fill(run_x0 as i32 * CELL, GY + gy as i32 * CELL,
                     (GW - run_x0) as i32 * CELL, CELL, c);
            }
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "r" | "R" => {
                self.rng = lcg(self.rng);
                self.grid = diamond_square(&mut self.rng);
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
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
