// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

// Map canvas dimensions and offset
const MX: i32 = 20;
const MY: i32 = 52;
const MW: i32 = 640;
const MH: i32 = 440;

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
const C_OCEAN: u32  = 0x0A1628FF;
const C_LAND: u32   = 0x2A4A2AFF;
const C_ACTIVE: u32 = 0x4A7A4AFF;

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

// Country: name + rect silhouette on a 640×440 map
// Rects as (x, y, w, h) relative to map origin
struct Country {
    name:  &'static str,
    rects: &'static [(i32, i32, i32, i32)],
}

static COUNTRIES: &[Country] = &[
    Country { name: "Russia",        rects: &[(340,20,220,60),(280,30,90,40),(380,60,180,50),(300,80,200,40)] },
    Country { name: "Canada",        rects: &[(60,20,200,60),(50,60,160,50),(80,30,120,60),(100,80,100,30)] },
    Country { name: "USA",           rects: &[(60,120,220,80),(70,130,200,60),(80,200,60,30),(60,140,180,50)] },
    Country { name: "Brazil",        rects: &[(180,220,120,100),(170,250,100,80),(190,300,80,60),(200,230,90,100)] },
    Country { name: "Australia",     rects: &[(420,280,160,100),(430,290,140,80),(440,360,80,40),(480,300,100,80)] },
    Country { name: "China",         rects: &[(380,100,160,80),(400,120,140,70),(380,160,120,50),(420,110,110,80)] },
    Country { name: "India",         rects: &[(390,160,80,100),(380,200,70,80),(400,250,50,50),(410,170,60,100)] },
    Country { name: "Argentina",     rects: &[(170,290,60,120),(175,320,50,100),(180,390,40,50),(172,300,55,110)] },
    Country { name: "Mexico",        rects: &[(60,180,120,60),(70,200,100,50),(120,220,60,30),(80,190,90,50)] },
    Country { name: "Indonesia",     rects: &[(430,240,60,30),(480,250,70,28),(440,245,50,25),(490,240,60,30)] },
    Country { name: "Sudan",         rects: &[(320,190,70,70),(330,210,60,60),(325,200,65,60),(335,195,55,65)] },
    Country { name: "Algeria",       rects: &[(270,160,80,70),(280,175,70,60),(275,165,75,65),(285,170,60,60)] },
    Country { name: "Congo",         rects: &[(295,240,60,60),(305,250,50,55),(300,245,55,55),(310,240,45,60)] },
    Country { name: "Saudi Arabia",  rects: &[(340,170,80,70),(350,185,70,60),(345,175,75,65),(355,170,65,60)] },
    Country { name: "Kazakhstan",    rects: &[(340,90,110,55),(350,100,100,48),(345,95,105,50),(355,92,90,52)] },
    Country { name: "France",        rects: &[(250,110,50,40),(255,118,45,35),(252,112,48,38),(258,115,40,35)] },
    Country { name: "Germany",       rects: &[(258,100,45,35),(262,107,40,30),(260,103,43,33),(265,100,36,33)] },
    Country { name: "Japan",         rects: &[(460,120,28,70),(463,128,24,65),(461,122,26,68),(464,115,22,72)] },
    Country { name: "UK",            rects: &[(245,100,26,38),(248,107,22,34),(246,102,24,36),(250,100,20,38)] },
    Country { name: "Spain",         rects: &[(242,122,52,35),(248,130,46,30),(244,125,50,33),(250,122,40,33)] },
    Country { name: "Turkey",        rects: &[(295,132,70,35),(302,140,63,30),(298,135,67,33),(305,132,55,33)] },
    Country { name: "Iran",          rects: &[(340,148,70,48),(348,157,62,42),(344,151,66,45),(352,148,55,46)] },
    Country { name: "Nigeria",       rects: &[(270,220,55,48),(277,228,48,43),(273,223,52,45),(280,220,42,46)] },
    Country { name: "Ethiopia",      rects: &[(330,220,60,55),(337,228,53,50),(333,222,57,52),(340,220,46,53)] },
    Country { name: "Egypt",         rects: &[(300,165,55,45),(307,173,48,40),(303,168,52,42),(310,165,42,43)] },
    Country { name: "Peru",          rects: &[(148,250,55,80),(154,260,48,73),(151,253,52,77),(157,250,42,78)] },
    Country { name: "Colombia",      rects: &[(150,220,55,45),(156,228,48,40),(153,222,52,43),(158,220,42,43)] },
    Country { name: "South Africa",  rects: &[(285,320,65,55),(292,328,58,50),(288,322,62,52),(295,320,50,53)] },
    Country { name: "Poland",        rects: &[(270,100,48,35),(275,107,43,30),(272,103,46,33),(278,100,38,33)] },
    Country { name: "Ukraine",       rects: &[(285,105,65,38),(290,113,60,33),(287,108,63,35),(295,105,50,35)] },
];

struct App {
    idx:       usize,
    order:     Vec<usize>,
    input:     String,
    hints:     usize,
    score:     i32,
    done:      bool,
    feedback:  &'static str,
    fb_good:   bool,
    skipped:   usize,
    correct:   usize,
}

impl App {
    fn new() -> Self {
        let order: Vec<usize> = (0..COUNTRIES.len()).collect();
        App {
            idx: 0, order,
            input: String::new(),
            hints: 0, score: 0,
            done: false,
            feedback: "",
            fb_good: false,
            skipped: 0,
            correct: 0,
        }
    }

    fn current(&self) -> &Country { &COUNTRIES[self.order[self.idx]] }

    fn hint_text(&self) -> String {
        let name = self.current().name;
        let show = self.hints.min(name.len());
        let mut s: String = name.chars().take(show).collect();
        for _ in show..name.len() { s.push('_'); }
        s
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Geo Puzzle");
        text(130, 8, C_HINT, "Type country name + Enter  H:hint  N:skip  Q:quit");

        if self.done {
            fill(MX, MY, W - MX * 2, H - MY - 28, C_CARD);
            border(MX, MY, W - MX * 2, H - MY - 28, C_BORDER);
            text(MX + 20, MY + 40, C_ORANGE, "Quiz Complete!");
            text(MX + 20, MY + 70, C_TEXT, &format!("Score: {}/{}  Correct: {}  Skipped: {}", self.score, COUNTRIES.len(), self.correct, self.skipped));
            text(MX + 20, MY + 100, C_HINT, "Press Q to quit.");
            flush();
            return;
        }

        // Map canvas
        fill(MX, MY, MW, MH, C_OCEAN);
        border(MX, MY, MW, MH, C_BORDER);

        // Draw all countries (dim)
        for ci in &self.order {
            let c = &COUNTRIES[*ci];
            for &(rx, ry, rw, rh) in c.rects {
                fill(MX + rx, MY + ry, rw, rh, C_LAND);
            }
        }

        // Highlight current country
        let cur = self.current();
        for &(rx, ry, rw, rh) in cur.rects {
            fill(MX + rx, MY + ry, rw, rh, C_ACTIVE);
        }

        // Right panel
        let px = MX + MW + 12;
        fill(px, MY, W - px - 4, MH, C_CARD);
        border(px, MY, W - px - 4, MH, C_BORDER);

        let qnum = self.idx + 1;
        let qtotal = COUNTRIES.len();
        text(px + 8, MY + 10, C_ORANGE, &format!("Country {}/{}", qnum, qtotal));

        let hint_str = self.hint_text();
        text(px + 8, MY + 34, C_SEL, &hint_str);
        text(px + 8, MY + 56, C_HINT, &format!("Hints used: {}/5  (-{} pts)", self.hints, self.hints));

        text(px + 8, MY + 86, C_ORANGE, "Score so far:");
        text(px + 8, MY + 104, C_GREEN, &format!("{}", self.score));

        text(px + 8, MY + 134, C_HINT, &format!("Correct: {}", self.correct));
        text(px + 8, MY + 152, C_HINT, &format!("Skipped: {}", self.skipped));

        if !self.feedback.is_empty() {
            let fc = if self.fb_good { C_GREEN } else { C_RED };
            text(px + 8, MY + 180, fc, self.feedback);
        }

        // Input box below map
        let iy = MY + MH + 12;
        fill(MX, iy, MW, 36, C_CARD);
        border(MX, iy, MW, 36, C_BORDER);
        text(MX + 10, iy + 10, C_HINT, "Guess: ");
        let cursor_input = format!("{}|", self.input);
        text(MX + 66, iy + 10, C_TEXT, &cursor_input);

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!("Q{}/{}  H=hint  N=skip  score={}", qnum, qtotal, self.score));
        flush();
    }

    fn advance(&mut self) {
        self.idx += 1;
        self.input.clear();
        self.hints = 0;
        self.feedback = "";
        if self.idx >= COUNTRIES.len() { self.done = true; }
    }

    fn handle(&mut self, line: &str) {
        if self.done { if line == "q" || line == "Q" || line == "\x03" { std::process::exit(0); } self.draw(); return; }

        match line {
            "h" | "H" => {
                if self.hints < 5 { self.hints += 1; }
                self.feedback = "";
            }
            "n" | "N" => {
                self.feedback = "Skipped";
                self.fb_good = false;
                self.skipped += 1;
                self.advance();
            }
            "\x7f" => { self.input.pop(); }
            "\r" | "" => {
                let guess = self.input.trim().to_lowercase();
                let answer = self.current().name.to_lowercase();
                if guess == answer {
                    let pts = (5 - self.hints as i32).max(1);
                    self.score += pts;
                    self.correct += 1;
                    self.feedback = "Correct!";
                    self.fb_good = true;
                    self.advance();
                } else {
                    self.feedback = "Wrong, try again";
                    self.fb_good = false;
                    self.input.clear();
                }
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {
                if line.len() == 1 {
                    let b = line.as_bytes()[0];
                    if b.is_ascii_graphic() || b == b' ' { self.input.push(b as char); }
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
