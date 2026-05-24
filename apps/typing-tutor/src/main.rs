// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 880;
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

const PHRASES: [&str; 40] = [
    "the quick brown fox jumps over the lazy dog",
    "pack my box with five dozen liquor jugs",
    "how vexingly quick daft zebras jump",
    "the five boxing wizards jump quickly",
    "sphinx of black quartz judge my vow",
    "two driven jocks help fax my big quiz",
    "five quacking zephyrs jolt my wax bed",
    "the jay pig fox zebra and my wolves quack",
    "bright vixens jump dozy fowl quack",
    "coding is the art of telling a computer what to do",
    "practice makes perfect keep typing every day",
    "a good programmer writes code that humans can read",
    "the best way to learn is by doing it yourself",
    "software is eating the world one line at a time",
    "every expert was once a beginner who kept going",
    "rust is a systems language that runs blazingly fast",
    "type with intention and let your fingers flow",
    "keyboards are the instruments of the digital age",
    "focus on accuracy first and speed will follow",
    "ten fingers working together create magic",
    "the journey of a thousand miles begins with one step",
    "consistency is the key to mastering any skill",
    "challenge yourself daily to grow beyond your limits",
    "errors are opportunities to learn something new",
    "slow is smooth and smooth is fast",
    "quality code is written for humans not machines",
    "think twice code once debug never",
    "every keystroke brings you closer to your goal",
    "open source software is built by communities",
    "the terminal is your friend learn to love it",
    "good habits compound into extraordinary results",
    "write programs that do one thing and do it well",
    "simplicity is the ultimate sophistication",
    "measure twice cut once ship with confidence",
    "fast fingers and clear thinking win the day",
    "the compiler never lies listen to its warnings",
    "version control is the safety net of developers",
    "readable code is a gift to your future self",
    "debug with patience and curiosity not frustration",
    "ship early ship often iterate on feedback",
];

fn lcg(seed: u64) -> u64 {
    seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

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

struct Game {
    phrase:        Vec<u8>,
    typed:         Vec<u8>,
    correct_count: usize,
    error_count:   usize,
    ticks:         u64,
    start_tick:    Option<u64>,
    done:          bool,
    phrase_idx:    usize,
    seed:          u64,
}

impl Game {
    fn new(seed: u64) -> Self {
        let s = lcg(seed);
        let idx = (s >> 33) as usize % PHRASES.len();
        Game {
            phrase:        PHRASES[idx].as_bytes().to_vec(),
            typed:         Vec::new(),
            correct_count: 0,
            error_count:   0,
            ticks:         0,
            start_tick:    None,
            done:          false,
            phrase_idx:    idx,
            seed,
        }
    }

    fn type_char(&mut self, c: u8) {
        if self.done { return; }
        let pos = self.typed.len();
        if pos >= self.phrase.len() { return; }
        if self.start_tick.is_none() { self.start_tick = Some(self.ticks); }
        let expected = self.phrase[pos];
        self.typed.push(c);
        if c == expected {
            self.correct_count += 1;
        } else {
            self.error_count += 1;
        }
        if self.typed.len() == self.phrase.len() {
            self.done = true;
        }
    }

    fn backspace(&mut self) {
        if self.done { return; }
        if let Some(c) = self.typed.pop() {
            let pos = self.typed.len();
            let expected = self.phrase[pos];
            if c == expected {
                if self.correct_count > 0 { self.correct_count -= 1; }
            } else {
                if self.error_count > 0 { self.error_count -= 1; }
            }
        }
    }

    fn wpm(&self) -> u64 {
        if let Some(start) = self.start_tick {
            let elapsed = self.ticks.saturating_sub(start);
            if elapsed == 0 { return 0; }
            // ticks ~= pong replies ~60/min; words = correct_chars/5
            let words_x100 = (self.correct_count as u64 * 100) / 5;
            let minutes_x100 = elapsed * 100 / 60;
            if minutes_x100 == 0 { return 0; }
            words_x100 / minutes_x100
        } else {
            0
        }
    }

    fn accuracy(&self) -> u64 {
        let total = self.correct_count + self.error_count;
        if total == 0 { return 100; }
        (self.correct_count as u64 * 100) / total as u64
    }
}

const CHAR_W: u32 = 10; // medium font approx width
const LINE_X: u32 = 40;
const LINE_Y: u32 = 160;
const CHARS_PER_LINE: usize = 60;

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Typing Tutor");
    text(180, 16, C_HINT, &format!("Phrase #{}", g.phrase_idx + 1));
    text(360, 16, C_HINT, &format!("WPM: {}  Accuracy: {}%", g.wpm(), g.accuracy()));
    text(620, 16, C_HINT, "Bsp:delete  R:next phrase");

    // Stats bar
    let typed = g.typed.len();
    let total = g.phrase.len();
    fill(LINE_X, 80, (W - LINE_X * 2) * typed as u32 / total.max(1) as u32, 8, C_GREEN);
    fill(LINE_X, 80, W - LINE_X * 2, 8, 0x0u32);
    border(LINE_X, 80, W - LINE_X * 2, 8, C_BORDER);
    fill(LINE_X, 80, (W - LINE_X * 2) * typed as u32 / total.max(1) as u32, 8, C_GREEN);

    text(LINE_X, 100, C_HINT, &format!("{}/{} characters", typed, total));

    // Phrase display — up to 2 lines of 60 chars
    let phrase_bytes = &g.phrase;
    for line_idx in 0..2usize {
        let start = line_idx * CHARS_PER_LINE;
        if start >= phrase_bytes.len() { break; }
        let end = (start + CHARS_PER_LINE).min(phrase_bytes.len());
        let ly = LINE_Y + line_idx as u32 * 40;

        for (i, &ch) in phrase_bytes[start..end].iter().enumerate() {
            let abs_i = start + i;
            let cx = LINE_X + i as u32 * CHAR_W;

            let color = if abs_i < g.typed.len() {
                if g.typed[abs_i] == ch { C_GREEN } else { C_RED }
            } else if abs_i == g.typed.len() {
                C_TEXT
            } else {
                C_HINT
            };

            // Cursor underline
            if abs_i == g.typed.len() && !g.done {
                fill(cx, ly + 22, CHAR_W, 3, C_SEL);
            }

            let s = if ch == b' ' { " ".to_string() } else { (ch as char).to_string() };
            text(cx, ly, color, &s);
        }
    }

    // Done overlay
    if g.done {
        let bx = (W - 360) / 2;
        let by = (H - 100) / 2;
        fill(bx, by, 360, 100, C_HEADER);
        border(bx, by, 360, 100, C_GREEN);
        text(bx + 80, by + 14, C_GREEN, "Phrase Complete!");
        text(bx + 24, by + 42, C_TEXT, &format!("WPM: {}   Accuracy: {}%", g.wpm(), g.accuracy()));
        text(bx + 80, by + 68, C_HINT, "Press R for next phrase");
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut seed: u64 = 0xFACEFACE12345678;
    let mut game = Game::new(seed);

    println!("@supervisor: raise typing-tutor");
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            game.ticks += 1;
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x7f" => { game.backspace(); }
            "r" | "R" if game.done => {
                seed = lcg(seed);
                game = Game::new(seed);
            }
            s if s.len() == 1 && !game.done => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7F {
                    game.type_char(b);
                }
            }
            _ => {}
        }
        draw(&game);
    }
}
