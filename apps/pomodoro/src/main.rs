use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 440;
const H: u32 = 380;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_OK: u32     = 0x3FB950FF;
const C_BREAK: u32  = 0x58A6FFFF;
const C_WORK: u32   = 0xFF7B72FF;

const WORK_SECS:        u64 = 25 * 60;
const SHORT_BREAK_SECS: u64 = 5  * 60;
const LONG_BREAK_SECS:  u64 = 15 * 60;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

#[derive(Clone, Copy, PartialEq)]
enum Phase { Work, ShortBreak, LongBreak }

impl Phase {
    fn label(self) -> &'static str {
        match self {
            Phase::Work       => "Focus — Work",
            Phase::ShortBreak => "Short Break",
            Phase::LongBreak  => "Long Break",
        }
    }
    fn duration(self) -> u64 {
        match self {
            Phase::Work       => WORK_SECS,
            Phase::ShortBreak => SHORT_BREAK_SECS,
            Phase::LongBreak  => LONG_BREAK_SECS,
        }
    }
    fn color(self) -> u32 {
        match self {
            Phase::Work       => C_WORK,
            Phase::ShortBreak => C_BREAK,
            Phase::LongBreak  => C_ACCENT,
        }
    }
}

struct Timer {
    phase:         Phase,
    pomo_count:    u32,
    running:       bool,
    elapsed_secs:  u64,   // elapsed within current phase
    last_instant:  Option<Instant>,
}

impl Timer {
    fn new() -> Self {
        Timer { phase: Phase::Work, pomo_count: 0, running: false, elapsed_secs: 0, last_instant: None }
    }

    fn tick(&mut self) {
        if !self.running { return; }
        if let Some(inst) = self.last_instant {
            let delta = inst.elapsed().as_secs();
            if delta > 0 {
                self.elapsed_secs += delta;
                self.last_instant = Some(Instant::now());
                // Auto-advance if phase complete
                if self.elapsed_secs >= self.phase.duration() {
                    self.advance();
                }
            }
        }
    }

    fn advance(&mut self) {
        match self.phase {
            Phase::Work => {
                self.pomo_count += 1;
                if self.pomo_count % 4 == 0 {
                    self.phase = Phase::LongBreak;
                } else {
                    self.phase = Phase::ShortBreak;
                }
            }
            Phase::ShortBreak | Phase::LongBreak => {
                self.phase = Phase::Work;
            }
        }
        self.elapsed_secs = 0;
        self.last_instant = if self.running { Some(Instant::now()) } else { None };
    }

    fn toggle_running(&mut self) {
        self.running = !self.running;
        self.last_instant = if self.running { Some(Instant::now()) } else { None };
    }

    fn reset(&mut self) {
        self.elapsed_secs = 0;
        self.last_instant = if self.running { Some(Instant::now()) } else { None };
    }

    fn remaining(&self) -> u64 {
        self.phase.duration().saturating_sub(self.elapsed_secs)
    }
}

fn draw(t: &Timer) {
    fill(0, 0, W, H, C_BG);
    fill(0, 34, W, 1, C_BORDER);
    text(20, 14, C_ACCENT, "Pomodoro Timer");

    let phase_color = t.phase.color();
    let phase_label = t.phase.label();

    // Phase label centered
    let lw = phase_label.len() as u32 * 8;
    let lx = (W - lw) / 2;
    text(lx, 50, phase_color, phase_label);

    // Countdown large — centered
    let rem = t.remaining();
    let mm  = rem / 60;
    let ss  = rem % 60;
    let countdown = format!("{mm:02}:{ss:02}");
    let cw = countdown.len() as u32 * 16;
    let cx = (W - cw) / 2;
    println!("VYOMA_DRAW:draw_text:{cx},90,{phase_color:#010x},l,{countdown}");

    // Running / paused indicator
    let state_str = if t.running { "▶ running" } else { "⏸ paused" };
    let sw = state_str.len() as u32 * 8;
    text((W - sw) / 2, 155, C_DIM, state_str);

    // Progress bar
    let bar_x  = 40u32;
    let bar_y  = 185u32;
    let bar_w  = W - 80;
    let bar_h  = 16u32;
    let elapsed_ratio = if t.phase.duration() > 0 {
        (t.elapsed_secs.min(t.phase.duration()) * bar_w as u64 / t.phase.duration()) as u32
    } else { 0 };
    fill(bar_x, bar_y, bar_w, bar_h, 0x21262DFF);
    if elapsed_ratio > 0 { fill(bar_x, bar_y, elapsed_ratio, bar_h, phase_color); }
    fill(bar_x, bar_y + bar_h, bar_w, 1, C_BORDER);

    // Pomodoro count dots
    let dot_y = 220u32;
    let pomo_label = format!("Pomodoro #{}", t.pomo_count + 1);
    let pw = pomo_label.len() as u32 * 8;
    text((W - pw) / 2, dot_y, C_DIM, &pomo_label);

    // Dot row showing completed pomodoros (up to 8)
    let dot_row_y = dot_y + 20;
    let dot_size = 14u32;
    let dot_gap  = 6u32;
    let total_dots = 4u32; // one set of 4
    let set_w = total_dots * (dot_size + dot_gap) - dot_gap;
    let dot_start = (W - set_w) / 2;
    for i in 0..4u32 {
        let dx = dot_start + i * (dot_size + dot_gap);
        let completed = t.pomo_count > i || (t.pomo_count % 4 > i);
        let dc = if completed { phase_color } else { C_HINT };
        fill(dx, dot_row_y, dot_size, dot_size, dc);
    }

    fill(0, H - 30, W, 1, C_BORDER);
    text(20, H - 18, C_HINT, "Space: start/pause   n: next phase   r: reset   Ctrl+C: quit");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut timer = Timer::new();

    draw(&timer);

    // Request ticks
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:pong") || raw.starts_with("REPLY:ping") {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }

        timer.tick();

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            " " => {
                timer.toggle_running();
            }
            "n" | "N" => {
                timer.advance();
            }
            "r" | "R" => {
                timer.reset();
            }
            _ => {}
        }

        draw(&timer);
    }
}
