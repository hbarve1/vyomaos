use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

// Canvas dimensions and position
const CW: i32 = 800;
const CH: i32 = 600;
const CX: i32 = 20;
const CY: i32 = 52;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

const MAX_ITER: i64 = 64;
const FP: i64 = 1 << 28; // fixed-point shift

// Mandelbrot iteration in fixed-point i64
// Returns escape iteration count (0..MAX_ITER), MAX_ITER = inside set
fn mandelbrot(cx: i64, cy: i64) -> i64 {
    let mut zx: i64 = 0;
    let mut zy: i64 = 0;
    for i in 0..MAX_ITER {
        // zx2 = zx*zx >> FP_SHIFT, but zx is already scaled by FP
        let zx2 = (zx * zx) >> 28;
        let zy2 = (zy * zy) >> 28;
        if zx2 + zy2 > 4 * FP { return i; }
        let new_zx = zx2 - zy2 + cx;
        zy = 2 * ((zx * zy) >> 28) + cy;
        zx = new_zx;
    }
    MAX_ITER
}

// 5 palette functions: each takes iter 0..MAX_ITER and returns RGBA u32
// Palette 0: fire  (black→red→yellow→white)
fn palette_fire(iter: i64) -> u32 {
    if iter == MAX_ITER { return 0x000000FF; }
    let t = iter * 255 / MAX_ITER;
    let r = (t * 3).min(255) as u32;
    let g = ((t as i64 - 85).max(0) * 3).min(255) as u32;
    let b = ((t as i64 - 170).max(0) * 3).min(255) as u32;
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}
// Palette 1: ice (black→dark blue→cyan→white)
fn palette_ice(iter: i64) -> u32 {
    if iter == MAX_ITER { return 0x000010FF; }
    let t = iter * 255 / MAX_ITER;
    let b = (t * 3).min(255) as u32;
    let g = ((t as i64 - 85).max(0) * 3).min(255) as u32;
    let r = ((t as i64 - 170).max(0) * 3).min(255) as u32;
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}
// Palette 2: mono (grayscale)
fn palette_mono(iter: i64) -> u32 {
    if iter == MAX_ITER { return 0x000000FF; }
    let v = (iter * 255 / MAX_ITER) as u32;
    (v << 24) | (v << 16) | (v << 8) | 0xFF
}
// Palette 3: rainbow (HSV hue sweep)
fn palette_rainbow(iter: i64) -> u32 {
    if iter == MAX_ITER { return 0x000000FF; }
    let h = (iter * 360 / MAX_ITER) as u32;
    let region = h / 60;
    let rem = (h % 60) * 255 / 60;
    let (r, g, b): (u32, u32, u32) = match region {
        0 => (255, rem,       0),
        1 => (255 - rem, 255, 0),
        2 => (0, 255,         rem),
        3 => (0, 255 - rem,   255),
        4 => (rem, 0,         255),
        _ => (255, 0,         255 - rem),
    };
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}
// Palette 4: earth (dark green→brown→tan)
fn palette_earth(iter: i64) -> u32 {
    if iter == MAX_ITER { return 0x0A0A05FF; }
    let t = iter as u32 * 4;
    let r = (20 + t.min(80)) as u32;
    let g = (40 + t.min(60)) as u32;
    let b = (10 + (t / 3).min(40)) as u32;
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

fn color_for(iter: i64, palette: usize) -> u32 {
    match palette {
        0 => palette_fire(iter),
        1 => palette_ice(iter),
        2 => palette_mono(iter),
        3 => palette_rainbow(iter),
        _ => palette_earth(iter),
    }
}

const PALETTE_NAMES: &[&str] = &["Fire", "Ice", "Mono", "Rainbow", "Earth"];

struct App {
    // View center in fixed-point, scale = width in fixed-point units
    cx:      i64,
    cy:      i64,
    scale:   i64,  // FP units spanning the full canvas width
    palette: usize,
}

impl App {
    fn new() -> Self {
        App {
            cx:      (-1 * FP) / 2,  // -0.5 in FP
            cy:      0,
            scale:   3 * FP,          // 3.0 units wide
            palette: 0,
        }
    }

    fn reset(&mut self) {
        self.cx = (-1 * FP) / 2;
        self.cy = 0;
        self.scale = 3 * FP;
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Fractal Explorer");
        text(200, 8, C_HINT, "Arrows:pan  +/-:zoom  C:palette  R:reset  Q:quit");

        // Render Mandelbrot canvas pixel by pixel (2×2 blocks for speed)
        let block = 2i32;
        let cols = CW / block;
        let rows = CH / block;
        for py in 0..rows {
            for px in 0..cols {
                // Map pixel to complex plane
                let fx = self.cx + (px as i64 * 2 - cols as i64) * self.scale / CW as i64;
                let fy = self.cy + (py as i64 * 2 - rows as i64) * self.scale * CH as i64 / (CW as i64 * rows as i64);
                let iter = mandelbrot(fx, fy);
                let c = color_for(iter, self.palette);
                fill(CX + px * block, CY + py * block, block, block, c);
            }
        }

        border(CX - 1, CY - 1, CW + 2, CH + 2, C_BORDER);

        // Info panel (right side)
        let px = CX + CW + 12;
        fill(px, CY, W - px - 4, CH, C_CARD);
        border(px, CY, W - px - 4, CH, C_BORDER);
        text(px + 8, CY + 8,  C_ORANGE, "Palette:");
        text(px + 8, CY + 26, C_SEL, PALETTE_NAMES[self.palette]);
        text(px + 8, CY + 52, C_ORANGE, "Controls:");
        text(px + 8, CY + 70,  C_HINT, "Arrow keys: pan");
        text(px + 8, CY + 88,  C_HINT, "+/-: zoom in/out");
        text(px + 8, CY + 106, C_HINT, "C: cycle palette");
        text(px + 8, CY + 124, C_HINT, "R: reset view");
        text(px + 8, CY + 142, C_HINT, "Q: quit");
        text(px + 8, CY + 170, C_ORANGE, "View:");
        text(px + 8, CY + 188, C_HINT, &format!("Cx: {:.4}", self.cx as f64 / FP as f64));
        text(px + 8, CY + 206, C_HINT, &format!("Cy: {:.4}", self.cy as f64 / FP as f64));
        text(px + 8, CY + 224, C_HINT, &format!("W:  {:.4}", self.scale as f64 / FP as f64));

        // Palette swatches
        text(px + 8, CY + 260, C_ORANGE, "Palettes:");
        for (i, name) in PALETTE_NAMES.iter().enumerate() {
            let sy = CY + 278 + i as i32 * 20;
            let swatch = color_for(i as i64 * MAX_ITER / PALETTE_NAMES.len() as i64, i);
            fill(px + 8, sy, 12, 12, swatch);
            let tc = if i == self.palette { C_SEL } else { C_HINT };
            let marker = if i == self.palette { ">" } else { " " };
            text(px + 24, sy, tc, &format!("{} {}", marker, name));
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!("Mandelbrot  palette={}  zoom={:.2}x  center=({:.3},{:.3})",
             PALETTE_NAMES[self.palette],
             3.0 / (self.scale as f64 / FP as f64),
             self.cx as f64 / FP as f64,
             self.cy as f64 / FP as f64));
        flush();
    }

    fn handle(&mut self, line: &str) {
        let pan = self.scale / 8;
        match line {
            "\x1b[A" => self.cy -= pan,
            "\x1b[B" => self.cy += pan,
            "\x1b[D" => self.cx -= pan,
            "\x1b[C" => self.cx += pan,
            "+" | "=" => self.scale /= 2,
            "-" => self.scale *= 2,
            "c" | "C" => self.palette = (self.palette + 1) % PALETTE_NAMES.len(),
            "r" | "R" => self.reset(),
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
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
