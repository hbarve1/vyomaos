use std::io::{self, BufRead, Write};

const W: u32 = 720;
const H: u32 = 560;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_RED: u32     = 0xFF7B72FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn text_l(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},l,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

#[derive(PartialEq)]
enum State { Setup, Running, Paused, Done }

struct Timer {
    // Setup: digit input cycling through HH MM SS positions
    input:    [u8; 6],  // digits: H H M M S S
    pos:      usize,    // current input position 0..6
    // Runtime
    total:    u64,      // total seconds set
    remain:   u64,      // remaining seconds
    state:    State,
    flash:    u8,       // flash counter for Done state
}

impl Timer {
    fn new() -> Self {
        Timer {
            input:  [0; 6],
            pos:    0,
            total:  0,
            remain: 0,
            state:  State::Setup,
            flash:  0,
        }
    }

    fn input_to_secs(&self) -> u64 {
        let hh = self.input[0] as u64 * 10 + self.input[1] as u64;
        let mm = self.input[2] as u64 * 10 + self.input[3] as u64;
        let ss = self.input[4] as u64 * 10 + self.input[5] as u64;
        hh * 3600 + mm * 60 + ss
    }

    fn push_digit(&mut self, d: u8) {
        if self.state != State::Setup { return; }
        // Shift all digits left, insert new at end
        for i in 0..5 { self.input[i] = self.input[i + 1]; }
        self.input[5] = d;
    }

    fn start(&mut self) {
        let secs = self.input_to_secs();
        if secs == 0 { return; }
        self.total  = secs;
        self.remain = secs;
        self.state  = State::Running;
    }

    fn reset(&mut self) {
        self.input  = [0; 6];
        self.total  = 0;
        self.remain = 0;
        self.state  = State::Setup;
        self.flash  = 0;
    }

    fn tick(&mut self) {
        if self.state != State::Running { return; }
        if self.remain == 0 {
            self.state = State::Done;
            self.flash = 6;
            return;
        }
        self.remain -= 1;
        if self.remain == 0 {
            self.state = State::Done;
            self.flash = 6;
        }
    }

    fn hms(&self) -> (u64, u64, u64) {
        let r = self.remain;
        (r / 3600, (r % 3600) / 60, r % 60)
    }

    fn hms_setup(&self) -> (u64, u64, u64) {
        let hh = self.input[0] as u64 * 10 + self.input[1] as u64;
        let mm = self.input[2] as u64 * 10 + self.input[3] as u64;
        let ss = self.input[4] as u64 * 10 + self.input[5] as u64;
        (hh, mm, ss)
    }
}

const DIGIT_X: u32 = 60;
const DIGIT_Y: u32 = 180;
const DIGIT_W: u32 = 48; // 'l' font ~16px wide per char × 3 chars = 48 for 2-digit
const SEP_W:   u32 = 24;
const BLOCK_GAP: u32 = 16;

// Positions: HH : MM : SS — total width = 3*96 + 2*24 + 4*16 = 288+48+64 = 400
// Center in W=720: start_x = (720-400)/2 = 160
const DX: u32 = 160;

fn draw_digit_block(x: u32, y: u32, val: u64, color: u32, cursor: bool) {
    let s = format!("{:02}", val);
    // Each 'l' char is ~16px wide, so 2 chars = 32px; add padding
    let bg = if cursor { 0x1F4068FF } else { 0x161B22FF };
    fill(x, y - 8, 80, 80, bg);
    if cursor {
        fill(x, y + 60, 80, 4, 0x58A6FFFF);
    }
    text_l(x + 8, y, color, &s);
}

fn draw_sep(x: u32, y: u32) {
    text_l(x, y, 0x30363DFF, ":");
}

fn draw(t: &Timer) {
    let flash_on = t.flash > 0 && t.flash % 2 == 0;
    let bg = if flash_on { 0x3D0000FF } else { C_BG };
    fill(0, 0, W, H, bg);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Countdown");

    let state_str = match t.state {
        State::Setup   => "Setup — type digits",
        State::Running => "Running",
        State::Paused  => "Paused",
        State::Done    => "TIME'S UP!",
    };
    let state_col = match t.state {
        State::Done => C_RED,
        State::Running => C_GREEN,
        State::Paused  => C_ORANGE,
        _ => C_HINT,
    };
    text(160, 16, state_col, state_str);
    text(420, 16, C_HINT, "Digits:set  Enter:start  Space:pause  R:reset");

    // Large digit display
    let (hh, mm, ss) = if t.state == State::Setup { t.hms_setup() } else { t.hms() };

    let color = match t.state {
        State::Done    => C_RED,
        State::Paused  => C_ORANGE,
        State::Running => C_GREEN,
        _              => C_TEXT,
    };

    // HH at DX, MM at DX+96+24, SS at DX+2*(96+24)
    let hx = DX;
    let mx = DX + 96 + 24;
    let sx = DX + 2 * (96 + 24);

    let setup = t.state == State::Setup;
    draw_digit_block(hx, DIGIT_Y, hh, color, setup && t.input[0] == 0 && t.input[1] == 0);
    draw_sep(hx + 80, DIGIT_Y);
    draw_digit_block(mx, DIGIT_Y, mm, color, setup && t.input[2] == 0 && t.input[3] == 0 && (t.input[0] != 0 || t.input[1] != 0));
    draw_sep(mx + 80, DIGIT_Y);
    draw_digit_block(sx, DIGIT_Y, ss, color, setup);

    // Progress bar
    if t.total > 0 && t.state != State::Setup {
        let bar_x = 40u32;
        let bar_y = DIGIT_Y + 100;
        let bar_w = W - 80;
        let bar_h = 12u32;
        fill(bar_x, bar_y, bar_w, bar_h, 0x21262DFF);
        border(bar_x, bar_y, bar_w, bar_h, C_BORDER);
        let elapsed = t.total.saturating_sub(t.remain);
        let filled = bar_w * elapsed as u32 / t.total as u32;
        let bar_col = if t.state == State::Done { C_RED } else { C_GREEN };
        fill(bar_x, bar_y, filled, bar_h, bar_col);

        // Time label
        let elapsed_str = format!("{}:{:02}:{:02} elapsed", elapsed / 3600, (elapsed % 3600) / 60, elapsed % 60);
        let remain_str  = format!("{}:{:02}:{:02} left", hh, mm, ss);
        text(bar_x, bar_y + 16, C_HINT, &elapsed_str);
        text(bar_x + bar_w - 140, bar_y + 16, C_HINT, &remain_str);
    }

    // Setup hint
    if t.state == State::Setup {
        text((W - 280) / 2, DIGIT_Y + 100, C_HINT, "Type digits to set time — HHMMSS order");
        text((W - 200) / 2, DIGIT_Y + 118, C_HINT, "Press Enter to start");
    }

    // Done overlay
    if t.state == State::Done {
        let bx = (W - 280) / 2;
        let by = (H - 80) / 2 + 60;
        fill(bx, by, 280, 80, C_HEADER);
        border(bx, by, 280, 80, C_RED);
        text(bx + 60, by + 16, C_RED, "TIME'S UP!");
        text(bx + 60, by + 44, C_HINT, "R to reset");
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut timer = Timer::new();

    println!("@supervisor: raise countdown");
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&timer);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            timer.tick();
            if timer.flash > 0 { timer.flash -= 1; }
            draw(&timer);
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            " " => {
                match timer.state {
                    State::Running => timer.state = State::Paused,
                    State::Paused  => timer.state = State::Running,
                    _ => {}
                }
            }
            "\r" | "" => { timer.start(); }
            "r" | "R" => { timer.reset(); }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= b'0' && b <= b'9' { timer.push_digit(b - b'0'); }
            }
            _ => {}
        }
        draw(&timer);
    }
}
