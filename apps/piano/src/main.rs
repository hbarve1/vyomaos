// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 520;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_WHITE: u32  = 0xF0F0F0FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_BLACK: u32  = 0x1A1A1AFF;

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

// Note layout for 2 octaves C3–B4
// Each octave: C D E F G A B  (7 white keys)
// Black keys:  C# D# _ F# G# A# _  (5 black keys, no black after E and B)
// Pattern indices (0=white, 1=black, position within octave):
// C=0w C#=0b D=1w D#=1b E=2w F=3w F#=3b G=4w G#=4b A=5w A#=5b B=6w

const NOTE_NAMES: &[&str] = &[
    "C3","C#3","D3","D#3","E3","F3","F#3","G3","G#3","A3","A#3","B3",
    "C4","C#4","D4","D#4","E4","F4","F#4","G4","G#4","A4","A#4","B4",
];

// For each note 0-23: (white_index, is_black, black_offset_from_white)
// white_index: position in the 14-white-key sequence (0-13)
// is_black: bool
// black_x_offset: pixel offset relative to the white key to the left

fn note_layout(note: usize) -> (bool, i32) {
    // note % 12 position within octave
    let oct = note / 12;
    let pos = note % 12;
    // white key indices within octave: C=0 D=1 E=2 F=3 G=4 A=5 B=6
    // chromatic: C C# D D# E F F# G G# A A# B
    //             0  1  2  3  4 5  6  7  8  9 10 11
    let (is_black, white_left): (bool, usize) = match pos {
        0  => (false, 0),
        1  => (true,  0),
        2  => (false, 1),
        3  => (true,  1),
        4  => (false, 2),
        5  => (false, 3),
        6  => (true,  3),
        7  => (false, 4),
        8  => (true,  4),
        9  => (false, 5),
        10 => (true,  5),
        11 => (false, 6),
        _  => unreachable!(),
    };
    let white_global = oct * 7 + white_left;
    (is_black, white_global as i32)
}

// Qwerty piano key mapping: z-m = C3-B3, a-k = C4-B4
// z x c v b n m = C3 D3 E3 F3 G3 A3 B3
// a s d f g h j = C4 D4 E4 F4 G4 A4 B4
// black keys: q w e r t = C#3 D#3 F#3 G#3 A#3
//             1 2 3 4 5 = C#4 D#4 F#4 G#4 A#4 (using row above a)
// Simplified: map each key char to note index 0-23

fn char_to_note(c: u8) -> Option<usize> {
    match c {
        b'z' => Some(0),   // C3
        b'x' => Some(2),   // D3
        b'c' => Some(4),   // E3
        b'v' => Some(5),   // F3
        b'b' => Some(7),   // G3
        b'n' => Some(9),   // A3
        b'm' => Some(11),  // B3
        b'a' => Some(12),  // C4
        b's' => Some(14),  // D4
        b'd' => Some(16),  // E4
        b'f' => Some(17),  // F4
        b'g' => Some(19),  // G4
        b'h' => Some(21),  // A4
        b'j' => Some(23),  // B4
        // Black keys
        b'q' => Some(1),   // C#3
        b'w' => Some(3),   // D#3
        b'e' => Some(6),   // F#3
        b'r' => Some(8),   // G#3
        b't' => Some(10),  // A#3
        b'y' => Some(13),  // C#4
        b'u' => Some(15),  // D#4
        b'i' => Some(18),  // F#4
        b'o' => Some(20),  // G#4
        b'p' => Some(22),  // A#4
        _ => None,
    }
}

const KEY_W: i32 = 56;  // white key width
const KEY_H: i32 = 160; // white key height
const BK_W: i32  = 34;  // black key width
const BK_H: i32  = 96;  // black key height
const KY: i32    = 100; // top of keyboard
const KX: i32    = 20;  // left margin

struct App {
    active:   Option<usize>,
    last_note: Option<usize>,
    demo:     bool,
    seed:     u64,
    history:  Vec<usize>,
}

impl App {
    fn new() -> Self {
        App { active: None, last_note: None, demo: false, seed: 0xABCDEF0123456789, history: Vec::new() }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Virtual Piano");
        text(160, 8, C_HINT, "z-m/a-j:white  q-t/y-p:black  D:demo  Q:quit");

        // Keyboard background
        fill(KX - 4, KY - 4, 14 * KEY_W + 8, KEY_H + 8, C_CARD);
        border(KX - 4, KY - 4, 14 * KEY_W + 8, KEY_H + 8, C_BORDER);

        // Draw white keys first
        for note in 0..24usize {
            let (is_black, wi) = note_layout(note);
            if is_black { continue; }
            let x = KX + wi * KEY_W;
            let active = self.active == Some(note);
            let bg = if active { C_SEL } else { C_WHITE };
            fill(x + 1, KY + 1, KEY_W - 2, KEY_H - 2, bg);
            border(x, KY, KEY_W, KEY_H, C_BORDER);
            // Note label at bottom
            let tc = if active { 0x000000FF } else { C_HINT };
            text(x + 16, KY + KEY_H - 20, tc, &NOTE_NAMES[note][..2]);
        }

        // Draw black keys on top
        for note in 0..24usize {
            let (is_black, wi) = note_layout(note);
            if !is_black { continue; }
            let x = KX + wi * KEY_W + KEY_W - BK_W / 2;
            let active = self.active == Some(note);
            let bg = if active { C_SEL } else { C_BLACK };
            fill(x, KY, BK_W, BK_H, bg);
            border(x, KY, BK_W, BK_H, C_BORDER);
        }

        // Info panel
        let py = KY + KEY_H + 16;
        fill(KX - 4, py, W - KX * 2 + 8, 100, C_CARD);
        border(KX - 4, py, W - KX * 2 + 8, 100, C_BORDER);

        let demo_str = if self.demo { "DEMO ON" } else { "DEMO OFF" };
        let dc = if self.demo { C_GREEN } else { C_HINT };
        text(KX + 4, py + 8, dc, demo_str);

        if let Some(n) = self.active {
            text(KX + 4, py + 30, C_ORANGE, &format!("Playing: {}", NOTE_NAMES[n]));
        } else if let Some(n) = self.last_note {
            text(KX + 4, py + 30, C_HINT, &format!("Last: {}", NOTE_NAMES[n]));
        }

        // Key mapping hints
        text(KX + 4, py + 52, C_HINT, "White: z x c v b n m | a s d f g h j");
        text(KX + 4, py + 70, C_HINT, "Black: q w e r t     | y u i o p");

        // History strip
        let hx = KX + 300;
        text(hx, py + 8, C_ORANGE, "Recent:");
        for (i, &n) in self.history.iter().rev().take(10).enumerate() {
            let (is_black, _) = note_layout(n);
            let tc = if is_black { C_SEL } else { C_TEXT };
            text(hx + 64 + i as i32 * 40, py + 8, tc, NOTE_NAMES[n]);
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        let cn = self.active.map(|n| NOTE_NAMES[n]).unwrap_or("—");
        text(12, H - 18, C_HINT, &format!("Note: {}  Demo: {}", cn, if self.demo { "on" } else { "off" }));

        flush();
    }

    fn press(&mut self, note: usize) {
        self.active = Some(note);
        self.last_note = Some(note);
        self.history.push(note);
        if self.history.len() > 20 { self.history.remove(0); }
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" {
            if self.demo {
                self.seed = lcg(self.seed);
                let n = (self.seed >> 32) as usize % 24;
                self.press(n);
            } else {
                self.active = None;
            }
            self.draw();
            return;
        }
        match line {
            "d" | "D" => {
                self.demo = !self.demo;
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {
                if line.len() == 1 {
                    let b = line.as_bytes()[0];
                    if let Some(note) = char_to_note(b) {
                        self.press(note);
                    } else {
                        self.active = None;
                    }
                } else {
                    self.active = None;
                }
            }
        }
        self.draw();
        if self.demo {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        if line == "REPLY:pong" {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }
    }
}
