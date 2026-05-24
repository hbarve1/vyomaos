use std::io::{self, BufRead, Write};

const W: i32  = 960;
const H: i32  = 720;
const GY: i32 = 24;
const TW: usize = 120;
const TH: usize = 80;
const CELL: i32 = 8;
const DS: usize = 129; // 2^7 + 1

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;

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

fn lerp_u8(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 * (1.0 - t) + b as f32 * t) as u8
}

fn height_color(h: f32) -> u32 {
    let (r, g, b) = if h < 0.25 {
        let t = h / 0.25;
        (lerp_u8(8,  18, t), lerp_u8(20,  60, t), lerp_u8(120, 190, t))
    } else if h < 0.40 {
        let t = (h - 0.25) / 0.15;
        (lerp_u8(18, 40, t), lerp_u8(60, 130, t), lerp_u8(190,  55, t))
    } else if h < 0.60 {
        let t = (h - 0.40) / 0.20;
        (lerp_u8(40, 80, t), lerp_u8(130, 160, t), lerp_u8(55, 40, t))
    } else if h < 0.75 {
        let t = (h - 0.60) / 0.15;
        (lerp_u8(80, 165, t), lerp_u8(160, 145, t), lerp_u8(40, 90, t))
    } else {
        let t = (h - 0.75) / 0.25;
        (lerp_u8(165, 235, t), lerp_u8(145, 235, t), lerp_u8(90, 235, t))
    };
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
}

fn diamond_square(rng: &mut u64) -> Vec<f32> {
    let mut h = vec![0.0f32; DS * DS];

    let rnd = |rng: &mut u64| -> f32 {
        *rng = lcg(*rng);
        (*rng >> 32) as f32 / u32::MAX as f32
    };

    h[0]           = rnd(rng);
    h[DS - 1]      = rnd(rng);
    h[(DS-1)*DS]   = rnd(rng);
    h[(DS-1)*DS + DS-1] = rnd(rng);

    let mut step = 128usize;
    let mut scale = 0.85f32;

    while step > 1 {
        let half = step / 2;

        let mut y = 0;
        while y < DS - 1 {
            let mut x = 0;
            while x < DS - 1 {
                let avg = (h[y*DS+x] + h[y*DS+x+step]
                         + h[(y+step)*DS+x] + h[(y+step)*DS+x+step]) * 0.25;
                let v = avg + (rnd(rng) - 0.5) * scale;
                h[(y+half)*DS + x+half] = v.clamp(0.0, 1.0);
                x += step;
            }
            y += step;
        }

        let mut y = 0;
        while y < DS {
            let x0 = if (y / half) % 2 == 0 { half } else { 0 };
            let mut x = x0;
            while x < DS {
                let mut sum = 0.0f32;
                let mut cnt = 0u32;
                if y >= half   { sum += h[(y-half)*DS+x]; cnt += 1; }
                if y+half < DS { sum += h[(y+half)*DS+x]; cnt += 1; }
                if x >= half   { sum += h[y*DS+x-half];   cnt += 1; }
                if x+half < DS { sum += h[y*DS+x+half];   cnt += 1; }
                let v = sum / cnt as f32 + (rnd(rng) - 0.5) * scale;
                h[y*DS + x] = v.clamp(0.0, 1.0);
                x += step;
            }
            y += half;
        }

        step = half;
        scale *= 0.55;
    }
    h
}

fn sample_terrain(ds: &[f32]) -> Vec<f32> {
    let mut t = vec![0.0f32; TW * TH];
    for y in 0..TH {
        for x in 0..TW {
            let sx = x * (DS - 1) / (TW - 1);
            let sy = y * (DS - 1) / (TH - 1);
            t[y * TW + x] = ds[sy * DS + sx];
        }
    }
    t
}

struct App {
    terrain: Vec<f32>,
    rng:     u64,
}

impl App {
    fn new() -> Self {
        let mut rng: u64 = 0xDEADBEEF_CAFEBABE;
        let ds = diamond_square(&mut rng);
        App { terrain: sample_terrain(&ds), rng }
    }

    fn regen(&mut self) {
        self.rng = lcg(self.rng);
        let ds = diamond_square(&mut self.rng);
        self.terrain = sample_terrain(&ds);
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(12, 4, C_TEXT, "Terrain Generator — diamond-square fractal");
        text(680, 4, C_HINT, "R=regenerate  Q=quit");

        fill(0, GY, W, H - GY, C_BG);

        for y in 0..TH {
            let mut run_start = 0usize;
            let mut run_color = height_color(self.terrain[y * TW]);
            for x in 1..=TW {
                let col = if x < TW { height_color(self.terrain[y * TW + x]) } else { 0 };
                if col != run_color || x == TW {
                    fill(run_start as i32 * CELL,
                         GY + y as i32 * CELL,
                         (x - run_start) as i32 * CELL,
                         CELL,
                         run_color);
                    run_start = x;
                    run_color = col;
                }
            }
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "r" | "R"           => { self.regen(); }
            "q" | "Q" | "\x03"  => std::process::exit(0),
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
