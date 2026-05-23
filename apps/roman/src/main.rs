use std::io::{self, BufRead, Write};

const W: i32 = 800;
const H: i32 = 600;

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

const ROMAN_VALS: &[(u32, &str)] = &[
    (1000,"M"),(900,"CM"),(500,"D"),(400,"CD"),
    (100,"C"),(90,"XC"),(50,"L"),(40,"XL"),
    (10,"X"),(9,"IX"),(5,"V"),(4,"IV"),(1,"I"),
];

fn to_roman(mut n: u32) -> String {
    let mut s = String::new();
    for &(val, sym) in ROMAN_VALS {
        while n >= val { s.push_str(sym); n -= val; }
    }
    s
}

fn roman_val(c: char) -> u32 {
    match c { 'I'|'i' => 1, 'V'|'v' => 5, 'X'|'x' => 10, 'L'|'l' => 50,
               'C'|'c' => 100, 'D'|'d' => 500, 'M'|'m' => 1000, _ => 0 }
}

fn from_roman(s: &str) -> Option<u32> {
    if s.is_empty() { return None; }
    let up = s.to_uppercase();
    let chars: Vec<char> = up.chars().collect();
    if chars.iter().any(|&c| roman_val(c) == 0) { return None; }
    let mut total = 0u32;
    for i in 0..chars.len() {
        let cur  = roman_val(chars[i]);
        let next = if i + 1 < chars.len() { roman_val(chars[i+1]) } else { 0 };
        if cur < next { total = total.saturating_sub(cur); } else { total += cur; }
    }
    if total >= 1 && total <= 3999 { Some(total) } else { None }
}

fn convert(input: &str) -> Option<String> {
    if input.is_empty() { return None; }
    if input.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        let n: u32 = input.parse().ok()?;
        if n >= 1 && n <= 3999 { Some(to_roman(n)) } else { None }
    } else {
        from_roman(input).map(|n| format!("{}", n))
    }
}

enum Mode { Converter, Quiz }

struct App {
    mode:        Mode,
    input:       String,
    result:      String,   // last conversion result
    history:     Vec<(String, String)>,
    quiz_num:    u32,
    quiz_roman:  bool, // false=show arabic→type roman, true=show roman→type arabic
    correct:     u32,
    total:       u32,
    streak:      u32,
    best_streak: u32,
    feedback:    Option<bool>, // Some(true)=correct, Some(false)=wrong, None=none
    wrong_ans:   String,
    seed:        u64,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            mode: Mode::Converter,
            input: String::new(), result: String::new(),
            history: Vec::new(),
            quiz_num: 0, quiz_roman: false,
            correct: 0, total: 0, streak: 0, best_streak: 0,
            feedback: None, wrong_ans: String::new(),
            seed: 0xC0DE_5AFE_ABCD_1234u64,
        };
        a.next_quiz();
        a
    }

    fn next_quiz(&mut self) {
        self.seed = lcg(self.seed);
        self.quiz_num = ((self.seed >> 33) as u32) % 3999 + 1;
        self.seed = lcg(self.seed);
        self.quiz_roman = (self.seed >> 63) == 1;
        self.input.clear();
        self.feedback = None;
        self.wrong_ans.clear();
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Roman Numerals");
        text(200, 8, C_HINT, "Tab:mode  Q:quit");

        // Mode tabs
        let tabs = [("Converter", matches!(self.mode, Mode::Converter)),
                    ("Quiz",      matches!(self.mode, Mode::Quiz))];
        for (i, &(label, active)) in tabs.iter().enumerate() {
            let tx = 650 + i as i32 * 80;
            if active {
                fill(tx - 4, 4, label.len() as i32 * 8 + 8, 24, C_CARD);
                border(tx - 4, 4, label.len() as i32 * 8 + 8, 24, C_SEL);
                text(tx, 10, C_SEL, label);
            } else {
                text(tx, 10, C_HINT, label);
            }
        }

        match self.mode {
            Mode::Converter => self.draw_converter(),
            Mode::Quiz      => self.draw_quiz(),
        }

        fill(0, H - 24, W, 24, C_HEADER);
        let status = match self.mode {
            Mode::Converter => format!("Converter | type arabic(1-3999) or roman → Enter | history: {}", self.history.len()),
            Mode::Quiz      => format!("Quiz | {}/{} correct | Streak: {} | Best: {}", self.correct, self.total, self.streak, self.best_streak),
        };
        text(12, H - 18, C_HINT, &status);
        flush();
    }

    fn draw_converter(&self) {
        // Input area
        fill(24, 44, W - 48, 44, C_CARD);
        border(24, 44, W - 48, 44, C_BORDER);
        text(32, 50, C_HINT, "Input (arabic or roman):");
        let inp = if self.input.is_empty() { "type here and press Enter" } else { &self.input };
        let inp_c = if self.input.is_empty() { C_HINT } else { C_TEXT };
        text(32, 66, inp_c, inp);

        // Conversion arrow and result
        if !self.result.is_empty() {
            text(W / 2 - 4, 104, C_HINT, "↓");
            // Large result display
            fill(24, 116, W - 48, 80, C_CARD);
            border(24, 116, W - 48, 80, C_ORANGE);
            let res_x = W / 2 - self.result.len() as i32 * 8 / 2;
            text(res_x, 148, C_ORANGE, &self.result);
        }

        // Roman numeral reference
        fill(24, 210, W - 48, 2, C_BORDER);
        text(24, 218, C_HINT, "Reference:");
        let ref_vals = [("I=1","V=5","X=10","L=50"),("C=100","D=500","M=1000","max=3999")];
        for (i, &(a, b, c, d)) in ref_vals.iter().enumerate() {
            let ry = 234 + i as i32 * 18;
            text(24, ry, C_HINT, a);
            text(100, ry, C_HINT, b);
            text(180, ry, C_HINT, c);
            text(268, ry, C_HINT, d);
        }
        text(24, 270, C_HINT, "Subtractive: IV=4 IX=9 XL=40 XC=90 CD=400 CM=900");

        // History panel
        fill(24, 300, W - 48, H - 328, C_CARD);
        border(24, 300, W - 48, H - 328, C_BORDER);
        text(32, 308, C_HINT, "History (last 10):");
        for (i, (inp, res)) in self.history.iter().rev().take(10).enumerate() {
            let hy = 324 + i as i32 * 18;
            text(32, hy, C_HINT, &format!("{}.", i + 1));
            text(56, hy, C_TEXT, inp);
            text(200, hy, C_HINT, "→");
            text(224, hy, C_ORANGE, res);
        }
    }

    fn draw_quiz(&self) {
        let challenge = if self.quiz_roman {
            to_roman(self.quiz_num)
        } else {
            format!("{}", self.quiz_num)
        };
        let prompt = if self.quiz_roman {
            "Convert to Arabic:"
        } else {
            "Convert to Roman:"
        };

        // Challenge card
        fill(24, 44, W - 48, 100, C_CARD);
        border(24, 44, W - 48, 100, C_SEL);
        text(32, 52, C_HINT, prompt);
        let ch_x = W / 2 - challenge.len() as i32 * 8 / 2;
        text(ch_x, 80, C_TEXT, &challenge);
        text(32, 112, C_HINT, &format!("#{} of ∞", self.total + 1));

        // Input area
        fill(24, 156, W - 48, 36, C_CARD);
        border(24, 156, W - 48, 36, C_BORDER);
        text(32, 162, C_HINT, "Answer:");
        let inp = if self.input.is_empty() { "type your answer and press Enter" } else { &self.input };
        let inp_c = if self.input.is_empty() { C_HINT } else { C_TEXT };
        text(120, 162, inp_c, inp);

        // Feedback
        if let Some(ok) = self.feedback {
            let (fb_msg, fb_c) = if ok {
                ("Correct!  (any key for next)", C_GREEN)
            } else {
                let msg = format!("Wrong!  Answer: {}  (any key for next)", self.wrong_ans);
                (msg, C_RED)
            };
            let fb_str: &str = &fb_msg;
            text(32, 208, fb_c, fb_str);
        }

        // Score panel
        let sp_y = 238i32;
        let half = (W - 52) / 2;
        fill(24, sp_y, half, 64, C_CARD);
        border(24, sp_y, half, 64, C_BORDER);
        text(32, sp_y + 8, C_HINT, "SCORE");
        let pct = if self.total > 0 { self.correct * 100 / self.total } else { 0 };
        let sc = if pct > 79 { C_GREEN } else if pct > 59 { C_ORANGE } else { C_HINT };
        text(32, sp_y + 28, sc, &format!("{}/{} ({}%)", self.correct, self.total, pct));

        let sp2_x = 28 + half;
        fill(sp2_x, sp_y, half, 64, C_CARD);
        border(sp2_x, sp_y, half, 64, C_BORDER);
        text(sp2_x + 8, sp_y + 8, C_HINT, "STREAK");
        let stc = if self.streak > 5 { C_GREEN } else if self.streak > 2 { C_ORANGE } else { C_HINT };
        text(sp2_x + 8, sp_y + 28, stc, &format!("{}  (best: {})", self.streak, self.best_streak));

        // Mode indicator
        fill(24, sp_y + 74, W - 48, 22, C_CARD);
        let mode_txt = if self.quiz_roman {
            "Mode: Roman → Arabic   (N=skip)"
        } else {
            "Mode: Arabic → Roman   (N=skip)"
        };
        text(32, sp_y + 80, C_HINT, mode_txt);

        // Recent history
        if !self.history.is_empty() {
            fill(24, sp_y + 106, W - 48, H - sp_y - 130, C_CARD);
            border(24, sp_y + 106, W - 48, H - sp_y - 130, C_BORDER);
            text(32, sp_y + 114, C_HINT, "Recent:");
            for (i, (q, a)) in self.history.iter().rev().take(5).enumerate() {
                let hy = sp_y + 130 + i as i32 * 18;
                text(32, hy, C_HINT, q);
                text(220, hy, C_HINT, "→");
                text(244, hy, C_TEXT, a);
            }
        }
    }

    fn submit_converter(&mut self) {
        if self.input.is_empty() { return; }
        if let Some(r) = convert(&self.input) {
            self.result = r.clone();
            if self.history.len() >= 10 { self.history.remove(0); }
            self.history.push((self.input.clone(), r));
        } else {
            self.result = "Invalid input".to_string();
        }
        self.input.clear();
    }

    fn submit_quiz(&mut self) {
        let expected = if self.quiz_roman {
            format!("{}", self.quiz_num)
        } else {
            to_roman(self.quiz_num)
        };
        let correct = self.input.to_uppercase() == expected.to_uppercase();
        self.total += 1;
        if correct {
            self.correct += 1;
            self.streak += 1;
            self.best_streak = self.best_streak.max(self.streak);
            self.feedback = Some(true);
        } else {
            self.streak = 0;
            self.feedback = Some(false);
            self.wrong_ans = expected.clone();
        }
        if self.history.len() >= 10 { self.history.remove(0); }
        let q = if self.quiz_roman { to_roman(self.quiz_num) } else { format!("{}", self.quiz_num) };
        self.history.push((q, expected));
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\t" => {
                self.mode = match self.mode { Mode::Converter => Mode::Quiz, Mode::Quiz => Mode::Converter };
                self.input.clear();
                self.draw();
                return;
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }

        match self.mode {
            Mode::Converter => {
                match line {
                    "\x7f" => { self.input.pop(); }
                    ""     => { self.submit_converter(); }
                    s if s.len() == 1 => {
                        let b = s.as_bytes()[0];
                        if (b.is_ascii_alphanumeric() || b == b'-') && self.input.len() < 16 {
                            self.input.push_str(s);
                        }
                    }
                    _ => {}
                }
            }
            Mode::Quiz => {
                if self.feedback.is_some() {
                    match line {
                        "n" | "N" => {}
                        _ => {}
                    }
                    self.next_quiz();
                    self.draw();
                    return;
                }
                match line {
                    "\x7f" => { self.input.pop(); }
                    "n" | "N" => { self.next_quiz(); self.draw(); return; }
                    ""     => {
                        if !self.input.is_empty() { self.submit_quiz(); }
                    }
                    s if s.len() == 1 => {
                        let b = s.as_bytes()[0];
                        if b.is_ascii_alphanumeric() && self.input.len() < 12 {
                            self.input.push_str(s);
                        }
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
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
