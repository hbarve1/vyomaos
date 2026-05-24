use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GY: i32 = 24;
const CANVAS_W: i32 = 700;
const PANEL_X: i32 = 704;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;
const C_BORDER: u32 = 0x30363DFF;
const C_CARD:   u32 = 0x161B22FF;

const PT_COLS: [u32; 4] = [0x58A6FFFF, 0x3FB950FF, 0xFFA657FF, 0xFF7B72FF];
const PT_NAMES: [&str; 4] = ["P0", "P1", "P2", "P3"];

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

fn hue_rgba(h: f32) -> u32 {
    let h6 = (h * 6.0).rem_euclid(6.0);
    let s = h6 as u32;
    let f = h6 - s as f32;
    let (r, g, b): (u8, u8, u8) = match s {
        0 => (255, (f * 255.0) as u8, 0),
        1 => (((1.0 - f) * 255.0) as u8, 255, 0),
        2 => (0, 255, (f * 255.0) as u8),
        3 => (0, ((1.0 - f) * 255.0) as u8, 255),
        4 => ((f * 255.0) as u8, 0, 255),
        _ => (255, 0, ((1.0 - f) * 255.0) as u8),
    };
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
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

fn bezier(p: [(f32, f32); 4], t: f32) -> (f32, f32) {
    let mut q = p;
    for k in 1..4 {
        for i in 0..(4 - k) {
            q[i] = (
                q[i].0 * (1.0 - t) + q[i + 1].0 * t,
                q[i].1 * (1.0 - t) + q[i + 1].1 * t,
            );
        }
    }
    q[0]
}

struct App {
    pts:      [(i32, i32); 4],
    sel:      usize,
    show_tan: bool,
}

impl App {
    fn new() -> Self {
        App {
            pts: [(150, 500), (250, 200), (480, 560), (640, 260)],
            sel: 0,
            show_tan: true,
        }
    }

    fn pts_f(&self) -> [(f32, f32); 4] {
        self.pts.map(|(x, y)| (x as f32, y as f32))
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(12, 4, C_TEXT, "Bezier Curve Editor");
        text(440, 4, C_HINT, "Tab=select  Arrows=move  T=tangents  R=reset  Q=quit");

        fill(0, GY, CANVAS_W, H - GY, C_BG);
        fill(PANEL_X, GY, W - PANEL_X, H - GY, C_CARD);
        fill(PANEL_X - 1, GY, 1, H - GY, C_BORDER);

        let pts = self.pts_f();

        // control polygon (faint gray)
        for i in 0..3 {
            draw_line(pts[i].0 as i32, pts[i].1 as i32,
                      pts[i+1].0 as i32, pts[i+1].1 as i32, C_BORDER);
        }

        // tangent handles
        if self.show_tan {
            draw_line(pts[0].0 as i32, pts[0].1 as i32,
                      pts[1].0 as i32, pts[1].1 as i32, 0x9E6A03FF);
            draw_line(pts[2].0 as i32, pts[2].1 as i32,
                      pts[3].0 as i32, pts[3].1 as i32, 0x9E6A03FF);
        }

        // curve with hue trail
        let mut prev = bezier(pts, 0.0);
        for i in 1..=200 {
            let t = i as f32 / 200.0;
            let cur = bezier(pts, t);
            let color = hue_rgba(t * 0.85);
            let x = cur.0 as i32;
            let y = cur.1 as i32;
            let px = prev.0 as i32;
            let py = prev.1 as i32;
            let steps = ((x - px).abs()).max((y - py).abs()).max(1);
            for s in 0..=steps {
                let tx = px + (x - px) * s / steps;
                let ty = py + (y - py) * s / steps;
                fill(tx - 1, ty - 1, 3, 3, color);
            }
            prev = cur;
        }

        // control points
        for (i, &(cx, cy)) in self.pts.iter().enumerate() {
            let col = PT_COLS[i];
            if i == self.sel {
                fill(cx - 8, cy - 8, 16, 16, 0xFFFFFFFF);
                fill(cx - 6, cy - 6, 12, 12, col);
            } else {
                fill(cx - 5, cy - 5, 10, 10, col);
            }
        }

        // right panel
        text(PANEL_X + 10, GY + 12, C_HINT, "Control Points");
        let tan_str = if self.show_tan { "on" } else { "off" };
        text(PANEL_X + 10, GY + 28, C_HINT, &format!("Tangents: {}", tan_str));

        for (i, &(px, py)) in self.pts.iter().enumerate() {
            let wy = GY + 60 + i as i32 * 80;
            let col = PT_COLS[i];
            let sel_m = if i == self.sel { " <" } else { "" };
            text(PANEL_X + 10, wy, col, &format!("{}{}", PT_NAMES[i], sel_m));
            text(PANEL_X + 10, wy + 20, C_HINT, &format!("x: {}", px));
            text(PANEL_X + 10, wy + 40, C_HINT, &format!("y: {}", py));
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        let (x, y) = self.pts[self.sel];
        let clamp_x = |v: i32| v.clamp(10, CANVAS_W - 10);
        let clamp_y = |v: i32| v.clamp(GY + 10, H - 10);
        match line {
            "\x1b[A" => { self.pts[self.sel] = (x, clamp_y(y - 5)); }
            "\x1b[B" => { self.pts[self.sel] = (x, clamp_y(y + 5)); }
            "\x1b[C" => { self.pts[self.sel] = (clamp_x(x + 5), y); }
            "\x1b[D" => { self.pts[self.sel] = (clamp_x(x - 5), y); }
            "\t"     => { self.sel = (self.sel + 1) % 4; }
            "t" | "T" => { self.show_tan = !self.show_tan; }
            "r" | "R" => {
                self.pts = [(150, 500), (250, 200), (480, 560), (640, 260)];
                self.sel = 0;
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
