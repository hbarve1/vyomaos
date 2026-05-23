use std::io::{self, BufRead, Write};

const W: i32 = 900;
const H: i32 = 640;

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
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}

const MORSE: [(&str, &str); 36] = [
    ("A",".-"),   ("B","-..."), ("C","-.-."), ("D","-.."),  ("E","."),
    ("F","..-."), ("G","--."),  ("H","...."), ("I",".."),   ("J",".---"),
    ("K","-.-"),  ("L",".-.."), ("M","--"),   ("N","-."),   ("O","---"),
    ("P",".--."), ("Q","--.-"), ("R",".-."),  ("S","..."),  ("T","-"),
    ("U","..-"),  ("V","...-"), ("W",".--"),  ("X","-..-"), ("Y","-.--"),
    ("Z","--.."), ("0","-----"),("1",".----"),("2","..---"),("3","...--"),
    ("4","....-"),("5","....."),("6","-...."),("7","--..."),("8","---.."),
    ("9","----."),
];

enum Mode { Practice, Decode }

enum Feedback { None, Correct, Wrong(String) }

struct App {
    mode:        Mode,
    challenge:   usize,
    input:       String,
    correct:     usize,
    total:       usize,
    streak:      usize,
    best_streak: usize,
    feedback:    Feedback,
    seed:        u64,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            mode: Mode::Practice, challenge: 0, input: String::new(),
            correct: 0, total: 0, streak: 0, best_streak: 0,
            feedback: Feedback::None, seed: 0xC0DE_5AFE_1234u64,
        };
        a.next_challenge();
        a
    }

    fn next_challenge(&mut self) {
        self.seed = lcg(self.seed);
        self.challenge = ((self.seed >> 33) as usize) % MORSE.len();
        self.input.clear();
        self.feedback = Feedback::None;
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Morse Code Trainer");
        text(W - 360, 8, C_HINT, "Tab=mode  N=skip  Q=quit");

        // Left sidebar — Morse table
        let sw = 280i32;
        fill(8, 36, sw, H - 60, C_CARD);
        border(8, 36, sw, H - 60, C_BORDER);
        text(16, 44, C_HINT, "MORSE TABLE");

        for i in 0..36usize {
            let (sym, code) = MORSE[i];
            let col = (i / 18) as i32;
            let row = (i % 18) as i32;
            let tx = 16 + col * 132;
            let ty = 62 + row * 18;
            let is_cur = i == self.challenge;
            if is_cur { fill(tx - 2, ty - 2, 128, 16, C_HEADER); }
            text(tx,      ty, if is_cur { C_SEL  } else { C_HINT }, sym);
            text(tx + 14, ty, if is_cur { C_TEXT } else { C_HINT }, code);
        }

        // Right panel
        let px = sw + 16;
        let pw = W - px - 8;
        fill(px, 36, pw, H - 60, C_CARD);
        border(px, 36, pw, H - 60, C_BORDER);

        let (mode_label, mode_c) = match self.mode {
            Mode::Practice => ("PRACTICE MODE", C_ORANGE),
            Mode::Decode   => ("DECODE MODE",   C_SEL),
        };
        text(px + 12, 44, mode_c, mode_label);
        let mode_desc = match self.mode {
            Mode::Practice => "Type the Morse code using . and -, then Enter",
            Mode::Decode   => "Look up the Morse code and type the letter/digit",
        };
        text(px + 12, 62, C_HINT, mode_desc);

        // Challenge card
        let (sym, code) = MORSE[self.challenge];
        let challenge_text = match self.mode { Mode::Practice => sym, Mode::Decode => code };
        let card_border_c = match self.mode { Mode::Practice => C_ORANGE, Mode::Decode => C_SEL };
        fill(px + 12, 84, pw - 24, 56, C_HEADER);
        border(px + 12, 84, pw - 24, 56, card_border_c);
        let card_cx = px + 12 + (pw - 24) / 2;
        text(card_cx - challenge_text.len() as i32 * 4, 104, C_TEXT, challenge_text);

        // Input area
        text(px + 12, 152, C_HINT, "Your answer:");
        fill(px + 12, 168, pw - 24, 30, C_HEADER);
        border(px + 12, 168, pw - 24, 30, C_BORDER);
        let (inp_text, inp_c) = if self.input.is_empty() {
            let hint = match self.mode {
                Mode::Practice => "type . and - then Enter",
                Mode::Decode   => "type a letter or digit",
            };
            (hint, C_HINT)
        } else {
            (self.input.as_str(), C_TEXT)
        };
        text(px + 20, 176, inp_c, inp_text);

        // Feedback line
        match &self.feedback {
            Feedback::None => {}
            Feedback::Correct => { text(px + 12, 210, C_GREEN, "Correct!  (any key for next)"); }
            Feedback::Wrong(ans) => {
                let msg = format!("Wrong!  Answer: {}  (any key for next)", ans);
                let max_c = (pw - 24) as usize / 8;
                let show = if msg.len() > max_c { &msg[..max_c] } else { &msg };
                text(px + 12, 210, C_RED, show);
            }
        }

        // Score panels
        let sp_y = 238i32;
        let half_w = (pw - 28) / 2;
        fill(px + 12, sp_y, half_w, 64, C_HEADER);
        border(px + 12, sp_y, half_w, 64, C_BORDER);
        text(px + 20, sp_y + 8, C_HINT, "SCORE");
        let pct = if self.total > 0 { self.correct * 100 / self.total } else { 0 };
        let sc = if pct > 79 { C_GREEN } else if pct > 59 { C_ORANGE } else { C_HINT };
        text(px + 20, sp_y + 28, sc, &format!("{}/{} ({}%)", self.correct, self.total, pct));

        let sp2_x = px + 16 + half_w;
        fill(sp2_x, sp_y, half_w, 64, C_HEADER);
        border(sp2_x, sp_y, half_w, 64, C_BORDER);
        text(sp2_x + 8, sp_y + 8, C_HINT, "STREAK");
        let stc = if self.streak > 5 { C_GREEN } else if self.streak > 2 { C_ORANGE } else { C_HINT };
        text(sp2_x + 8, sp_y + 28, stc, &format!("{}  (best: {})", self.streak, self.best_streak));

        // Tip
        let tip_y = sp_y + 74;
        fill(px + 12, tip_y, pw - 24, 22, C_HEADER);
        let tip = match self.mode {
            Mode::Practice => ". = dit (short)   - = dah (long)   Enter to submit",
            Mode::Decode   => "Find the symbol in the table on the left, then type it",
        };
        text(px + 20, tip_y + 4, C_HINT, tip);

        text(px + 12, tip_y + 34, C_HINT,
             &format!("Current challenge: {} = {}", sym, code));

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("{} | {}/{} correct ({}%) | Streak: {} | Best: {}",
                 mode_label, self.correct, self.total, pct, self.streak, self.best_streak));

        flush();
    }

    fn submit(&mut self) {
        let (sym, code) = MORSE[self.challenge];
        let expected = match self.mode { Mode::Practice => code, Mode::Decode => sym };
        self.total += 1;
        if self.input.to_uppercase() == expected.to_uppercase() {
            self.correct += 1;
            self.streak += 1;
            self.best_streak = self.best_streak.max(self.streak);
            self.feedback = Feedback::Correct;
        } else {
            self.streak = 0;
            self.feedback = Feedback::Wrong(expected.to_string());
        }
        self.draw();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\t" => {
                self.mode = match self.mode { Mode::Practice => Mode::Decode, Mode::Decode => Mode::Practice };
                self.next_challenge(); self.draw(); return;
            }
            "n" | "N" => { self.next_challenge(); self.draw(); return; }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }

        if !matches!(self.feedback, Feedback::None) {
            self.next_challenge(); self.draw(); return;
        }

        match line {
            "\x7f" => {
                if matches!(self.mode, Mode::Practice) && !self.input.is_empty() {
                    self.input.pop(); self.draw();
                }
            }
            "" => {
                if matches!(self.mode, Mode::Practice) && !self.input.is_empty() {
                    self.submit();
                }
            }
            s if (s == "." || s == "-") && matches!(self.mode, Mode::Practice) => {
                if self.input.len() < 7 { self.input.push_str(s); }
                self.draw();
            }
            s if s.len() == 1 && !s.starts_with('\x1b') && matches!(self.mode, Mode::Decode) => {
                self.input = s.to_uppercase();
                self.submit();
            }
            _ => {}
        }
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
