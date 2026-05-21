use std::io::{self, BufRead, Write};

const W: u32 = 1040;
const H: u32 = 640;
const HEADER_H: u32 = 48;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;

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

const PHRASES: [&str; 10] = [
    "the quick brown fox jumps over the lazy dog",
    "pack my box with five dozen liquor jugs",
    "how vexingly quick daft zebras jump",
    "the five boxing wizards jump quickly",
    "sphinx of black quartz judge my vow",
    "two driven jocks help fax my big quiz",
    "the job requires extra pluck and zeal",
    "waltz nymph for quick jigs vex bud",
    "glib jocks quiz nymph to vex dwarf",
    "bright vixens jump dozing fowl quack",
];

// CPU WPM: racer 0=40, 1=60, 2=80 WPM
// chars per tick: WPM * 5 chars/word / 60 sec/min / ticks_per_sec
// ticks_per_sec ≈ 10 (ping-pong roughly 100ms per tick)
// chars_per_tick = WPM * 5 / 600
const CPU_WPM: [u32; 3] = [40, 60, 80];
const CPU_COLORS: [u32; 3] = [0xBF5AF2FF, C_ORANGE, C_RED];
const CPU_NAMES: [&str; 3] = ["CPU-Slow", "CPU-Mid", "CPU-Fast"];

// Progress bar geometry
const BAR_X: u32 = 200;
const BAR_W: u32 = 600;
const BAR_H: u32 = 32;
const BAR_Y0: u32 = HEADER_H + 240; // player bar y
const BAR_STEP: u32 = 60;

#[derive(PartialEq)]
enum State { Ready, Racing, Done(usize) } // Done(winner): 0=player, 1..3=cpu

struct Race {
    phrase:      Vec<char>,
    typed:       usize,   // chars correctly typed
    cpu_prog:    [u32; 3], // 0..phrase.len()*100 fixed-point progress (×100)
    ticks:       u32,
    player_wpm:  u32,
    state:       State,
    seed:        u64,
    wins:        u32,
    losses:      u32,
    wrong:       bool,    // current char typed wrong
}

impl Race {
    fn new(seed: u64) -> Self {
        let mut s = lcg(seed);
        let idx = (s >> 33) as usize % PHRASES.len();
        s = lcg(s);
        Race {
            phrase:     PHRASES[idx].chars().collect(),
            typed:      0,
            cpu_prog:   [0; 3],
            ticks:      0,
            player_wpm: 0,
            state:      State::Racing,
            seed:       s,
            wins:       0,
            losses:     0,
            wrong:      false,
        }
    }

    fn new_race(&mut self) {
        let wins = self.wins;
        let losses = self.losses;
        let s = lcg(self.seed);
        let idx = (s >> 33) as usize % PHRASES.len();
        let seed2 = lcg(s);
        *self = Race {
            phrase:     PHRASES[idx].chars().collect(),
            typed:      0,
            cpu_prog:   [0; 3],
            ticks:      0,
            player_wpm: 0,
            state:      State::Racing,
            seed:       seed2,
            wins,
            losses,
            wrong:      false,
        };
    }

    fn tick(&mut self) {
        if self.state != State::Racing { return; }
        self.ticks += 1;
        let total = self.phrase.len() as u32;
        // Advance CPUs: chars_per_tick = WPM*5/600; ×100 fixed-point = WPM*500/600
        for i in 0..3 {
            let advance = CPU_WPM[i] * 500 / 600; // ≈ WPM*0.833 in fixed-point/100
            self.cpu_prog[i] = (self.cpu_prog[i] + advance).min(total * 100);
            if self.cpu_prog[i] >= total * 100 {
                // CPU wins
                self.losses += 1;
                self.state = State::Done(i + 1);
                return;
            }
        }
        // Compute player WPM (typed chars / 5 / elapsed_min)
        if self.ticks > 0 {
            let elapsed_ticks = self.ticks;
            let elapsed_sec_x10 = elapsed_ticks; // ≈ 1 tick/sec (approx)
            if elapsed_sec_x10 > 0 {
                self.player_wpm = self.typed as u32 * 600 / 5 / elapsed_ticks.max(1);
            }
        }
    }

    fn type_char(&mut self, c: char) {
        if self.state != State::Racing { return; }
        if self.typed >= self.phrase.len() { return; }
        if c == self.phrase[self.typed] {
            self.typed += 1;
            self.wrong = false;
            if self.typed == self.phrase.len() {
                self.wins += 1;
                self.state = State::Done(0);
            }
        } else {
            self.wrong = true;
        }
    }

    fn backspace(&mut self) {
        if self.state != State::Racing { return; }
        if self.typed > 0 { self.typed -= 1; }
        self.wrong = false;
    }

    fn player_progress_100(&self) -> u32 {
        if self.phrase.is_empty() { return 100; }
        (self.typed as u32 * 100) / self.phrase.len() as u32
    }
}

fn draw(r: &Race) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Typing Race");
    text(180, 16, C_HINT, &format!("Wins: {}  Losses: {}  WPM: {}", r.wins, r.losses, r.player_wpm));
    text(560, 16, C_HINT, "Type phrase  Backspace=del  N=new");

    // Phrase display
    let phrase_y = HEADER_H + 24;
    let phrase_x = 40u32;
    for (i, &c) in r.phrase.iter().enumerate() {
        let cx = phrase_x + i as u32 * 16;
        let color = if i < r.typed {
            C_GREEN
        } else if i == r.typed {
            if r.wrong { C_RED } else { C_SEL }
        } else {
            C_HINT
        };
        let s = c.to_string();
        text(cx, phrase_y, color, &s);
    }
    // Underline current char
    if r.typed < r.phrase.len() {
        let ux = phrase_x + r.typed as u32 * 16;
        fill(ux, phrase_y + 18, 12, 2, if r.wrong { C_RED } else { C_SEL });
    }

    // Progress bars
    let player_p = r.player_progress_100();

    // Player bar
    let py = BAR_Y0;
    text(40, py + 8, C_SEL, "You");
    fill(BAR_X, py, BAR_W, BAR_H, C_CARD);
    fill(BAR_X, py, BAR_W * player_p / 100, BAR_H, C_SEL);
    border(BAR_X, py, BAR_W, BAR_H, C_BORDER);
    text(BAR_X + BAR_W + 8, py + 8, C_HINT, &format!("{}%", player_p));

    for i in 0..3 {
        let cy = BAR_Y0 + (i as u32 + 1) * BAR_STEP;
        let prog = r.cpu_prog[i] * 100 / (r.phrase.len() as u32 * 100).max(1);
        let prog_clamp = prog.min(100);
        text(40, cy + 8, CPU_COLORS[i], CPU_NAMES[i]);
        fill(BAR_X, cy, BAR_W, BAR_H, C_CARD);
        if prog_clamp > 0 {
            fill(BAR_X, cy, BAR_W * prog_clamp / 100, BAR_H, CPU_COLORS[i]);
        }
        border(BAR_X, cy, BAR_W, BAR_H, C_BORDER);
        text(BAR_X + BAR_W + 8, cy + 8, C_HINT, &format!("{} WPM", CPU_WPM[i]));
    }

    // Win/Loss overlay
    match &r.state {
        State::Done(winner) => {
            let bx = (W - 400) / 2;
            let by = HEADER_H + 60;
            fill(bx, by, 400, 100, C_HEADER);
            if *winner == 0 {
                border(bx, by, 400, 100, C_GREEN);
                text(bx + 90, by + 16, C_GREEN, "You Win!");
                text(bx + 40, by + 44, C_HINT, &format!("WPM: {}  N=new race", r.player_wpm));
            } else {
                border(bx, by, 400, 100, C_RED);
                text(bx + 60, by + 16, C_RED, &format!("{} won!", CPU_NAMES[winner - 1]));
                text(bx + 40, by + 44, C_HINT, &format!("Your WPM: {}  N=new race", r.player_wpm));
            }
        }
        State::Racing | State::Ready => {}
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let seed: u64 = 0xACE1234567890ABCULL;
    let mut race = Race::new(seed);

    println!("@supervisor: raise typing-race");
    let _ = io::stdout().flush();
    // Start ping-pong
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&race);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") {
            race.tick();
            if race.state == State::Racing {
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            draw(&race);
            continue;
        }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "n" | "N" => {
                race.new_race();
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            "\x7f" => { race.backspace(); }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7f {
                    race.type_char(b as char);
                }
            }
            _ => {}
        }
        draw(&race);
    }
}
