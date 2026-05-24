// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: i32 = 960;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_CARD: u32   = 0x161B22FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

static SENTENCES: &[&str] = &[
    "the quick brown fox jumps over the lazy dog",
    "pack my box with five dozen liquor jugs",
    "how vexingly quick daft zebras jump",
    "the five boxing wizards jump quickly",
    "sphinx of black quartz judge my vow",
    "two driven jocks help fax my big quiz",
    "the job requires extra pluck and zeal from every young wage earner",
    "a mad boxer shot a quick gloved jab to the jaw of his dizzy opponent",
    "the quick onyx goblin jumps over the lazy dwarf",
    "jackdaws love my big sphinx of quartz",
    "we promptly judged antique ivory buckles for the next prize",
    "a quart jar of oil mixed with zinc oxide makes a very bright paint",
    "six big juicy steaks sizzled in a pan as five workmen left the quarry",
    "all questions asked by five watched experts amaze the judge",
    "back in my quaint garden jaunty zinnias vie with flaunting phlox",
    "five or six big jet planes zoomed quickly by the new tower",
    "the explorer was frozen in his big kayak just after making off with six quills",
    "few quips galvanized the mock jury box",
    "by jove my quick study of lexicography won a prize",
    "waxy and quivering jelly beans were picked from the big freezer",
    "the wizard quickly jinxed the gnomes before they vaporized",
    "go quickly and fix the jet blue whale before dawn",
    "amazingly few discotheques provide jukeboxes",
    "crazy fredrick bought many very exquisite opal jewels",
    "the quick brown fox will jump over the lazy poodle next to the log",
    "my faxed joke won a pager in the cable tv quiz show",
    "questions of a zealous nature have become perplexing to judges",
    "john quietly gave back most of the prize money to six hungry zebras",
    "we quickly seized the black axle and just saved it from going in the quagmire",
    "blowzy red vixens fight for a quick jump",
];

#[derive(PartialEq)]
enum State { Ready, Typing, Done }

struct Score { wpm: u32, acc: u32 }

struct App {
    seed:      u64,
    sentence:  &'static str,
    typed:     Vec<u8>,
    state:     State,
    start:     Option<Instant>,
    errors:    usize,
    leaderboard: Vec<Score>,
}

impl App {
    fn new() -> Self {
        let seed = 0xC0FFEE_1234u64;
        let idx = (seed >> 32) as usize % SENTENCES.len();
        App {
            seed,
            sentence: SENTENCES[idx],
            typed: Vec::new(),
            state: State::Ready,
            start: None,
            errors: 0,
            leaderboard: Vec::new(),
        }
    }

    fn restart(&mut self) {
        self.seed = lcg(self.seed);
        let idx = (self.seed >> 32) as usize % SENTENCES.len();
        self.sentence = SENTENCES[idx];
        self.typed.clear();
        self.state = State::Ready;
        self.start = None;
        self.errors = 0;
    }

    fn wpm(&self) -> u32 {
        if let Some(start) = self.start {
            let secs = start.elapsed().as_secs_f32().max(0.1);
            let words = self.typed.len() as f32 / 5.0;
            ((words / secs) * 60.0) as u32
        } else { 0 }
    }

    fn accuracy(&self) -> u32 {
        let total = self.typed.len() + self.errors;
        if total == 0 { return 100; }
        let correct = self.typed.len().saturating_sub(
            self.typed.iter().zip(self.sentence.bytes()).filter(|(a, b)| a != b).count()
        );
        (correct * 100 / total.max(1)) as u32
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Typing Speed Test v2");
        text(240, 8, C_HINT, "Type the sentence  Tab=restart  Q=quit");

        // Sentence display panel
        fill(20, 52, W - 40, 140, C_CARD);
        border(20, 52, W - 40, 140, C_BORDER);

        let sentence = self.sentence.as_bytes();
        let typed = &self.typed;
        let max_cols = 80usize;
        let char_w = 9i32;

        // Wrap sentence into lines of max_cols
        let words: Vec<&str> = self.sentence.split(' ').collect();
        let mut lines: Vec<(usize, usize)> = Vec::new(); // (start_byte, end_byte) in sentence
        let mut line_start = 0usize;
        let mut line_len = 0usize;
        for (wi, w) in words.iter().enumerate() {
            let add = w.len() + if line_len > 0 { 1 } else { 0 };
            if line_len > 0 && line_len + add > max_cols {
                lines.push((line_start, line_start + line_len));
                line_start += line_len + 1;
                line_len = w.len();
            } else {
                if line_len > 0 { line_len += 1; }
                line_len += w.len();
                if wi == words.len() - 1 {
                    lines.push((line_start, line_start + line_len));
                }
            }
        }

        for (li, &(start, end)) in lines.iter().enumerate() {
            let ly = 68 + li as i32 * 20;
            for bi in start..end.min(sentence.len()) {
                let x = 28 + (bi - start) as i32 * char_w;
                let c = if bi < typed.len() {
                    if typed[bi] == sentence[bi] { C_GREEN } else { C_RED }
                } else if bi == typed.len() {
                    C_SEL
                } else {
                    C_HINT
                };
                let ch = [sentence[bi]];
                text(x, ly, c, std::str::from_utf8(&ch).unwrap_or(" "));
            }
            // Cursor past end of sentence
            if typed.len() >= end && typed.len() <= end + 1 && li == lines.len() - 1 {
                let x = 28 + (end - start) as i32 * char_w;
                fill(x, ly, 2, 14, C_SEL);
            }
        }

        // Stats row
        let wpm = self.wpm();
        let acc = self.accuracy();
        let elapsed = self.start.map(|s| s.elapsed().as_secs()).unwrap_or(0);

        fill(20, 204, W - 40, 50, C_CARD);
        border(20, 204, W - 40, 50, C_BORDER);
        text(32, 214, C_ORANGE, "WPM");
        text(32, 232, C_SEL, &format!("{}", wpm));
        text(110, 214, C_ORANGE, "Accuracy");
        text(110, 232, C_GREEN, &format!("{}%", acc));
        text(220, 214, C_ORANGE, "Time");
        text(220, 232, C_HINT, &format!("{}s", elapsed));
        text(310, 214, C_ORANGE, "Progress");
        let pct = typed.len() * 100 / sentence.len().max(1);
        fill(310, 236, 200, 8, C_BORDER);
        fill(310, 236, 200 * pct as i32 / 100, 8, C_GREEN);

        if self.state == State::Done {
            fill(20, 265, W - 40, 40, 0x1A2A1AFF);
            border(20, 265, W - 40, 40, C_GREEN);
            text(32, 278, C_GREEN, &format!("COMPLETE!  {} WPM  {}% accuracy  {}s  Tab=restart", wpm, acc, elapsed));
        } else if self.state == State::Ready {
            fill(20, 265, W - 40, 40, C_CARD);
            border(20, 265, W - 40, 40, C_BORDER);
            text(32, 278, C_HINT, "Start typing to begin...");
        }

        // Leaderboard
        fill(20, 316, W - 40, 180, C_CARD);
        border(20, 316, W - 40, 180, C_BORDER);
        text(32, 326, C_ORANGE, "Leaderboard (last 5 scores)");
        if self.leaderboard.is_empty() {
            text(32, 348, C_HINT, "No scores yet — complete a sentence to record your score.");
        } else {
            for (i, s) in self.leaderboard.iter().enumerate() {
                let tc = if i == 0 { C_ORANGE } else { C_HINT };
                text(32, 348 + i as i32 * 22, tc,
                     &format!("{}. {} WPM  {}% accuracy", i + 1, s.wpm, s.acc));
            }
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!("{}/{} chars  {} WPM  {}% acc",
             typed.len(), sentence.len(), wpm, acc));
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\t" => { self.restart(); self.draw(); return; }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        if self.state == State::Done { self.draw(); return; }

        if line == "\x7f" {
            // Backspace
            if !self.typed.is_empty() { self.typed.pop(); }
        } else if line.len() == 1 {
            let b = line.as_bytes()[0];
            if b.is_ascii_graphic() || b == b' ' {
                if self.state == State::Ready {
                    self.state = State::Typing;
                    self.start = Some(Instant::now());
                }
                let expected = self.sentence.as_bytes().get(self.typed.len()).copied().unwrap_or(0);
                if b != expected { self.errors += 1; }
                self.typed.push(b);

                if self.typed.len() >= self.sentence.len() {
                    self.state = State::Done;
                    let score = Score { wpm: self.wpm(), acc: self.accuracy() };
                    self.leaderboard.insert(0, score);
                    if self.leaderboard.len() > 5 { self.leaderboard.truncate(5); }
                }
            }
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
