use std::io::{self, BufRead, Write};

const W: u32 = 840;
const H: u32 = 660;
const HEADER_H: u32 = 48;

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

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn text_l(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},l,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const DIFF_NAMES: [&str; 4] = ["Easy", "Medium", "Hard", "Expert"];
const TIMER_TICKS: u32 = 80; // ~10 seconds

// Generate a question for given difficulty. Returns (new_seed, a, b, op_char, answer)
fn gen_question(seed: u64, diff: usize) -> (u64, i64, i64, char, i64) {
    let mut s = lcg(seed);
    let op = (s >> 33) as usize % 4;
    s = lcg(s);

    let (a, b, answer, op_ch) = match (diff, op) {
        // Easy
        (0, 0) => { let a = (s >> 33) as i64 % 20 + 1; s = lcg(s); let b = (s >> 33) as i64 % 20 + 1; (a, b, a + b, '+') }
        (0, 1) => { let a = (s >> 33) as i64 % 20 + 1; s = lcg(s); let b = (s >> 33) as i64 % (a + 1); (a, b, a - b, '-') }
        (0, 2) => { let a = (s >> 33) as i64 % 8 + 2;  s = lcg(s); let b = (s >> 33) as i64 % 8 + 2;  (a, b, a * b, '×') }
        (0, _) => { let d = (s >> 33) as i64 % 9 + 2; s = lcg(s); let q = (s >> 33) as i64 % 10 + 1; (d * q, d, q, '÷') }
        // Medium
        (1, 0) => { let a = (s >> 33) as i64 % 50 + 1; s = lcg(s); let b = (s >> 33) as i64 % 50 + 1; (a, b, a + b, '+') }
        (1, 1) => { let a = (s >> 33) as i64 % 50 + 10; s = lcg(s); let b = (s >> 33) as i64 % a.min(50) + 1; (a, b, a - b, '-') }
        (1, 2) => { let a = (s >> 33) as i64 % 11 + 2; s = lcg(s); let b = (s >> 33) as i64 % 11 + 2; (a, b, a * b, '×') }
        (1, _) => { let d = (s >> 33) as i64 % 10 + 2; s = lcg(s); let q = (s >> 33) as i64 % 15 + 2; (d * q, d, q, '÷') }
        // Hard
        (2, 0) => { let a = (s >> 33) as i64 % 100 + 1; s = lcg(s); let b = (s >> 33) as i64 % 100 + 1; (a, b, a + b, '+') }
        (2, 1) => { let a = (s >> 33) as i64 % 100 + 20; s = lcg(s); let b = (s >> 33) as i64 % a.min(100) + 1; (a, b, a - b, '-') }
        (2, 2) => { let a = (s >> 33) as i64 % 20 + 3; s = lcg(s); let b = (s >> 33) as i64 % 20 + 3; (a, b, a * b, '×') }
        (2, _) => { let d = (s >> 33) as i64 % 15 + 2; s = lcg(s); let q = (s >> 33) as i64 % 20 + 5; (d * q, d, q, '÷') }
        // Expert
        (_, 0) => { let a = (s >> 33) as i64 % 400 + 100; s = lcg(s); let b = (s >> 33) as i64 % 400 + 100; (a, b, a + b, '+') }
        (_, 1) => { let a = (s >> 33) as i64 % 400 + 100; s = lcg(s); let b = (s >> 33) as i64 % a.min(400) + 1; (a, b, a - b, '-') }
        (_, 2) => { let a = (s >> 33) as i64 % 20 + 10; s = lcg(s); let b = (s >> 33) as i64 % 20 + 10; (a, b, a * b, '×') }
        (_, _) => { let d = (s >> 33) as i64 % 20 + 2; s = lcg(s); let q = (s >> 33) as i64 % 40 + 10; (d * q, d, q, '÷') }
    };
    (s, a, b, op_ch, answer)
}

#[derive(PartialEq)]
enum Phase { Asking, Flashing(bool, u32) } // bool=correct, u32=ticks_left

struct Quiz {
    diff:        usize,
    a:           i64,
    b:           i64,
    op:          char,
    answer:      i64,
    input:       String,
    streak:      u32,
    best:        u32,
    total:       u32,
    correct:     u32,
    timer:       u32,    // ticks_left (counts down)
    phase:       Phase,
    seed:        u64,
}

impl Quiz {
    fn new(seed: u64, diff: usize) -> Self {
        let (s, a, b, op, answer) = gen_question(seed, diff);
        Quiz {
            diff,
            a, b, op, answer,
            input: String::new(),
            streak: 0, best: 0, total: 0, correct: 0,
            timer: TIMER_TICKS,
            phase: Phase::Asking,
            seed: s,
        }
    }

    fn next_question(&mut self) {
        let (s, a, b, op, answer) = gen_question(self.seed, self.diff);
        self.a = a; self.b = b; self.op = op; self.answer = answer;
        self.seed = s;
        self.input.clear();
        self.timer = TIMER_TICKS;
        self.phase = Phase::Asking;
    }

    fn submit(&mut self) {
        if self.phase != Phase::Asking { return; }
        self.total += 1;
        let val: i64 = self.input.parse().unwrap_or(i64::MAX);
        let ok = val == self.answer;
        if ok {
            self.correct += 1;
            self.streak += 1;
            if self.streak > self.best { self.best = self.streak; }
        } else {
            self.streak = 0;
        }
        self.phase = Phase::Flashing(ok, 30);
    }

    fn timeout(&mut self) {
        if self.phase != Phase::Asking { return; }
        self.total += 1;
        self.streak = 0;
        self.phase = Phase::Flashing(false, 30);
    }

    fn tick(&mut self) {
        match &mut self.phase {
            Phase::Asking => {
                if self.timer > 0 {
                    self.timer -= 1;
                } else {
                    self.timeout();
                }
            }
            Phase::Flashing(_, rem) => {
                if *rem > 0 { *rem -= 1; }
                else { self.next_question(); }
            }
        }
    }
}

fn draw(q: &Quiz) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Math Quiz");
    let diff_color = [C_GREEN, C_SEL, C_ORANGE, C_RED][q.diff];
    text(160, 16, diff_color, DIFF_NAMES[q.diff]);
    text(280, 16, C_HINT, &format!("Streak: {}  Best: {}  {}/{}", q.streak, q.best, q.correct, q.total));
    text(580, 16, C_HINT, "Enter:submit  Tab:diff  N:reset");

    // Timer bar
    let bar_y = HEADER_H + 8;
    fill(40, bar_y, W - 80, 8, C_CARD);
    let bar_fill = (W - 80) * q.timer / TIMER_TICKS;
    let bar_color = if q.timer > TIMER_TICKS * 2 / 3 { C_GREEN }
                    else if q.timer > TIMER_TICKS / 3 { C_ORANGE }
                    else { C_RED };
    fill(40, bar_y, bar_fill, 8, bar_color);

    // Question display
    let q_y = HEADER_H + 80;
    let question = format!("{} {} {} = ?", q.a, q.op, q.b);
    text_l((W - question.len() as u32 * 18) / 2, q_y, C_TEXT, &question);

    // Input box
    let input_y = q_y + 80;
    let input_x = (W - 240) / 2;
    fill(input_x, input_y, 240, 56, C_CARD);
    border(input_x, input_y, 240, 56, C_BORDER);

    let flash_color = match &q.phase {
        Phase::Flashing(true, _) => C_GREEN,
        Phase::Flashing(false, _) => C_RED,
        Phase::Asking => C_TEXT,
    };

    let display_str = if let Phase::Flashing(correct, _) = &q.phase {
        if *correct { format!("{} ✓", q.answer) } else { format!("{} ✗", q.answer) }
    } else if q.input.is_empty() {
        "_".to_string()
    } else {
        q.input.clone()
    };
    text_l(input_x + 12, input_y + 12, flash_color, &display_str);

    // Difficulty selector
    let sel_y = input_y + 120;
    text((W - 320) / 2, sel_y - 24, C_HINT, "Tab to change difficulty:");
    for (i, name) in DIFF_NAMES.iter().enumerate() {
        let dx = (W - 320) / 2 + i as u32 * 82;
        let color = if i == q.diff { diff_color } else { C_HINT };
        fill(dx, sel_y, 76, 32, if i == q.diff { C_CARD } else { C_BG });
        border(dx, sel_y, 76, 32, if i == q.diff { diff_color } else { C_BORDER });
        text(dx + 8, sel_y + 8, color, name);
    }

    // Stats row
    let stats_y = sel_y + 80;
    let acc = if q.total > 0 { q.correct * 100 / q.total } else { 100 };
    text(40, stats_y, C_HINT, &format!("Accuracy: {}%  Questions answered: {}", acc, q.total));

    flush();
}

fn main() {
    let stdin = io::stdin();
    let seed: u64 = 0xFACEB00C12345678;
    let mut quiz = Quiz::new(seed, 0);

    println!("@supervisor: raise math-quiz");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&quiz);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") {
            quiz.tick();
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&quiz);
            continue;
        }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\t" => {
                quiz.diff = (quiz.diff + 1) % 4;
                quiz.next_question();
            }
            "n" | "N" => {
                let new_seed = lcg(quiz.seed);
                quiz = Quiz::new(new_seed, quiz.diff);
            }
            "\x7f" => { quiz.input.pop(); }
            "" => { quiz.submit(); }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b.is_ascii_digit() && quiz.input.len() < 8 {
                    if quiz.phase == Phase::Asking {
                        quiz.input.push(b as char);
                    }
                } else if b == b'-' && quiz.input.is_empty() && quiz.phase == Phase::Asking {
                    quiz.input.push('-');
                }
            }
            _ => {}
        }
        draw(&quiz);
    }
}
