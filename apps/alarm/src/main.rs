// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 640;
const H: i32 = 480;

const C_BG: u32       = 0x0D1117FF;
const C_HEADER: u32   = 0x21262DFF;
const C_BORDER: u32   = 0x30363DFF;
const C_TEXT: u32     = 0xE6EDF3FF;
const C_HINT: u32     = 0x6E7681FF;
const C_ORANGE: u32   = 0xFFA657FF;
const C_SEL: u32      = 0x58A6FFFF;
const C_GREEN: u32    = 0x3FB950FF;
const C_RED: u32      = 0xFF7B72FF;
const C_YELLOW: u32   = 0xD29922FF;
const C_CARD: u32     = 0x161B22FF;

const DAYS: [&str; 7]   = ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"];
const MONTHS: [&str; 12] = ["Jan","Feb","Mar","Apr","May","Jun",
                              "Jul","Aug","Sep","Oct","Nov","Dec"];

// 7-segment layout: each digit is 5 wide × 7 tall cells (each cell = SEG_CELL px)
// Segments: a=top, b=top-right, c=bot-right, d=bot, e=bot-left, f=top-left, g=mid
// bits: a b c d e f g
const SEGS: [u8; 10] = [
    0b1110111, // 0
    0b0010010, // 1
    0b1101101, // 2
    0b1111001, // 3
    0b0111010, // 4 (corrected)
    0b1011011, // 5
    0b1011111, // 6
    0b1110010, // 7
    0b1111111, // 8
    0b1111011, // 9
];

const SEG_W: i32 = 10; // thickness
const SEG_LEN: i32 = 36; // length of each segment bar

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c);
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s);
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn draw_seg_digit(ox: i32, oy: i32, d: u8, color: u32) {
    let s = SEGS[d as usize];
    let x0 = ox;
    let x1 = ox + SEG_W;
    let x2 = ox + SEG_W + SEG_LEN;
    let y0 = oy;
    let y1 = oy + SEG_W;
    let y2 = oy + SEG_W + SEG_LEN;
    let y3 = oy + SEG_W * 2 + SEG_LEN;
    let y4 = oy + SEG_W * 2 + SEG_LEN * 2;

    // a: top horizontal
    if s & 0b1000000 != 0 { fill(x1, y0, SEG_LEN, SEG_W, color); }
    // b: top-right vertical
    if s & 0b0100000 != 0 { fill(x2, y1, SEG_W, SEG_LEN, color); }
    // c: bot-right vertical
    if s & 0b0010000 != 0 { fill(x2, y3, SEG_W, SEG_LEN, color); }
    // d: bottom horizontal
    if s & 0b0001000 != 0 { fill(x1, y4, SEG_LEN, SEG_W, color); }
    // e: bot-left vertical
    if s & 0b0000100 != 0 { fill(x0, y3, SEG_W, SEG_LEN, color); }
    // f: top-left vertical
    if s & 0b0000010 != 0 { fill(x0, y1, SEG_W, SEG_LEN, color); }
    // g: middle horizontal
    if s & 0b0000001 != 0 { fill(x1, y2, SEG_LEN, SEG_W, color); }
}

// digit total width = SEG_W*2 + SEG_LEN = 10+10+36 = 56, height = SEG_W*2 + SEG_LEN*2 = 92
const DIGIT_W: i32 = 56;
const DIGIT_H: i32 = 92;
const DIGIT_GAP: i32 = 8;
const COLON_W: i32 = 12;

fn draw_colon(ox: i32, oy: i32, c: u32) {
    let cy = DIGIT_H / 3;
    fill(ox + 2, oy + cy, 8, 8, c);
    fill(ox + 2, oy + cy * 2, 8, 8, c);
}

fn draw_time(ticks: u64, flash: bool) {
    let secs = ticks % 60;
    let mins = (ticks / 60) % 60;
    let hrs  = (ticks / 3600) % 24;
    let color = if flash { C_ORANGE } else { C_SEL };

    let total_w = DIGIT_W * 6 + COLON_W * 2 + DIGIT_GAP * 7;
    let ox = (W - total_w) / 2;
    let oy = 90;

    // background clear
    fill(ox - 4, oy - 4, total_w + 8, DIGIT_H + 8, C_BG);

    let mut cx = ox;
    let digits = [
        (hrs / 10) as u8, (hrs % 10) as u8,
        (mins / 10) as u8, (mins % 10) as u8,
        (secs / 10) as u8, (secs % 10) as u8,
    ];

    for i in 0..6 {
        draw_seg_digit(cx, oy, digits[i], color);
        cx += DIGIT_W + DIGIT_GAP;
        if i == 1 || i == 3 {
            draw_colon(cx, oy, color);
            cx += COLON_W + DIGIT_GAP;
        }
    }
}

struct Alarm {
    hh: u8,
    mm: u8,
    label: String,
    enabled: bool,
    fired: bool,
}

#[derive(PartialEq)]
enum Mode {
    Normal,
    AddHH,
    AddMM,
    AddLabel,
}

struct App {
    ticks: u64,
    alarms: Vec<Alarm>,
    selected: usize,
    mode: Mode,
    input_hh: String,
    input_mm: String,
    input_label: String,
    flash_ticks: u8,
    seed: u64,
}

impl App {
    fn new() -> Self {
        App {
            ticks: 0,
            alarms: Vec::new(),
            selected: 0,
            mode: Mode::Normal,
            input_hh: String::new(),
            input_mm: String::new(),
            input_label: String::new(),
            flash_ticks: 0,
            seed: 0xA1A2_A3A4_A5A6_A7A8u64,
        }
    }

    fn draw(&self) {
        let flash = self.flash_ticks > 0 && self.flash_ticks % 2 == 0;
        let bg = if flash { 0x3F1A00FF } else { C_BG };
        fill(0, 0, W, H, bg);

        // Header
        fill(0, 0, W, 30, C_HEADER);
        text(8, 7, C_TEXT, "Alarm Clock");
        text(W - 160, 7, C_HINT, "A=add  D=del  E=toggle");

        // Date line
        let day_idx = (4 + (self.ticks / 86400)) % 7; // start Wed (2026-05-21)
        let total_days = self.ticks / 86400;
        let (yr, mo, da) = approx_date(2026, 5, 21, total_days);
        let date_str = format!("{} {:02} {} {}", DAYS[day_idx as usize], da, MONTHS[mo as usize - 1], yr);
        let dw = date_str.len() as i32 * 8;
        text((W - dw) / 2, 40, C_HINT, &date_str);

        // Clock
        draw_time(self.ticks, flash);

        // Progress bar (minutes in current hour)
        let min_in_hr = (self.ticks / 60) % 60;
        let bar_x = 60;
        let bar_y = 196;
        let bar_w = W - 120;
        let bar_h = 8;
        fill(bar_x, bar_y, bar_w, bar_h, C_CARD);
        border(bar_x, bar_y, bar_w, bar_h, C_BORDER);
        let filled = (min_in_hr as i32 * bar_w) / 60;
        if filled > 0 { fill(bar_x, bar_y, filled, bar_h, C_SEL); }
        text(bar_x, bar_y + 12, C_HINT, "min");

        // Alarm list panel
        fill(30, 220, W - 60, H - 240, C_CARD);
        border(30, 220, W - 60, H - 240, C_BORDER);
        text(38, 226, C_HINT, "ALARMS");

        for (i, a) in self.alarms.iter().enumerate() {
            let ay = 246 + i as i32 * 36;
            let sel = i == self.selected;
            if sel {
                fill(32, ay - 2, W - 64, 34, C_HEADER);
                border(32, ay - 2, W - 64, 34, C_SEL);
            }
            let time_str = format!("{:02}:{:02}", a.hh, a.mm);
            let tc = if !a.enabled { C_HINT } else if sel { C_SEL } else { C_TEXT };
            text(44, ay + 8, tc, &time_str);
            text(110, ay + 8, tc, &a.label);
            let en_str = if a.enabled { "[ON] " } else { "[OFF]" };
            let ec = if a.enabled { C_GREEN } else { C_RED };
            text(W - 90, ay + 8, ec, en_str);
        }

        if self.alarms.is_empty() {
            text(44, 260, C_HINT, "No alarms — press A to add");
        }

        // Add form
        match self.mode {
            Mode::AddHH => {
                fill(80, 380, 480, 60, C_HEADER);
                border(80, 380, 480, 60, C_SEL);
                text(90, 390, C_HINT, "Hour (00-23):");
                text(250, 390, C_TEXT, &self.input_hh);
                text(90, 408, C_HINT, "Enter to confirm");
            }
            Mode::AddMM => {
                fill(80, 380, 480, 60, C_HEADER);
                border(80, 380, 480, 60, C_SEL);
                let hh_str = format!("{}:{{}}", self.input_hh);
                text(90, 390, C_HINT, &format!("Minute (00-59) for {}:", self.input_hh));
                text(400, 390, C_TEXT, &self.input_mm);
                let _ = hh_str;
                text(90, 408, C_HINT, "Enter to confirm");
            }
            Mode::AddLabel => {
                fill(80, 380, 480, 60, C_HEADER);
                border(80, 380, 480, 60, C_SEL);
                let hhmm = format!("{:0>2}:{:0>2}", self.input_hh, self.input_mm);
                text(90, 390, C_HINT, &format!("Label for {}:", hhmm));
                text(250, 390, C_TEXT, &self.input_label);
                text(90, 408, C_HINT, "Enter to confirm  Esc=cancel");
            }
            Mode::Normal => {
                text(30, H - 18, C_HINT, "↑↓=nav  A=add  D=delete  E=toggle enabled");
            }
        }

        flush();
    }

    fn check_alarms(&mut self) {
        let hh = ((self.ticks / 3600) % 24) as u8;
        let mm = ((self.ticks / 60) % 60) as u8;
        let ss = (self.ticks % 60) as u8;
        if ss != 0 { return; }
        for a in &mut self.alarms {
            if a.enabled && !a.fired && a.hh == hh && a.mm == mm {
                a.fired = true;
                self.flash_ticks = 10;
                println!("@supervisor: notify Alarm {} fired!", a.label);
            }
        }
        // reset fired flag when minute changes
        if mm == 0 {
            for a in &mut self.alarms { a.fired = false; }
        }
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" {
            self.ticks += 1;
            if self.flash_ticks > 0 { self.flash_ticks -= 1; }
            self.check_alarms();
            self.draw();
            println!("@supervisor: ping");
            return;
        }

        match self.mode {
            Mode::Normal => match line {
                "\x1b[A" => {
                    if self.selected > 0 { self.selected -= 1; }
                    self.draw();
                }
                "\x1b[B" => {
                    if !self.alarms.is_empty() && self.selected < self.alarms.len() - 1 {
                        self.selected += 1;
                    }
                    self.draw();
                }
                "a" | "A" => {
                    if self.alarms.len() < 5 {
                        self.input_hh.clear();
                        self.input_mm.clear();
                        self.input_label.clear();
                        self.mode = Mode::AddHH;
                        self.draw();
                    }
                }
                "d" | "D" => {
                    if !self.alarms.is_empty() {
                        self.alarms.remove(self.selected);
                        if self.selected > 0 && self.selected >= self.alarms.len() {
                            self.selected -= 1;
                        }
                        self.draw();
                    }
                }
                "e" | "E" => {
                    if let Some(a) = self.alarms.get_mut(self.selected) {
                        a.enabled = !a.enabled;
                    }
                    self.draw();
                }
                _ => {}
            },
            Mode::AddHH => {
                if line == "" {
                    if let Ok(n) = self.input_hh.parse::<u8>() {
                        if n < 24 { self.mode = Mode::AddMM; self.draw(); }
                    }
                } else if line == "\x7f" {
                    self.input_hh.pop();
                    self.draw();
                } else if line == "\x1b" {
                    self.mode = Mode::Normal; self.draw();
                } else if line.len() == 1 && line.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    if self.input_hh.len() < 2 { self.input_hh.push_str(line); }
                    self.draw();
                }
            }
            Mode::AddMM => {
                if line == "" {
                    if let Ok(n) = self.input_mm.parse::<u8>() {
                        if n < 60 { self.mode = Mode::AddLabel; self.draw(); }
                    }
                } else if line == "\x7f" {
                    self.input_mm.pop();
                    self.draw();
                } else if line == "\x1b" {
                    self.mode = Mode::Normal; self.draw();
                } else if line.len() == 1 && line.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) {
                    if self.input_mm.len() < 2 { self.input_mm.push_str(line); }
                    self.draw();
                }
            }
            Mode::AddLabel => {
                if line == "" {
                    let hh = self.input_hh.parse::<u8>().unwrap_or(0);
                    let mm = self.input_mm.parse::<u8>().unwrap_or(0);
                    let label = if self.input_label.is_empty() {
                        format!("{:02}:{:02}", hh, mm)
                    } else {
                        self.input_label.clone()
                    };
                    self.alarms.push(Alarm { hh, mm, label, enabled: true, fired: false });
                    self.mode = Mode::Normal;
                    self.draw();
                } else if line == "\x7f" {
                    self.input_label.pop();
                    self.draw();
                } else if line == "\x1b" {
                    self.mode = Mode::Normal; self.draw();
                } else if line.len() == 1 {
                    if self.input_label.len() < 20 { self.input_label.push_str(line); }
                    self.draw();
                }
            }
        }
    }
}

fn approx_date(base_yr: u32, base_mo: u32, base_da: u32, days: u64) -> (u32, u32, u32) {
    let mut yr = base_yr;
    let mut mo = base_mo;
    let mut da = base_da + days as u32;
    loop {
        let days_in = days_in_month(yr, mo);
        if da <= days_in { break; }
        da -= days_in;
        mo += 1;
        if mo > 12 { mo = 1; yr += 1; }
    }
    (yr, mo, da)
}

fn days_in_month(yr: u32, mo: u32) -> u32 {
    match mo {
        1|3|5|7|8|10|12 => 31,
        4|6|9|11 => 30,
        2 => if yr % 4 == 0 { 29 } else { 28 },
        _ => 30,
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    // Initial draw + start tick loop
    app.draw();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
