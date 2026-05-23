use std::io::{self, BufRead, Write};

const W: i32 = 640;
const H: i32 = 480;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;

const THEMES: [(u32, u32); 5] = [
    (0x3FB950FF, 0x0E2A12FF),
    (0x58A6FFFF, 0x0D1E3AFF),
    (0xFFA657FF, 0x2A1A08FF),
    (0xFF7B72FF, 0x2A0D0AFF),
    (0xBC8CFFFF, 0x1E0F2EFF),
];

const THEME_NAMES: [&str; 5] = ["Green", "Blue", "Orange", "Red", "Purple"];

const CELL: i32   = 20;
const STRIDE: i32 = 24;
const GRID_X: i32 = 260;
const GRID_Y: i32 = 140;

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

struct App {
    sim_secs: u64,
    theme:    usize,
    ticks:    u64,
}

impl App {
    fn new() -> Self { App { sim_secs: 0, theme: 0, ticks: 0 } }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Binary Clock");
        let tname = THEME_NAMES[self.theme];
        text(W / 2 - tname.len() as i32 * 4, 8, C_HINT, tname);
        text(W - 280, 8, C_HINT, "T=theme  R=reset  Q=quit");

        let (lit, dim) = THEMES[self.theme];

        let hh = ((self.sim_secs % 86400) / 3600) as u32;
        let mm = ((self.sim_secs % 3600) / 60) as u32;
        let ss = (self.sim_secs % 60) as u32;

        let digits = [hh / 10, hh % 10, mm / 10, mm % 10, ss / 10, ss % 10];
        let col_labels = ["H1", "H2", "M1", "M2", "S1", "S2"];

        // Group labels (HH / MM / SS) centered above each pair
        let grp_data: [(&str, usize); 3] = [("HH", 0), ("MM", 2), ("SS", 4)];
        for &(grp, cs) in &grp_data {
            let gcx = GRID_X + cs as i32 * STRIDE + STRIDE + CELL / 2;
            text(gcx - grp.len() as i32 * 4, GRID_Y - 36, C_HINT, grp);
        }

        // Column labels
        for c in 0..6usize {
            let cx = GRID_X + c as i32 * STRIDE;
            text(cx + 2, GRID_Y - 18, C_HINT, col_labels[c]);
        }

        // Row labels (bit weights)
        let bit_labels = ["8", "4", "2", "1"];
        for r in 0..4usize {
            text(GRID_X - 24, GRID_Y + r as i32 * STRIDE + 4, C_HINT, bit_labels[r]);
        }

        // Group separators between H/M and M/S
        fill(GRID_X + 2 * STRIDE - 2, GRID_Y, 2, 4 * STRIDE, 0x30363DFF);
        fill(GRID_X + 4 * STRIDE - 2, GRID_Y, 2, 4 * STRIDE, 0x30363DFF);

        // Grid cells — bit3 is top row (most significant)
        for c in 0..6usize {
            let cx = GRID_X + c as i32 * STRIDE;
            let val = digits[c];
            for r in 0..4usize {
                let bit = 3 - r as u32;
                let on = (val >> bit) & 1 == 1;
                fill(cx, GRID_Y + r as i32 * STRIDE, CELL, CELL, if on { lit } else { dim });
            }
        }

        // Decimal time below grid
        let dty = GRID_Y + 4 * STRIDE + 20;
        let time_str = format!("{:02}:{:02}:{:02}", hh, mm, ss);
        text(W / 2 - time_str.len() as i32 * 4, dty, C_TEXT, &time_str);

        // Legend
        let ly = dty + 30;
        fill(W / 2 - 64, ly + 2, CELL, CELL, lit);
        text(W / 2 - 40, ly + 4, C_HINT, "= 1 (bit set)");
        fill(W / 2 + 60, ly + 2, CELL, CELL, dim);
        text(W / 2 + 84, ly + 4, C_HINT, "= 0 (bit clear)");

        // Info line
        let info_y = ly + 38;
        text(W / 2 - 160, info_y, C_HINT,
             "Each column = 4 bits (8,4,2,1)  |  Read top-to-bottom");

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("{:02}:{:02}:{:02}  |  {}  |  Tick {}", hh, mm, ss, tname, self.ticks));

        flush();
    }

    fn tick(&mut self) {
        self.ticks += 1;
        self.sim_secs += 1;
        self.draw();
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" { self.tick(); return; }
        match line {
            "t" | "T" => { self.theme = (self.theme + 1) % THEMES.len(); self.draw(); }
            "r" | "R" => { self.sim_secs = 0; self.ticks = 0; self.draw(); }
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
        if line == "REPLY:pong" {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }
    }
}
