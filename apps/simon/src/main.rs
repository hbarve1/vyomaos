use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 700;
const HEADER_H: u32 = 48;

// Color quadrant geometry
const PAD: u32 = 80;
const GAP: u32 = 12;
const SZ: u32 = (W - PAD * 2 - GAP) / 2; // ~264 px per quadrant

const QX0: u32 = PAD;
const QX1: u32 = PAD + SZ + GAP;
const QY0: u32 = HEADER_H + 20;
const QY1: u32 = HEADER_H + 20 + SZ + GAP;

// Colors: dim / bright for each of 4 buttons
const DIM: [u32; 4]    = [0x5A1010FF, 0x105A10FF, 0x10105AFF, 0x5A5A10FF];
const BRIGHT: [u32; 4] = [0xFF3B30FF, 0x30D158FF, 0x0A84FFFF, 0xFFD60AFF];
const LABEL: [&str; 4] = ["R", "G", "B", "Y"];
const KEY:   [&str; 4] = ["r/R", "g/G", "b/B", "y/Y"];

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

// quad x,y for index 0..4 (TL=0/red, TR=1/green, BL=2/blue, BR=3/yellow)
fn quad_pos(i: usize) -> (u32, u32) {
    match i {
        0 => (QX0, QY0),
        1 => (QX1, QY0),
        2 => (QX0, QY1),
        _ => (QX1, QY1),
    }
}

#[derive(PartialEq)]
enum Phase {
    Idle,
    Showing,   // animate sequence; flash_idx = current step, flash_tick counts sub-ticks
    Waiting,   // player inputs
    GameOver,
}

struct App {
    seq:        Vec<usize>,   // color indices 0..4
    input_pos:  usize,        // how many player inputs correct so far
    phase:      Phase,
    flash_idx:  usize,        // which seq item we're showing
    flash_on:   bool,         // bright or dim phase
    flash_tick: u32,          // ticks in current flash phase
    seed:       u64,
    score:      u32,
    best:       u32,
    lit:        Option<usize>, // which button is visually lit (player press feedback)
}

impl App {
    fn new(seed: u64) -> Self {
        App {
            seq: Vec::new(),
            input_pos: 0,
            phase: Phase::Idle,
            flash_idx: 0,
            flash_on: false,
            flash_tick: 0,
            seed,
            score: 0,
            best: 0,
            lit: None,
        }
    }

    fn start(&mut self) {
        self.seq.clear();
        self.input_pos = 0;
        self.score = 0;
        self.add_step();
        self.begin_show();
    }

    fn add_step(&mut self) {
        self.seed = lcg(self.seed);
        let c = (self.seed >> 33) as usize % 4;
        self.seq.push(c);
    }

    fn begin_show(&mut self) {
        self.phase = Phase::Showing;
        self.flash_idx = 0;
        self.flash_on = true;
        self.flash_tick = 0;
        self.input_pos = 0;
        self.lit = Some(self.seq[0]);
    }

    fn tick(&mut self) {
        if self.phase != Phase::Showing { return; }
        self.flash_tick += 1;
        let on_ticks  = 6;
        let off_ticks = 4;
        let pause_ticks = 3;
        if self.flash_on {
            if self.flash_tick >= on_ticks {
                self.flash_on = false;
                self.flash_tick = 0;
                self.lit = None;
            }
        } else {
            if self.flash_tick >= off_ticks {
                self.flash_idx += 1;
                if self.flash_idx >= self.seq.len() {
                    // pause before handing off to player
                    if self.flash_tick >= off_ticks + pause_ticks {
                        self.phase = Phase::Waiting;
                        self.lit = None;
                    }
                } else {
                    self.flash_on = true;
                    self.flash_tick = 0;
                    self.lit = Some(self.seq[self.flash_idx]);
                }
            }
        }
    }

    fn player_press(&mut self, c: usize) {
        if self.phase != Phase::Waiting { return; }
        self.lit = Some(c);
        if self.seq[self.input_pos] == c {
            self.input_pos += 1;
            if self.input_pos == self.seq.len() {
                self.score += 1;
                if self.score > self.best { self.best = self.score; }
                self.add_step();
                self.begin_show();
            }
        } else {
            self.phase = Phase::GameOver;
        }
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Simon Says");
    text(160, 16, C_HINT, &format!("Score: {}  Best: {}", app.score, app.best));
    text(400, 16, C_HINT, "R/G/B/Y:press  N:new game");

    // Draw 4 colored quadrants
    for i in 0..4 {
        let (qx, qy) = quad_pos(i);
        let lit = app.lit == Some(i);
        let color = if lit { BRIGHT[i] } else { DIM[i] };
        fill(qx, qy, SZ, SZ, color);
        border(qx, qy, SZ, SZ, if lit { BRIGHT[i] } else { C_BORDER });
        // Key hint
        let lx = qx + SZ / 2 - 8;
        let ly = qy + SZ / 2 - 8;
        text_l(lx, ly, if lit { C_BG } else { 0xFFFFFF44 }, LABEL[i]);
        text(qx + 4, qy + SZ - 20, C_HINT, KEY[i]);
    }

    // Status line
    let status_y = QY1 + SZ + GAP + 16;
    match app.phase {
        Phase::Idle => {
            text(PAD, status_y, C_HINT, "Press N to start a new game");
        }
        Phase::Showing => {
            let step_txt = format!("Watch: step {}/{}", app.flash_idx + 1, app.seq.len());
            text(PAD, status_y, C_TEXT, &step_txt);
        }
        Phase::Waiting => {
            let step_txt = format!("Your turn: input {}/{}", app.input_pos + 1, app.seq.len());
            text(PAD, status_y, C_GREEN, &step_txt);
        }
        Phase::GameOver => {}
    }

    // Game Over overlay
    if app.phase == Phase::GameOver {
        let bx = (W - 320) / 2;
        let by = (H - 100) / 2;
        fill(bx, by, 320, 100, C_HEADER);
        border(bx, by, 320, 100, C_RED);
        text(bx + 70, by + 16, C_RED, "Game Over!");
        text(bx + 24, by + 44, C_HINT, &format!("Score: {}  Best: {}  N=new", app.score, app.best));
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut seed: u64 = 0xAB12CD34EF567890;
    let mut app = App::new(seed);

    println!("@supervisor: raise simon");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") {
            // ping-pong tick
            app.tick();
            if app.phase == Phase::Showing {
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            draw(&app);
            continue;
        }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "n" | "N" => {
                seed = lcg(seed);
                app = App::new(seed);
                app.start();
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            "r" | "R" => { app.player_press(0); }
            "g" | "G" => { app.player_press(1); }
            "b" | "B" => { app.player_press(2); }
            "y" | "Y" => { app.player_press(3); }
            _ => {}
        }

        // Start showing animation ping-pong when entering Showing phase
        if app.phase == Phase::Showing {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }

        draw(&app);
    }
}
