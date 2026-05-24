// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 680;

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

static WORDS: &[&str] = &[
    // 4-letter anagram groups
    "stop","tops","pots","opts","spot","post",
    "team","mate","tame","meat",
    "evil","vile","live","veil",
    "rats","star","arts","tars",
    "care","race","acre",
    "owns","snow","nows","sown",
    "sale","lase","seal","ales","leas",
    "emit","time","mite","item",
    "leap","pale","peal","plea","alep",
    "nude","dune","done","node","undo",
    // 5-letter anagram groups
    "stare","tears","rates","tares","aster",
    "least","steal","tales","slate","tesla",
    "trace","cater","react","crate","carte",
    "pleat","leapt","plate","petal",
    "snare","crane","caner","nacre","nears",
    "earns","saner","reins","resin","risen","siren","rinse",
    "trail","trial","litre","tlair",
    "inert","inter","nitre","trine",
    "tired","tried","tires","tries","rites","write",
    "grime","miger","emir",
    "store","rotes","tores","roset","torse",
    "words","sword","dross",
    "spine","snipe","penis","pines","peins",
    "siren","resin","risen","rinse","reins",
    // 6-letter anagram groups
    "listen","silent","tinsel","enlist","inlets",
    "stream","master","tamers","maters","ramets",
    "alerts","alters","artels","ratels","estral",
    "hearts","earths","haters","shater",
    "shared","shader","rasher","dasher","shread",
    "detail","tailed","dilate","latied",
    "single","lensig","ingles","girnels",
    "caters","reacts","traces","cartes","caster","carets",
    "lament","mantle","mental","lament",
    "planet","platen","leapt",
    "rental","antler","learnt","altner",
    "garden","danger","gander","ranged","derang",
    "grains","rasing","garnis","resign","signer","singer",
    "gentle","gluten","lunget",
    "wasted","tasted","stated","lasted","slated","datles",
    "united","untied","nudite",
    // 7-letter anagram groups
    "strange","garnets","grantse","sternga",
    "replace","creepla","percale",
    "parties","traipse","pirates","pastier",
    "painter","pertain","repaint","pintare",
    "nastier","antsier","retinas","retsina","stainer","stearin",
    "created","reacted","catered","decater",
    "monster","mentors","torsmen",
    "dormant","mordant",
    "chapter","patcher","repatch",
    "battery","barytes",
    "student","stunted",
    "section","notices","coisten",
    // Standalone words (no easy anagrams)
    "apple","bread","chair","dance","eight",
    "flame","globe","house","image","judge",
    "knife","lemon","music","night","ocean",
    "paint","queen","river","shade","table",
    "umbra","voice","water","xenon","yacht",
    "zebra","atlas","blend","civic","delta",
    "epoch","flair","globe","heard","index",
    "joker","kayak","laser","manor","nexus",
    "olive","piano","quilt","radar","scent",
    "tidal","ultra","visor","witch","xylem",
    "youth","azure","baron","cedar","debut",
    "eagle","flint","grail","haste","irony",
    "joust","kneel","lyric","moose","nerve",
    "orbit","pearl","quake","raven","solar",
    "tapir","usage","valid","waltz","xerox",
    "yield","abbot","bench","clock","draft",
    "evoke","fjord","guise","haven","igloo",
    "jazzy","knave","latch","mimic","notch",
];

fn sort_word(w: &str) -> [u8; 8] {
    let mut arr = [0u8; 8];
    let b = w.as_bytes();
    let len = b.len().min(8);
    arr[..len].copy_from_slice(&b[..len]);
    arr[..len].sort_unstable();
    arr
}

fn find_anagrams<'a>(word: &str, dict: &[&'a str]) -> Vec<&'a str> {
    let w = word.to_lowercase();
    if w.len() < 3 || w.len() > 8 { return vec![]; }
    let key = sort_word(&w);
    dict.iter().filter(|&&d| {
        let dl = d.to_lowercase();
        dl != w && dl.len() == w.len() && sort_word(&dl) == key
    }).copied().collect()
}

fn scramble(word: &str, seed: u64) -> String {
    let mut chars: Vec<char> = word.chars().collect();
    let mut s = seed;
    for i in (1..chars.len()).rev() {
        s = lcg(s);
        let j = (s >> 33) as usize % (i + 1);
        chars.swap(i, j);
    }
    // Ensure scramble differs from original
    if chars.iter().zip(word.chars()).all(|(a, b)| *a == b) && chars.len() > 1 {
        chars.swap(0, 1);
    }
    chars.into_iter().collect()
}

enum Mode { Solver, Challenge }

struct App {
    mode:        Mode,
    input:       String,
    results:     Vec<String>,
    // challenge
    challenge:   String,   // the scrambled word shown
    answer:      String,   // correct answer
    timer:       u32,      // ticks remaining (15)
    submitted:   bool,
    last_ok:     Option<bool>,
    last_wrong:  String,
    correct:     u32,
    total:       u32,
    streak:      u32,
    best_streak: u32,
    seed:        u64,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            mode: Mode::Solver,
            input: String::new(), results: Vec::new(),
            challenge: String::new(), answer: String::new(),
            timer: 15, submitted: false,
            last_ok: None, last_wrong: String::new(),
            correct: 0, total: 0, streak: 0, best_streak: 0,
            seed: 0xC0DE_BABE_5AFE_1234u64,
        };
        a.new_challenge();
        a
    }

    fn new_challenge(&mut self) {
        self.seed = lcg(self.seed);
        let idx = ((self.seed >> 33) as usize) % WORDS.len();
        self.answer = WORDS[idx].to_string();
        self.seed = lcg(self.seed);
        self.challenge = scramble(&self.answer, self.seed);
        self.input.clear();
        self.timer = 15;
        self.submitted = false;
        self.last_ok = None;
        self.last_wrong.clear();
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Anagram Solver");
        text(200, 8, C_HINT, "Tab:mode  Ctrl+R:new  Q:quit");

        let tabs = [("Solver", matches!(self.mode, Mode::Solver)),
                    ("Challenge", matches!(self.mode, Mode::Challenge))];
        for (i, &(label, active)) in tabs.iter().enumerate() {
            let tx = 720 + i as i32 * 100;
            if active {
                fill(tx - 4, 4, label.len() as i32 * 8 + 8, 24, C_CARD);
                border(tx - 4, 4, label.len() as i32 * 8 + 8, 24, C_SEL);
                text(tx, 10, C_SEL, label);
            } else {
                text(tx, 10, C_HINT, label);
            }
        }

        match self.mode {
            Mode::Solver    => self.draw_solver(),
            Mode::Challenge => self.draw_challenge(),
        }

        fill(0, H - 24, W, 24, C_HEADER);
        let status = match self.mode {
            Mode::Solver    => format!("Solver | dictionary: {} words | found: {}", WORDS.len(), self.results.len()),
            Mode::Challenge => format!("Challenge | {}/{} correct ({}%) | Streak: {} | Best: {}",
                self.correct, self.total,
                if self.total > 0 { self.correct * 100 / self.total } else { 0 },
                self.streak, self.best_streak),
        };
        text(12, H - 18, C_HINT, &status);
        flush();
    }

    fn draw_solver(&self) {
        // Input box
        fill(24, 44, W - 48, 44, C_CARD);
        border(24, 44, W - 48, 44, C_BORDER);
        text(32, 50, C_HINT, "Type a word and press Enter to find anagrams:");
        let inp = if self.input.is_empty() { "e.g. stop, listen, plates..." } else { &self.input };
        let inp_c = if self.input.is_empty() { C_HINT } else { C_TEXT };
        text(32, 66, inp_c, inp);

        // Results
        fill(24, 98, W - 48, H - 126, C_CARD);
        border(24, 98, W - 48, H - 126, C_BORDER);

        if self.results.is_empty() {
            if self.input.is_empty() {
                text(32, 114, C_HINT, "Enter any word above to search for anagrams in the built-in dictionary.");
                text(32, 132, C_HINT, &format!("Dictionary has {} words (4-7 letters each).", WORDS.len()));
                // Show example anagram groups
                text(32, 158, C_HINT, "Examples:");
                let examples = [
                    ("listen →", "silent, tinsel, enlist"),
                    ("stop →",   "tops, pots, opts, spot, post"),
                    ("trace →",  "cater, react, crate, carte"),
                    ("stare →",  "tears, rates, tares, aster"),
                    ("parties →","traipse, pirates, pastier"),
                ];
                for (i, &(word, anags)) in examples.iter().enumerate() {
                    let ey = 176 + i as i32 * 18;
                    text(32, ey, C_ORANGE, word);
                    text(32 + word.len() as i32 * 8 + 4, ey, C_HINT, anags);
                }
            } else {
                text(32, 114, C_ORANGE, &format!("No anagrams found for \"{}\"", self.input));
                text(32, 132, C_HINT, "Try a different word, or check spelling.");
            }
        } else {
            text(32, 106, C_GREEN, &format!("{} anagram(s) found for \"{}\":", self.results.len(), self.input));
            let cols = 5;
            for (i, word) in self.results.iter().enumerate() {
                let col = (i % cols) as i32;
                let row = (i / cols) as i32;
                let rx = 32 + col * 180;
                let ry = 124 + row * 20;
                fill(rx - 2, ry - 2, word.len() as i32 * 8 + 6, 18, C_HEADER);
                text(rx, ry, C_SEL, word);
            }
        }
    }

    fn draw_challenge(&self) {
        // Timer bar
        let timer_w = (W - 48) * self.timer as i32 / 15;
        let timer_c = if self.timer > 10 { C_GREEN } else if self.timer > 5 { C_ORANGE } else { C_RED };
        fill(24, 44, W - 48, 12, C_CARD);
        fill(24, 44, timer_w.max(0), 12, timer_c);
        text(W - 80, 44, C_HINT, &format!("{}/15", self.timer));

        // Scrambled word display
        fill(24, 64, W - 48, 80, C_CARD);
        border(24, 64, W - 48, 80, C_ORANGE);
        text(32, 72, C_HINT, "Unscramble this word:");
        let chx = W / 2 - self.challenge.len() as i32 * 8 / 2;
        text(chx, 96, C_TEXT, &self.challenge);

        // Input area
        fill(24, 152, W - 48, 36, C_CARD);
        border(24, 152, W - 48, 36, C_BORDER);
        text(32, 160, C_HINT, "Your answer:");
        let inp = if self.input.is_empty() { "type your answer..." } else { &self.input };
        let inp_c = if self.input.is_empty() { C_HINT } else { C_TEXT };
        text(136, 160, inp_c, inp);

        // Feedback
        if let Some(ok) = self.last_ok {
            if ok {
                text(32, 200, C_GREEN, &format!("Correct!  \"{}\"  (any key for next)", self.answer));
            } else {
                text(32, 200, C_RED, &format!("Wrong!  Answer: \"{}\"  (any key for next)", self.answer));
            }
        } else if self.timer == 0 && !self.submitted {
            text(32, 200, C_RED, &format!("Time up!  Answer was: \"{}\"", self.answer));
        } else {
            text(32, 200, C_HINT, "Press Enter to submit  |  Ctrl+R to skip");
        }

        // Score panels
        let sp_y = 228i32;
        let hw = (W - 52) / 2;
        fill(24, sp_y, hw, 56, C_CARD);
        border(24, sp_y, hw, 56, C_BORDER);
        text(32, sp_y + 8, C_HINT, "SCORE");
        let pct = if self.total > 0 { self.correct * 100 / self.total } else { 0 };
        let sc = if pct > 79 { C_GREEN } else if pct > 59 { C_ORANGE } else { C_HINT };
        text(32, sp_y + 28, sc, &format!("{}/{} ({}%)", self.correct, self.total, pct));

        let sp2 = 28 + hw;
        fill(sp2, sp_y, hw, 56, C_CARD);
        border(sp2, sp_y, hw, 56, C_BORDER);
        text(sp2 + 8, sp_y + 8, C_HINT, "STREAK");
        let stc = if self.streak > 5 { C_GREEN } else if self.streak > 2 { C_ORANGE } else { C_HINT };
        text(sp2 + 8, sp_y + 28, stc, &format!("{}  (best: {})", self.streak, self.best_streak));

        // Hint: anagrams of answer
        let anags = find_anagrams(&self.answer, WORDS);
        if !anags.is_empty() && self.last_ok.is_some() {
            let ag_y = sp_y + 66;
            fill(24, ag_y, W - 48, 30, C_CARD);
            border(24, ag_y, W - 48, 30, C_BORDER);
            let ag_str = anags.iter().map(|w| *w).collect::<Vec<_>>().join(", ");
            text(32, ag_y + 8, C_HINT, &format!("Other anagrams of \"{}\": {}", self.answer, ag_str));
        }
    }

    fn submit_challenge(&mut self) {
        let ok = self.input.to_lowercase() == self.answer.to_lowercase();
        self.total += 1;
        if ok {
            self.correct += 1;
            self.streak += 1;
            self.best_streak = self.best_streak.max(self.streak);
            self.last_ok = Some(true);
        } else {
            self.streak = 0;
            self.last_ok = Some(false);
            self.last_wrong = self.input.clone();
        }
        self.submitted = true;
    }

    fn tick(&mut self) {
        if matches!(self.mode, Mode::Challenge) && !self.submitted && self.last_ok.is_none() {
            if self.timer > 0 {
                self.timer -= 1;
                if self.timer == 0 {
                    self.total += 1;
                    self.streak = 0;
                    self.submitted = true;
                }
                self.draw();
            }
        }
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" { self.tick(); return; }
        match line {
            "\t" => {
                self.mode = match self.mode { Mode::Solver => Mode::Challenge, Mode::Challenge => Mode::Solver };
                self.input.clear();
                self.results.clear();
                self.draw();
                return;
            }
            "\x12" => { // Ctrl+R
                match self.mode {
                    Mode::Challenge => { self.new_challenge(); }
                    Mode::Solver => { self.input.clear(); self.results.clear(); }
                }
                self.draw();
                return;
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }

        match self.mode {
            Mode::Solver => {
                match line {
                    "\x7f" => { self.input.pop(); }
                    "" => {
                        if !self.input.is_empty() {
                            let w = self.input.to_lowercase();
                            self.results = find_anagrams(&w, WORDS)
                                .into_iter().map(|s| s.to_string()).collect();
                            self.results.sort();
                        }
                    }
                    s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic() => {
                        if self.input.len() < 10 { self.input.push_str(s); }
                    }
                    _ => {}
                }
            }
            Mode::Challenge => {
                if self.submitted || self.timer == 0 {
                    // Any key advances to next challenge
                    match line { "q"|"Q"|"\x03"|"\t"|"\x12" => {} _ => { self.new_challenge(); } }
                    self.draw();
                    return;
                }
                match line {
                    "\x7f" => { self.input.pop(); }
                    "" => { if !self.input.is_empty() { self.submit_challenge(); } }
                    s if s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic() => {
                        if self.input.len() < 10 { self.input.push_str(s); }
                    }
                    _ => {}
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
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        if line == "REPLY:pong" {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }
    }
}
