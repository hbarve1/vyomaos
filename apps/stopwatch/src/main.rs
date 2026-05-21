use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 640;
const H: u32 = 480;
const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x161B22FF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn fmt_time(millis: u64) -> String {
    let h = millis / 3_600_000;
    let m = (millis / 60_000) % 60;
    let s = (millis / 1_000) % 60;
    let cs = (millis / 10) % 100;
    if h > 0 {
        format!("{:02}:{:02}:{:02}.{:02}", h, m, s, cs)
    } else {
        format!("{:02}:{:02}.{:02}", m, s, cs)
    }
}

struct State {
    running:     bool,
    accumulated: u64,   // ms before last start
    start:       Option<Instant>,
    laps:        Vec<u64>,
}

impl State {
    fn new() -> Self {
        State { running: false, accumulated: 0, start: None, laps: Vec::new() }
    }

    fn elapsed_ms(&self) -> u64 {
        let extra = self.start.as_ref().map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
        self.accumulated + extra
    }

    fn toggle(&mut self) {
        if self.running {
            let e = self.start.take().map(|s| s.elapsed().as_millis() as u64).unwrap_or(0);
            self.accumulated += e;
            self.running = false;
        } else {
            self.start = Some(Instant::now());
            self.running = true;
        }
    }

    fn reset(&mut self) {
        self.running = false;
        self.accumulated = 0;
        self.start = None;
        self.laps.clear();
    }

    fn lap(&mut self) {
        let ms = self.elapsed_ms();
        self.laps.push(ms);
        if self.laps.len() > 5 { self.laps.remove(0); }
    }
}

fn draw(s: &State) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, 36, C_HEADER);
    fill(0, 36, W, 1, C_BORDER);
    text(16, 10, C_TEXT, "Stopwatch");
    let status = if s.running { "RUNNING" } else { "STOPPED" };
    let sc = if s.running { C_GREEN } else { C_RED };
    text(W - 80, 10, sc, status);

    // Large time display
    let ms = s.elapsed_ms();
    let time_str = fmt_time(ms);
    let tw = time_str.len() as u32 * 24;
    let tx = (W - tw) / 2;
    let color = if s.running { C_TEXT } else { C_DIM };
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},l,{time_str}", tx, 80, color);

    // Status bar / buttons row
    fill(0, 160, W, 1, C_BORDER);
    let btns = &[
        ("Space: start/stop", if s.running { C_RED } else { C_GREEN }),
        ("r: reset",   C_DIM),
        ("l: lap",     C_YELLOW),
        ("Esc: close", C_HINT),
    ];
    let bw = (W - 32) / btns.len() as u32;
    for (i, &(label, color)) in btns.iter().enumerate() {
        let bx = 16 + i as u32 * bw;
        fill(bx, 170, bw - 8, 32, 0x21262DFF);
        border(bx, 170, bw - 8, 32, C_BORDER);
        let lw = label.len() as u32 * 8;
        text(bx + (bw - 8 - lw.min(bw - 8)) / 2, 180, color, label);
    }

    // Lap list
    if !s.laps.is_empty() {
        fill(0, 212, W, 1, C_BORDER);
        text(16, 220, C_HINT, "Lap times:");
        for (i, &lap_ms) in s.laps.iter().rev().enumerate() {
            let ly = 240 + i as u32 * 36;
            let lap_n = s.laps.len() - i;
            let lap_str = format!("Lap {:2}  {}", lap_n, fmt_time(lap_ms));
            let tc = if i == 0 { C_SEL } else { C_DIM };
            text(16, ly, tc, &lap_str);

            // Delta from previous lap
            if i + 1 < s.laps.len() {
                let prev = s.laps[s.laps.len() - i - 2];
                let delta = lap_ms.saturating_sub(prev);
                let delta_str = format!("+{}", fmt_time(delta));
                text(W - 200, ly, C_HINT, &delta_str);
            }
        }
    } else {
        text(W / 2 - 60, 240, C_HINT, "No laps recorded");
    }

    // Progress bar — visual only (wraps every minute)
    let bar_w = W - 32;
    let ms_in_minute = ms % 60_000;
    let filled = (ms_in_minute * bar_w as u64 / 60_000) as u32;
    fill(16, H - 24, bar_w, 12, 0x21262DFF);
    border(16, H - 24, bar_w, 12, C_BORDER);
    if filled > 0 { fill(17, H - 23, filled, 10, if s.running { C_GREEN } else { C_DIM }); }
    text(16, H - 40, C_HINT, "Elapsed (minute wrap):");

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut sw = State::new();

    println!("@supervisor: ping");
    println!("@supervisor: raise stopwatch");
    let _ = io::stdout().flush();

    draw(&sw);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            if sw.running { draw(&sw); }
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            " " => { sw.toggle(); draw(&sw); }
            "r" | "R" => { sw.reset(); draw(&sw); }
            "l" | "L" => { sw.lap(); draw(&sw); }
            _ => {}
        }
    }
}
