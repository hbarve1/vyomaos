use std::io::{self, BufRead, Write};

const W: i32 = 640;
const H: i32 = 400;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;

// Themes: (lit_color, dim_color)
const THEMES: [(u32, u32); 5] = [
    (0x3FB950FF, 0x0E2A12FF),
    (0x58A6FFFF, 0x0D1E3AFF),
    (0xFFA657FF, 0x2A1A08FF),
    (0xFF7B72FF, 0x2A0D0AFF),
    (0xBC8CFFFF, 0x1E0F2EFF),
];

const THEME_NAMES: [&str; 5] = ["Green", "Blue", "Orange", "Red", "Purple"];

// 5×7 LED font, bit4=col0(left)..bit0=col4(right)
const DIGITS: [[u8; 7]; 10] = [
    [14, 17, 17, 17, 17, 17, 14],
    [ 4, 12,  4,  4,  4,  4, 14],
    [14, 17,  1,  6,  8, 16, 31],
    [14,  1,  1, 14,  1,  1, 14],
    [ 2,  6, 10, 18, 31,  2,  2],
    [31, 16, 16, 30,  1,  1, 30],
    [ 6,  8, 16, 30, 17, 17, 14],
    [31,  1,  2,  4,  8,  8,  8],
    [14, 17, 17, 14, 17, 17, 14],
    [14, 17, 17, 15,  1,  1, 14],
];

const DIGIT_X: [i32; 6] = [93, 159, 252, 318, 411, 477];
const COLON_X: [i32; 2] = [231, 390];
const CLOCK_Y: i32 = 140;
const CELL: i32 = 12;
const STRIDE: i32 = 13;

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

fn draw_digit(dx: i32, dy: i32, d: usize, lit: u32, dim: u32) {
    let rows = &DIGITS[d];
    for row in 0..7usize {
        let bits = rows[row];
        for col in 0..5usize {
            let on = (bits >> (4 - col)) & 1 == 1;
            fill(dx + col as i32 * STRIDE, dy + row as i32 * STRIDE, CELL, CELL,
                 if on { lit } else { dim });
        }
    }
}

fn draw_colon(cx: i32, cy: i32, c: u32) {
    fill(cx, cy + 20, CELL, CELL, c);
    fill(cx, cy + 58, CELL, CELL, c);
}

fn sim_date(sim_secs: u64) -> String {
    let total_days = (sim_secs / 86400) as u32;
    let dow = (4 + total_days) % 7; // 2026-01-01 = Thursday = 4
    let months = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
    let days_in_month = [31u32, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let dow_names = ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"];

    let mut year = 2026u32;
    let mut remaining = total_days;
    while remaining >= 365 {
        remaining -= 365;
        year += 1;
    }
    let mut month = 0usize;
    loop {
        if remaining < days_in_month[month] { break; }
        remaining -= days_in_month[month];
        month += 1;
        if month == 12 { month = 0; }
    }
    let day = remaining + 1;
    format!("{} {} {:02}, {}", dow_names[dow as usize], months[month], day, year)
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
        text(12, 8, C_TEXT, "Pixel Clock");
        let theme_label = THEME_NAMES[self.theme];
        let theme_x = W / 2 - theme_label.len() as i32 * 4;
        text(theme_x, 8, C_HINT, theme_label);
        text(W - 280, 8, C_HINT, "T=theme  R=reset  Q=quit");

        let (lit, dim) = THEMES[self.theme];
        let hh = ((self.sim_secs % 86400) / 3600) as usize;
        let mm = ((self.sim_secs % 3600) / 60) as usize;
        let ss = (self.sim_secs % 60) as usize;

        draw_digit(DIGIT_X[0], CLOCK_Y, hh / 10, lit, dim);
        draw_digit(DIGIT_X[1], CLOCK_Y, hh % 10, lit, dim);
        draw_digit(DIGIT_X[2], CLOCK_Y, mm / 10, lit, dim);
        draw_digit(DIGIT_X[3], CLOCK_Y, mm % 10, lit, dim);
        draw_digit(DIGIT_X[4], CLOCK_Y, ss / 10, lit, dim);
        draw_digit(DIGIT_X[5], CLOCK_Y, ss % 10, lit, dim);

        let colon_c = if ss % 2 == 0 { lit } else { dim };
        draw_colon(COLON_X[0], CLOCK_Y, colon_c);
        draw_colon(COLON_X[1], CLOCK_Y, colon_c);

        let date_str = sim_date(self.sim_secs);
        let date_x = W / 2 - date_str.len() as i32 * 4;
        text(date_x, CLOCK_Y + 7 * STRIDE + 20, C_HINT, &date_str);

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("{:02}:{:02}:{:02}  |  {}  |  Tick {}", hh, mm, ss, theme_label, self.ticks));

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
