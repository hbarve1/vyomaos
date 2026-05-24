use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GY: i32 = 24;
const PLOT_W: i32 = 676;
const PANEL_X: i32 = 680;
const PLOT_TOP: i32 = GY + 40;
const PLOT_BOT: i32 = H - 20;

const C_HEADER: u32 = 0x21262DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_BG:     u32 = 0x0D1117FF;
const C_BORDER: u32 = 0x30363DFF;
const C_CARD:   u32 = 0x161B22FF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_RED:    u32 = 0xFF7B72FF;

const WAVE_COLS: [u32; 4] = [C_SEL, C_GREEN, C_ORANGE, C_RED];
const WAVE_NAMES: [&str; 4] = ["W1", "W2", "W3", "W4"];

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

struct Wave {
    freq:  f32,
    amp:   f32,
    phase: f32,
}

impl Wave {
    fn new(freq: f32, amp: f32) -> Self { Wave { freq, amp, phase: 0.0 } }
    fn y_norm(&self, xn: f32, t: f32) -> f32 {
        self.amp * (2.0 * std::f32::consts::PI * self.freq * xn + self.phase + t).sin()
    }
}

struct App {
    waves: [Wave; 4],
    sel:   usize,
    time:  f32,
}

impl App {
    fn new() -> Self {
        App {
            waves: [
                Wave::new(1.0, 0.8),
                Wave::new(2.0, 0.6),
                Wave::new(3.0, 0.4),
                Wave::new(4.5, 0.3),
            ],
            sel:  0,
            time: 0.0,
        }
    }

    fn render(&self) {
        fill(0, 0, W, GY, C_HEADER);
        text(12, 4, C_TEXT, "Sine Wave Generator");
        text(440, 4, C_HINT, "Tab=select  A/Z=amp  S/X=freq  D/C=phase  Q=quit");

        fill(0, GY, PLOT_W, H - GY, C_BG);
        fill(PANEL_X, GY, W - PANEL_X, H - GY, C_CARD);
        fill(PANEL_X - 1, GY, 1, H - GY, C_BORDER);

        let center = (PLOT_TOP + PLOT_BOT) / 2;
        let half_range = (PLOT_BOT - PLOT_TOP) as f32 / 2.0;
        fill(0, center, PLOT_W, 1, C_BORDER);

        for wi in 0..4 {
            let col = WAVE_COLS[wi];
            let w = &self.waves[wi];
            let mut prev_y = {
                let yn = w.y_norm(0.0, self.time);
                (center as f32 - yn * half_range) as i32
            };
            for px in 1..PLOT_W {
                let xn = px as f32 / PLOT_W as f32;
                let yn = w.y_norm(xn, self.time);
                let py = (center as f32 - yn * half_range) as i32;
                let py = py.clamp(PLOT_TOP, PLOT_BOT);
                let ylo = prev_y.min(py);
                let yhi = prev_y.max(py);
                fill(px, ylo, 1, (yhi - ylo + 1).max(2), col);
                prev_y = py;
            }
        }

        text(PANEL_X + 10, GY + 10, C_HINT, "Waves");

        for wi in 0..4 {
            let wy = GY + 36 + wi as i32 * 115;
            let col = WAVE_COLS[wi];
            let border = if wi == self.sel { col } else { C_BORDER };

            fill(PANEL_X + 8, wy, W - PANEL_X - 16, 105, C_BG);
            fill(PANEL_X + 8, wy, W - PANEL_X - 16, 1, border);
            fill(PANEL_X + 8, wy + 104, W - PANEL_X - 16, 1, border);
            fill(PANEL_X + 8, wy, 1, 105, border);
            fill(W - 9, wy, 1, 105, border);

            let sel_marker = if wi == self.sel { " >" } else { "" };
            text(PANEL_X + 16, wy + 8, col,    &format!("{}{}", WAVE_NAMES[wi], sel_marker));
            text(PANEL_X + 16, wy + 30, C_HINT, &format!("Amp   {:.2}", self.waves[wi].amp));
            text(PANEL_X + 16, wy + 52, C_HINT, &format!("Freq  {:.1} Hz", self.waves[wi].freq));
            text(PANEL_X + 16, wy + 74, C_HINT, &format!("Phase {:.2} rad", self.waves[wi].phase));
        }

        flush();
    }

    fn handle(&mut self, line: &str) {
        let w = &mut self.waves[self.sel];
        match line {
            "REPLY:pong" => {
                self.time += 0.07;
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            "\t" => { self.sel = (self.sel + 1) % 4; }
            "a" | "A" => { w.amp = (w.amp + 0.05).min(1.0); }
            "z" | "Z" => { w.amp = (w.amp - 0.05).max(0.0); }
            "s" | "S" => { w.freq = (w.freq + 0.1).min(20.0); }
            "x" | "X" => { w.freq = (w.freq - 0.1).max(0.1); }
            "d" | "D" => { w.phase += std::f32::consts::PI / 8.0; }
            "c" | "C" => { w.phase -= std::f32::consts::PI / 8.0; }
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
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
