use std::io::{self, BufRead, Write};

const W: u32 = 900;
const H: u32 = 640;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;

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
const C_YELLOW: u32 = 0xD29922FF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const MORSE: &[(&str, &str)] = &[
    ("A", ".-"),   ("B", "-..."), ("C", "-.-."), ("D", "-.."),
    ("E", "."),    ("F", "..-."), ("G", "--."),  ("H", "...."),
    ("I", ".."),   ("J", ".---"), ("K", "-.-"),  ("L", ".-.."),
    ("M", "--"),   ("N", "-."),   ("O", "---"),  ("P", ".--."),
    ("Q", "--.-"), ("R", ".-."),  ("S", "..."),  ("T", "-"),
    ("U", "..-"),  ("V", "...-"), ("W", ".--"),  ("X", "-..-"),
    ("Y", "-.--"), ("Z", "--.."),
    ("0", "-----"), ("1", ".----"), ("2", "..---"), ("3", "...--"),
    ("4", "....-"), ("5", "....."), ("6", "-...."), ("7", "--..."),
    ("8", "---.."), ("9", "----."),
];

fn char_to_morse(c: char) -> Option<&'static str> {
    let up = c.to_ascii_uppercase();
    let s = std::str::from_utf8(std::slice::from_ref(&(up as u8))).unwrap_or("");
    MORSE.iter().find(|&&(letter, _)| letter == s).map(|&(_, code)| code)
}

fn morse_to_char(code: &str) -> Option<&'static str> {
    MORSE.iter().find(|&&(_, c)| c == code).map(|&(letter, _)| letter)
}

fn text_to_morse(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c == ' ' {
            out.push('/');
            out.push(' ');
        } else if let Some(m) = char_to_morse(c) {
            if !out.is_empty() && !out.ends_with(' ') { out.push(' '); }
            out.push_str(m);
        }
    }
    out
}

fn morse_to_text(s: &str) -> String {
    let mut out = String::new();
    for word in s.split('/') {
        if !out.is_empty() { out.push(' '); }
        for code in word.split_whitespace() {
            if let Some(c) = morse_to_char(code.trim()) {
                out.push_str(c);
            } else if !code.is_empty() {
                out.push('?');
            }
        }
    }
    out
}

fn morse_display(code: &str) -> String {
    // Replace . with ● and - with ━ for visual display
    code.replace('.', "● ").replace('-', "━━ ")
}

#[derive(PartialEq)]
enum Mode { Encode, Decode, Quiz }

struct App {
    mode:        Mode,
    input:       String,
    quiz_letter: &'static str,
    quiz_morse:  &'static str,
    quiz_input:  String,
    quiz_score:  u32,
    quiz_total:  u32,
    seed:        u64,
    feedback:    Option<(String, u32)>, // (msg, color)
}

impl App {
    fn new() -> Self {
        let mut a = App {
            mode: Mode::Encode,
            input: String::new(),
            quiz_letter: "A",
            quiz_morse: ".-",
            quiz_input: String::new(),
            quiz_score: 0,
            quiz_total: 0,
            seed: 0xA0C5E_12345678ABu64,
            feedback: None,
        };
        a.new_quiz_question();
        a
    }

    fn new_quiz_question(&mut self) {
        self.seed = lcg(self.seed);
        let idx = (self.seed % MORSE.len() as u64) as usize;
        self.quiz_letter = MORSE[idx].0;
        self.quiz_morse = MORSE[idx].1;
        self.quiz_input.clear();
        self.feedback = None;
    }

    fn check_quiz(&mut self) {
        let answer = self.quiz_input.trim().to_string();
        self.quiz_total += 1;
        if answer == self.quiz_morse {
            self.quiz_score += 1;
            self.feedback = Some((format!("Correct! {} = {}", self.quiz_letter, self.quiz_morse), C_GREEN));
        } else {
            self.feedback = Some((format!("Wrong! {} = {}  (you typed: {})", self.quiz_letter, self.quiz_morse, answer), C_RED));
        }
        self.new_quiz_question();
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Morse Code Trainer");

    // Mode tabs
    let tabs = [("Encode", &Mode::Encode), ("Decode", &Mode::Decode), ("Quiz", &Mode::Quiz)];
    for (i, (label, m)) in tabs.iter().enumerate() {
        let tx = 240 + i as u32 * 120;
        let active = &app.mode == *m;
        let col = if active { C_SEL } else { C_HINT };
        if active { fill(tx - 4, 10, label.len() as u32 * CHAR_W + 8, 28, C_CARD); }
        text(tx, 17, col, label);
    }
    text(620, 16, C_HINT, "Tab:switch mode  Ctrl+C:exit");

    let cy = HEADER_H + 16;

    match &app.mode {
        Mode::Encode => {
            text(16, cy, C_HINT, "Type text to encode:");
            let display_input = if app.input.len() > 80 { &app.input[..80] } else { &app.input };
            fill(16, cy + 20, W - 32, 28, C_CARD);
            text(20, cy + 24, C_TEXT, display_input);
            fill(20 + display_input.len() as u32 * CHAR_W, cy + 22, 2, 20, C_SEL);

            let morse = text_to_morse(&app.input);
            text(16, cy + 64, C_HINT, "Morse output:");
            fill(16, cy + 80, W - 32, 32, C_CARD);
            let max_m = (W as usize - 40) / CHAR_W as usize;
            let morse_display_str = if morse.len() > max_m { &morse[..max_m] } else { &morse };
            text(20, cy + 84, C_ORANGE, morse_display_str);

            // Visual dots/dashes
            text(16, cy + 128, C_HINT, "Visual:");
            let vis = morse_display(&morse);
            let max_v = (W as usize - 40) / CHAR_W as usize;
            let vis_d = if vis.len() > max_v { &vis[..max_v] } else { &vis };
            text(20, cy + 144, C_SEL, vis_d);

            // Morse reference table
            text(16, cy + 200, C_HINT, "Reference:");
            for (i, &(letter, code)) in MORSE[..26].iter().enumerate() {
                let col = (i % 9) as u32;
                let row = (i / 9) as u32;
                let rx = 16 + col * 96;
                let ry = cy + 220 + row * 20;
                text(rx, ry, C_HINT, &format!("{}: {}", letter, code));
            }
        }

        Mode::Decode => {
            text(16, cy, C_HINT, "Type morse (. = dot, - = dash, space = letter, / = word):");
            fill(16, cy + 20, W - 32, 28, C_CARD);
            let max_i = (W as usize - 40) / CHAR_W as usize;
            let inp = if app.input.len() > max_i { &app.input[..max_i] } else { &app.input };
            text(20, cy + 24, C_ORANGE, inp);
            fill(20 + inp.len() as u32 * CHAR_W, cy + 22, 2, 20, C_SEL);

            let decoded = morse_to_text(&app.input);
            text(16, cy + 64, C_HINT, "Decoded text:");
            fill(16, cy + 80, W - 32, 40, C_CARD);
            let max_d = (W as usize - 40) / CHAR_W as usize;
            let dec_d = if decoded.len() > max_d { &decoded[..max_d] } else { &decoded };
            text(20, cy + 88, C_GREEN, dec_d);

            text(16, cy + 140, C_HINT, "Input guide: . (dot)  - (dash)  space (next letter)  / (next word)");
            text(16, cy + 164, C_HINT, "Example: .... . .-.. .-.. --- = HELLO");
        }

        Mode::Quiz => {
            text(16, cy, C_HINT, &format!("Score: {}/{}", app.quiz_score, app.quiz_total));

            // Show the letter, ask for morse
            text(16, cy + 30, C_HINT, "What is the Morse code for:");
            let letter_display = format!("  {}  ", app.quiz_letter);
            fill(W / 2 - 40, cy + 52, 80, 48, C_CARD);
            text(W / 2 - 16, cy + 64, C_ORANGE, app.quiz_letter);

            text(16, cy + 120, C_HINT, "Your answer (dots and dashes):");
            fill(16, cy + 140, W - 32, 28, C_CARD);
            let qi = &app.quiz_input;
            text(20, cy + 144, C_SEL, qi);
            fill(20 + qi.len() as u32 * CHAR_W, cy + 142, 2, 20, C_SEL);
            text(16, cy + 180, C_HINT, "Press Enter to check. Space between symbols.");

            if let Some((ref msg, col)) = app.feedback {
                fill(16, cy + 210, W - 32, 28, C_CARD);
                let max_f = (W as usize - 40) / CHAR_W as usize;
                let fd = if msg.len() > max_f { &msg[..max_f] } else { msg.as_str() };
                text(20, cy + 214, col, fd);
            }

            // Score bar
            if app.quiz_total > 0 {
                let pct = app.quiz_score * (W - 32) / app.quiz_total;
                fill(16, cy + 260, W - 32, 12, C_CARD);
                fill(16, cy + 260, pct, 12, C_GREEN);
            }
        }
    }

    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let mode_str = match &app.mode {
        Mode::Encode => "Encode mode — type text, see morse",
        Mode::Decode => "Decode mode — type morse, see text",
        Mode::Quiz   => "Quiz mode — identify morse for shown letter",
    };
    text(8, sb_y + 6, C_HINT, mode_str);
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise morse");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\t" => {
                app.mode = match app.mode {
                    Mode::Encode => Mode::Decode,
                    Mode::Decode => Mode::Quiz,
                    Mode::Quiz   => Mode::Encode,
                };
                app.input.clear();
                app.quiz_input.clear();
                app.feedback = None;
            }
            "\x7f" => {
                match app.mode {
                    Mode::Quiz   => { app.quiz_input.pop(); }
                    _            => { app.input.pop(); }
                }
            }
            "" => {
                if app.mode == Mode::Quiz {
                    app.check_quiz();
                }
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7f {
                    match app.mode {
                        Mode::Quiz   => { app.quiz_input.push(b as char); }
                        _            => { app.input.push(b as char); }
                    }
                }
            }
            _ => {}
        }
        draw(&app);
    }
}
