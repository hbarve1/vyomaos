// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use super::theory::{lcg, scale_notes, chord_notes, draw_note_wheel};
use super::{
    NOTES, SCALE_NAMES, CHORD_NAMES, INTERVAL_NAMES,
    C_BG, C_HEADER, C_BORDER, C_TEXT, C_HINT, C_ORANGE, C_SEL, C_GREEN, C_RED,
    C_YELLOW, C_CARD, W, H,
};

#[derive(PartialEq, Clone, Copy)]
pub enum Tab { Scales, Chords, Quiz }

pub struct QuizState {
    pub root: usize,
    pub interval: usize,
    pub options: [usize; 4],
    pub correct_idx: usize,
    pub answered: Option<usize>,
    pub score: i32,
    pub streak: u32,
    pub best_streak: u32,
    pub total: u32,
    pub correct_total: u32,
    pub feedback_ticks: u8,
    pub seed: u64,
}

impl QuizState {
    pub fn new(seed: u64) -> Self {
        let mut qs = QuizState {
            root: 0, interval: 0, options: [0; 4], correct_idx: 0,
            answered: None, score: 0, streak: 0, best_streak: 0,
            total: 0, correct_total: 0, feedback_ticks: 0, seed,
        };
        qs.next_question();
        qs
    }

    pub fn next_question(&mut self) {
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

    pub fn answer(&mut self, idx: usize) {
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

    pub fn tick(&mut self) -> bool {
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

pub struct App {
    pub tab: Tab,
    pub scale_root: usize,
    pub scale_idx: usize,
    pub chord_root: usize,
    pub chord_idx: usize,
    pub focus: u8, // 0=root, 1=type
    pub quiz: QuizState,
}

impl App {
    pub fn new() -> Self {
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

    pub fn draw(&self) {
        super::fill(0, 0, W, H, C_BG);
        super::fill(0, 0, W, 32, C_HEADER);
        super::text(12, 8, C_TEXT, "Music Theory");

        // Tabs
        let tabs = [("Scales", Tab::Scales), ("Chords", Tab::Chords), ("Interval Quiz", Tab::Quiz)];
        let mut tx = W - 320;
        for (name, t) in &tabs {
            let active = self.tab == *t;
            let bg = if active { C_SEL } else { C_CARD };
            let tc = if active { C_BG } else { C_HINT };
            let tw = name.len() as i32 * 8 + 16;
            super::fill(tx, 4, tw, 24, bg);
            super::border(tx, 4, tw, 24, C_BORDER);
            super::text(tx + 8, 10, tc, name);
            tx += tw + 4;
        }
        super::text(12, 8, C_HINT, "Tab=cycle");

        match self.tab {
            Tab::Scales => self.draw_scales(),
            Tab::Chords => self.draw_chords(),
            Tab::Quiz   => self.draw_quiz(),
        }

        super::flush();
    }

    fn draw_scales(&self) {
        let active_notes = scale_notes(self.scale_root, self.scale_idx);
        let cx = 280;
        let cy = 360;
        let r  = 200;
        draw_note_wheel(cx, cy, r, &active_notes, self.scale_root);

        // Controls panel (right side)
        let px = 580;
        super::fill(px, 50, W - px - 12, H - 60, C_CARD);
        super::border(px, 50, W - px - 12, H - 60, C_BORDER);

        super::text(px + 10, 60, C_HINT, "ROOT NOTE");
        // Root selector
        let rsel = self.focus == 0;
        let rbc = if rsel { C_SEL } else { C_BORDER };
        super::border(px + 10, 78, W - px - 32, 22, rbc);
        super::text(px + 14, 82, C_TEXT, NOTES[self.scale_root]);
        super::text(px + 14 + 24, 82, C_HINT, "  <> change");

        super::text(px + 10, 112, C_HINT, "SCALE TYPE");
        let sbc = if self.focus == 1 { C_SEL } else { C_BORDER };
        super::border(px + 10, 130, W - px - 32, 22, sbc);
        super::text(px + 14, 134, C_TEXT, SCALE_NAMES[self.scale_idx]);

        super::text(px + 10, 166, C_HINT, "SCALE NOTES:");
        for (i, &n) in active_notes.iter().enumerate() {
            let degree = ["1","2","3","4","5","6","7","?","?"];
            super::text(px + 10, 182 + i as i32 * 18, C_SEL,
                 &format!("{}: {}", degree[i.min(8)], NOTES[n]));
        }

        super::text(px + 10, H - 120, C_HINT, "NAVIGATION:");
        super::text(px + 10, H - 104, C_HINT, "Tab = change tab");
        super::text(px + 10, H - 88, C_HINT, "F = focus toggle");
        super::text(px + 10, H - 72, C_HINT, "<> = change");
        super::text(px + 10, H - 56, C_HINT, "UP/DN = scroll");
    }

    fn draw_chords(&self) {
        let c_notes = chord_notes(self.chord_root, self.chord_idx);
        let cx = 280;
        let cy = 360;
        let r  = 200;
        draw_note_wheel(cx, cy, r, &c_notes, self.chord_root);

        let px = 580;
        super::fill(px, 50, W - px - 12, H - 60, C_CARD);
        super::border(px, 50, W - px - 12, H - 60, C_BORDER);

        super::text(px + 10, 60, C_HINT, "ROOT NOTE");
        let rbc = if self.focus == 0 { C_SEL } else { C_BORDER };
        super::border(px + 10, 78, W - px - 32, 22, rbc);
        super::text(px + 14, 82, C_TEXT, NOTES[self.chord_root]);

        super::text(px + 10, 112, C_HINT, "CHORD TYPE");
        let cbc = if self.focus == 1 { C_SEL } else { C_BORDER };
        super::border(px + 10, 130, W - px - 32, 22, cbc);
        super::text(px + 14, 134, C_TEXT, CHORD_NAMES[self.chord_idx]);

        super::text(px + 10, 166, C_HINT, "CHORD NOTES:");
        let degree_names = ["Root","3rd","5th","7th"];
        for (i, &n) in c_notes.iter().enumerate() {
            let dn = if i < 4 { degree_names[i] } else { "?" };
            super::text(px + 10, 182 + i as i32 * 18, C_SEL,
                 &format!("{}: {}", dn, NOTES[n]));
        }

        // Chord voicing as text
        let voicing: String = c_notes.iter().map(|&n| NOTES[n]).collect::<Vec<_>>().join(" - ");
        super::text(px + 10, 260, C_ORANGE, &voicing);

        super::text(px + 10, H - 88, C_HINT, "F = focus toggle");
        super::text(px + 10, H - 72, C_HINT, "<> = change");
    }

    fn draw_quiz(&self) {
        let q = &self.quiz;
        // Header
        super::fill(12, 50, W - 24, 60, C_CARD);
        super::border(12, 50, W - 24, 60, C_BORDER);

        let root_note = NOTES[q.root];
        let interval_name = INTERVAL_NAMES[q.interval.min(12)];
        let question = format!("What is a {} above {}?", interval_name, root_note);
        super::text(20, 60, C_TEXT, &question);
        super::text(20, 78, C_HINT, "Select the correct note:");

        // Score bar
        let sc = if q.score < 0 { C_RED } else { C_GREEN };
        super::text(W - 220, 60, sc, &format!("Score: {}", q.score));
        super::text(W - 220, 78, C_ORANGE, &format!("Streak: {}/{}", q.streak, q.best_streak));
        let pct = if q.total > 0 { q.correct_total * 100 / q.total } else { 0 };
        super::text(W - 220, 96, C_HINT, &format!("{}/{} ({}%)", q.correct_total, q.total, pct));

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
            super::fill(40, oy, W - 80, 68, bg);
            super::border(40, oy, W - 80, 68, bc);
            super::text(56, oy + 12, C_ORANGE, &format!("{}.", labels[i]));
            super::text(80, oy + 12, C_TEXT, NOTES[opt_note]);
            // Show interval info
            let intervals_from_root = (opt_note + 12 - q.root) % 12;
            let iname = if intervals_from_root <= 12 { INTERVAL_NAMES[intervals_from_root] } else { "?" };
            super::text(120, oy + 12, C_HINT, &format!("({} semitones - {})", intervals_from_root, iname));

            // Note wheel mini-preview
            if let Some(ai) = q.answered {
                if ai == i || is_correct {
                    super::text(80, oy + 36, if is_correct { C_GREEN } else { C_RED },
                         if is_correct { "Correct!" } else { "Wrong" });
                }
            }
        }

        // Bottom hints
        super::text(40, H - 24, C_HINT, "A/B/C/D = answer   Tab = change tab");
    }

    pub fn handle(&mut self, line: &str) {
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
