use std::io::{self, BufRead, Write};

const W: i32 = 880;
const H: i32 = 680;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_CARD: u32   = 0x161B22FF;
const C_PURPLE: u32 = 0xBC8CFFFF;

const NOTES: [&str; 12] = [
    "C","C#","D","D#","E","F","F#","G","G#","A","A#","B"
];

// Scale intervals (half-steps from root)
const SCALE_NAMES: [&str; 8] = [
    "Major", "Natural Minor", "Dorian", "Phrygian",
    "Lydian", "Mixolydian", "Locrian", "Pentatonic",
];
const SCALE_INTERVALS: [[u8; 7]; 8] = [
    [2, 2, 1, 2, 2, 2, 1], // Major
    [2, 1, 2, 2, 1, 2, 2], // Natural Minor
    [2, 1, 2, 2, 2, 1, 2], // Dorian
    [1, 2, 2, 2, 1, 2, 2], // Phrygian
    [2, 2, 2, 1, 2, 2, 1], // Lydian
    [2, 2, 1, 2, 2, 1, 2], // Mixolydian
    [1, 2, 2, 1, 2, 2, 2], // Locrian
    [2, 2, 3, 2, 3, 0, 0], // Pentatonic (5 notes, padded)
];
const SCALE_LEN: [usize; 8] = [7, 7, 7, 7, 7, 7, 7, 5];

// Chord types: intervals from root
const CHORD_NAMES: [&str; 7] = ["Major", "Minor", "7th", "Maj7", "Dim", "Aug", "Sus4"];
// Each chord: intervals between successive chord tones
const CHORD_INTERVALS: [[u8; 3]; 7] = [
    [4, 3, 5], // Major:   1 3 5
    [3, 4, 5], // Minor:   1 b3 5
    [4, 3, 3], // Dom 7th: 1 3 5 b7
    [4, 3, 4], // Maj7:    1 3 5 7
    [3, 3, 6], // Dim:     1 b3 b5 (bb7)
    [4, 4, 4], // Aug:     1 3 #5
    [5, 2, 5], // Sus4:    1 4 5
];

// Named intervals
const INTERVAL_NAMES: [&str; 13] = [
    "Unison","m2","M2","m3","M3","P4","Tritone","P5","m6","M6","m7","M7","Octave"
];

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c);
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s);
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

fn scale_notes(root: usize, scale_idx: usize) -> Vec<usize> {
    let intervals = SCALE_INTERVALS[scale_idx];
    let len = SCALE_LEN[scale_idx];
    let mut notes = vec![root];
    let mut pos = root;
    for i in 0..len - 1 {
        pos = (pos + intervals[i] as usize) % 12;
        notes.push(pos);
    }
    notes
}

fn chord_notes(root: usize, chord_idx: usize) -> Vec<usize> {
    let intervals = CHORD_INTERVALS[chord_idx];
    let mut notes = vec![root];
    let mut pos = root;
    for i in 0..3 {
        pos = (pos + intervals[i] as usize) % 12;
        notes.push(pos);
    }
    notes
}

// Draw 12-note circle (note wheel). Center cx, cy, radius r.
fn draw_note_wheel(cx: i32, cy: i32, r: i32, active: &[usize], root: usize) {
    // Notes arranged like a clock: C at top, going clockwise
    for i in 0..12usize {
        // angle: i * 30 degrees, 0 = top, clockwise
        // cos(-angle+90°), sin(-angle+90°)
        // Using integer approximation: precomputed positions
        let (sin_i, cos_i) = CIRCLE_POS[i];
        let nx = cx + (r as i64 * sin_i / 1000) as i32;
        let ny = cy - (r as i64 * cos_i / 1000) as i32;

        let is_active = active.contains(&i);
        let is_root   = i == root;
        let bg = if is_root   { C_ORANGE }
                 else if is_active { C_SEL }
                 else { C_CARD };
        let bc = if is_root   { C_YELLOW }
                 else if is_active { C_GREEN }
                 else { C_BORDER };

        let nw = 24;
        let nh = 18;
        fill(nx - nw / 2, ny - nh / 2, nw, nh, bg);
        border(nx - nw / 2, ny - nh / 2, nw, nh, bc);
        let note = NOTES[i];
        let nc = if is_root { C_BG } else if is_active { C_TEXT } else { C_HINT };
        let nx_text = nx - (note.len() as i32 * 8) / 2;
        text(nx_text, ny - 8, nc, note);
    }
}

// Precomputed sin/cos * 1000 for 12 clock positions (30-degree steps)
// (sin, cos) * 1000 for 0°,30°,60°,...,330°
static CIRCLE_POS: [(i64, i64); 12] = [
    (0, 1000),    // 0°  C
    (500, 866),   // 30° C#
    (866, 500),   // 60° D
    (1000, 0),    // 90° D#
    (866, -500),  // 120° E
    (500, -866),  // 150° F
    (0, -1000),   // 180° F#
    (-500, -866), // 210° G
    (-866, -500), // 240° G#
    (-1000, 0),   // 270° A
    (-866, 500),  // 300° A#
    (-500, 866),  // 330° B
];

#[derive(PartialEq, Clone, Copy)]
enum Tab { Scales, Chords, Quiz }

struct QuizState {
    root: usize,
    interval: usize,
    options: [usize; 4],
    correct_idx: usize,
    answered: Option<usize>,
    score: i32,
    streak: u32,
    best_streak: u32,
    total: u32,
    correct_total: u32,
    feedback_ticks: u8,
    seed: u64,
}

impl QuizState {
    fn new(seed: u64) -> Self {
        let mut qs = QuizState {
            root: 0, interval: 0, options: [0; 4], correct_idx: 0,
            answered: None, score: 0, streak: 0, best_streak: 0,
            total: 0, correct_total: 0, feedback_ticks: 0, seed,
        };
        qs.next_question();
        qs
    }

    fn next_question(&mut self) {
        self.seed = lcg(self.seed);
        self.root = (self.seed >> 33) as usize % 12;
        self.seed = lcg(self.seed);
        self.interval = 1 + (self.seed >> 33) as usize % 12; // 1..=12
        let correct_note = (self.root + self.interval) % 12;

        // Generate 4 unique options
        let mut opts = [correct_note, 0, 0, 0];
        let mut used = [false; 12];
        used[correct_note] = true;
        let mut filled = 1;
        while filled < 4 {
            self.seed = lcg(self.seed);
            let n = (self.seed >> 33) as usize % 12;
            if !used[n] {
                used[n] = true;
                opts[filled] = n;
                filled += 1;
            }
        }
        // Shuffle
        for i in (1..4).rev() {
            self.seed = lcg(self.seed);
            let j = (self.seed >> 33) as usize % (i + 1);
            opts.swap(i, j);
        }
        self.options = opts;
        self.correct_idx = opts.iter().position(|&x| x == correct_note).unwrap_or(0);
        self.answered = None;
    }

    fn answer(&mut self, idx: usize) {
        if self.answered.is_some() { return; }
        self.answered = Some(idx);
        self.total += 1;
        if idx == self.correct_idx {
            self.score += 10;
            self.correct_total += 1;
            self.streak += 1;
            if self.streak > self.best_streak { self.best_streak = self.streak; }
        } else {
            self.score -= 5;
            self.streak = 0;
        }
        self.feedback_ticks = 3;
    }

    fn tick(&mut self) -> bool {
        if self.answered.is_some() && self.feedback_ticks > 0 {
            self.feedback_ticks -= 1;
            if self.feedback_ticks == 0 {
                self.next_question();
                return true;
            }
        }
        false
    }
}

struct App {
    tab: Tab,
    scale_root: usize,
    scale_idx: usize,
    chord_root: usize,
    chord_idx: usize,
    focus: u8, // 0=root, 1=type
    quiz: QuizState,
}

impl App {
    fn new() -> Self {
        App {
            tab: Tab::Scales,
            scale_root: 0,
            scale_idx: 0,
            chord_root: 0,
            chord_idx: 0,
            focus: 0,
            quiz: QuizState::new(0x4D75_1234_5678_9ABCu64),
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Music Theory");

        // Tabs
        let tabs = [("Scales", Tab::Scales), ("Chords", Tab::Chords), ("Interval Quiz", Tab::Quiz)];
        let mut tx = W - 320;
        for (name, t) in &tabs {
            let active = self.tab == *t;
            let bg = if active { C_SEL } else { C_CARD };
            let tc = if active { C_BG } else { C_HINT };
            let tw = name.len() as i32 * 8 + 16;
            fill(tx, 4, tw, 24, bg);
            border(tx, 4, tw, 24, C_BORDER);
            text(tx + 8, 10, tc, name);
            tx += tw + 4;
        }
        text(12, 8, C_HINT, "Tab=cycle");

        match self.tab {
            Tab::Scales => self.draw_scales(),
            Tab::Chords => self.draw_chords(),
            Tab::Quiz   => self.draw_quiz(),
        }

        flush();
    }

    fn draw_scales(&self) {
        let active_notes = scale_notes(self.scale_root, self.scale_idx);
        let cx = 280;
        let cy = 360;
        let r  = 200;
        draw_note_wheel(cx, cy, r, &active_notes, self.scale_root);

        // Controls panel (right side)
        let px = 580;
        fill(px, 50, W - px - 12, H - 60, C_CARD);
        border(px, 50, W - px - 12, H - 60, C_BORDER);

        text(px + 10, 60, C_HINT, "ROOT NOTE");
        // Root selector
        let rsel = self.focus == 0;
        let rbc = if rsel { C_SEL } else { C_BORDER };
        border(px + 10, 78, W - px - 32, 22, rbc);
        text(px + 14, 82, C_TEXT, NOTES[self.scale_root]);
        text(px + 14 + 24, 82, C_HINT, "  <> change");

        text(px + 10, 112, C_HINT, "SCALE TYPE");
        let sbc = if self.focus == 1 { C_SEL } else { C_BORDER };
        border(px + 10, 130, W - px - 32, 22, sbc);
        text(px + 14, 134, C_TEXT, SCALE_NAMES[self.scale_idx]);

        text(px + 10, 166, C_HINT, "SCALE NOTES:");
        for (i, &n) in active_notes.iter().enumerate() {
            let degree = ["1","2","3","4","5","6","7","?","?"];
            text(px + 10, 182 + i as i32 * 18, C_SEL,
                 &format!("{}: {}", degree[i.min(8)], NOTES[n]));
        }

        text(px + 10, H - 120, C_HINT, "NAVIGATION:");
        text(px + 10, H - 104, C_HINT, "Tab = change tab");
        text(px + 10, H - 88, C_HINT, "F = focus toggle");
        text(px + 10, H - 72, C_HINT, "<> = change");
        text(px + 10, H - 56, C_HINT, "UP/DN = scroll");
    }

    fn draw_chords(&self) {
        let c_notes = chord_notes(self.chord_root, self.chord_idx);
        let cx = 280;
        let cy = 360;
        let r  = 200;
        draw_note_wheel(cx, cy, r, &c_notes, self.chord_root);

        let px = 580;
        fill(px, 50, W - px - 12, H - 60, C_CARD);
        border(px, 50, W - px - 12, H - 60, C_BORDER);

        text(px + 10, 60, C_HINT, "ROOT NOTE");
        let rbc = if self.focus == 0 { C_SEL } else { C_BORDER };
        border(px + 10, 78, W - px - 32, 22, rbc);
        text(px + 14, 82, C_TEXT, NOTES[self.chord_root]);

        text(px + 10, 112, C_HINT, "CHORD TYPE");
        let cbc = if self.focus == 1 { C_SEL } else { C_BORDER };
        border(px + 10, 130, W - px - 32, 22, cbc);
        text(px + 14, 134, C_TEXT, CHORD_NAMES[self.chord_idx]);

        text(px + 10, 166, C_HINT, "CHORD NOTES:");
        let degree_names = ["Root","3rd","5th","7th"];
        for (i, &n) in c_notes.iter().enumerate() {
            let dn = if i < 4 { degree_names[i] } else { "?" };
            text(px + 10, 182 + i as i32 * 18, C_SEL,
                 &format!("{}: {}", dn, NOTES[n]));
        }

        // Chord voicing as text
        let voicing: String = c_notes.iter().map(|&n| NOTES[n]).collect::<Vec<_>>().join(" - ");
        text(px + 10, 260, C_ORANGE, &voicing);

        text(px + 10, H - 88, C_HINT, "F = focus toggle");
        text(px + 10, H - 72, C_HINT, "<> = change");
    }

    fn draw_quiz(&self) {
        let q = &self.quiz;
        // Header
        fill(12, 50, W - 24, 60, C_CARD);
        border(12, 50, W - 24, 60, C_BORDER);

        let root_note = NOTES[q.root];
        let interval_name = INTERVAL_NAMES[q.interval.min(12)];
        let question = format!("What is a {} above {}?", interval_name, root_note);
        text(20, 60, C_TEXT, &question);
        text(20, 78, C_HINT, "Select the correct note:");

        // Score bar
        let sc = if q.score < 0 { C_RED } else { C_GREEN };
        text(W - 220, 60, sc, &format!("Score: {}", q.score));
        text(W - 220, 78, C_ORANGE, &format!("Streak: {}/{}", q.streak, q.best_streak));
        let pct = if q.total > 0 { q.correct_total * 100 / q.total } else { 0 };
        text(W - 220, 96, C_HINT, &format!("{}/{} ({}%)", q.correct_total, q.total, pct));

        // Options (A B C D)
        let labels = ['A', 'B', 'C', 'D'];
        let correct_note = (q.root + q.interval) % 12;
        for (i, &opt_note) in q.options.iter().enumerate() {
            let oy = 140 + i as i32 * 80;
            let is_correct = opt_note == correct_note;
            let bg = match q.answered {
                Some(ai) if ai == i && is_correct => 0x1C3A1CFF,
                Some(ai) if ai == i && !is_correct => 0x3A1C1CFF,
                Some(_) if is_correct => 0x1C3A1CFF,
                _ => C_CARD,
            };
            let bc = match q.answered {
                Some(_) if is_correct => C_GREEN,
                Some(ai) if ai == i => C_RED,
                _ => C_BORDER,
            };
            fill(40, oy, W - 80, 68, bg);
            border(40, oy, W - 80, 68, bc);
            text(56, oy + 12, C_ORANGE, &format!("{}.", labels[i]));
            text(80, oy + 12, C_TEXT, NOTES[opt_note]);
            // Show interval info
            let intervals_from_root = (opt_note + 12 - q.root) % 12;
            let iname = if intervals_from_root <= 12 { INTERVAL_NAMES[intervals_from_root] } else { "?" };
            text(120, oy + 12, C_HINT, &format!("({} semitones - {})", intervals_from_root, iname));

            // Note wheel mini-preview
            if let Some(ai) = q.answered {
                if ai == i || is_correct {
                    text(80, oy + 36, if is_correct { C_GREEN } else { C_RED },
                         if is_correct { "Correct!" } else { "Wrong" });
                }
            }
        }

        // Bottom hints
        text(40, H - 24, C_HINT, "A/B/C/D = answer   Tab = change tab");
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" {
            let advanced = self.quiz.tick();
            if self.tab == Tab::Quiz { self.draw(); }
            if advanced || self.tab != Tab::Quiz {
                println!("@supervisor: ping");
            } else {
                println!("@supervisor: ping");
            }
            return;
        }

        match line {
            "\t" => {
                self.tab = match self.tab {
                    Tab::Scales => Tab::Chords,
                    Tab::Chords => Tab::Quiz,
                    Tab::Quiz   => Tab::Scales,
                };
                self.draw();
            }
            "f" | "F" => {
                self.focus = 1 - self.focus;
                self.draw();
            }
            "\x1b[C" | ">" => { // Right — increase
                match self.tab {
                    Tab::Scales => {
                        if self.focus == 0 { self.scale_root = (self.scale_root + 1) % 12; }
                        else { self.scale_idx = (self.scale_idx + 1) % 8; }
                    }
                    Tab::Chords => {
                        if self.focus == 0 { self.chord_root = (self.chord_root + 1) % 12; }
                        else { self.chord_idx = (self.chord_idx + 1) % 7; }
                    }
                    Tab::Quiz => {}
                }
                self.draw();
            }
            "\x1b[D" | "<" => { // Left — decrease
                match self.tab {
                    Tab::Scales => {
                        if self.focus == 0 { self.scale_root = (self.scale_root + 11) % 12; }
                        else { self.scale_idx = (self.scale_idx + 7) % 8; }
                    }
                    Tab::Chords => {
                        if self.focus == 0 { self.chord_root = (self.chord_root + 11) % 12; }
                        else { self.chord_idx = (self.chord_idx + 6) % 7; }
                    }
                    Tab::Quiz => {}
                }
                self.draw();
            }
            "\x1b[A" => { // Up
                match self.tab {
                    Tab::Scales => { self.scale_idx = (self.scale_idx + 7) % 8; }
                    Tab::Chords => { self.chord_idx = (self.chord_idx + 6) % 7; }
                    Tab::Quiz   => {}
                }
                self.draw();
            }
            "\x1b[B" => { // Down
                match self.tab {
                    Tab::Scales => { self.scale_idx = (self.scale_idx + 1) % 8; }
                    Tab::Chords => { self.chord_idx = (self.chord_idx + 1) % 7; }
                    Tab::Quiz   => {}
                }
                self.draw();
            }
            "a" | "A" if self.tab == Tab::Quiz => { self.quiz.answer(0); self.draw(); }
            "b" | "B" if self.tab == Tab::Quiz => { self.quiz.answer(1); self.draw(); }
            "c" | "C" if self.tab == Tab::Quiz => { self.quiz.answer(2); self.draw(); }
            "d" | "D" if self.tab == Tab::Quiz => { self.quiz.answer(3); self.draw(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
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
        let _ = io::stdout().flush();
    }
}
