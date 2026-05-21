use std::io::{self, BufRead, Write};

const W: u32 = 920;
const H: u32 = 680;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_CARD: u32    = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_RED: u32     = 0xFF7B72FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;

struct Q {
    cat:     &'static str,
    q:       &'static str,
    choices: [&'static str; 4],
    ans:     usize,
}

const QUESTIONS: &[Q] = &[
    Q { cat: "Science",   q: "What is the chemical symbol for gold?",                            choices: ["Au", "Ag", "Fe", "Cu"],               ans: 0 },
    Q { cat: "Science",   q: "How many bones are in the adult human body?",                      choices: ["196", "206", "216", "226"],            ans: 1 },
    Q { cat: "Science",   q: "What planet is known as the Red Planet?",                          choices: ["Venus", "Jupiter", "Mars", "Saturn"],  ans: 2 },
    Q { cat: "Science",   q: "What is the speed of light (approx) in km/s?",                    choices: ["200,000", "300,000", "400,000", "150,000"], ans: 1 },
    Q { cat: "Science",   q: "What element has atomic number 1?",                                choices: ["Helium", "Oxygen", "Hydrogen", "Carbon"], ans: 2 },
    Q { cat: "Science",   q: "How many chromosomes do humans normally have?",                    choices: ["44", "46", "48", "42"],               ans: 1 },
    Q { cat: "History",   q: "In what year did World War II end?",                               choices: ["1943", "1944", "1945", "1946"],        ans: 2 },
    Q { cat: "History",   q: "Who was the first President of the United States?",                choices: ["John Adams", "Thomas Jefferson", "Benjamin Franklin", "George Washington"], ans: 3 },
    Q { cat: "History",   q: "The Great Wall of China was built primarily to defend against?",   choices: ["Japan", "Mongolia", "Russia", "Korea"], ans: 1 },
    Q { cat: "History",   q: "In what year did the Berlin Wall fall?",                           choices: ["1987", "1988", "1989", "1990"],        ans: 2 },
    Q { cat: "History",   q: "Which empire was ruled by Julius Caesar?",                         choices: ["Greek", "Roman", "Ottoman", "Persian"], ans: 1 },
    Q { cat: "History",   q: "Who wrote the Declaration of Independence?",                       choices: ["George Washington", "John Adams", "Thomas Jefferson", "James Madison"], ans: 2 },
    Q { cat: "Geography", q: "What is the capital of Australia?",                                choices: ["Sydney", "Melbourne", "Brisbane", "Canberra"], ans: 3 },
    Q { cat: "Geography", q: "Which is the longest river in the world?",                         choices: ["Amazon", "Yangtze", "Nile", "Mississippi"], ans: 2 },
    Q { cat: "Geography", q: "How many countries are in Africa?",                                choices: ["44", "54", "64", "74"],               ans: 1 },
    Q { cat: "Geography", q: "What is the smallest country in the world?",                       choices: ["Monaco", "San Marino", "Liechtenstein", "Vatican City"], ans: 3 },
    Q { cat: "Geography", q: "Which ocean is the largest?",                                      choices: ["Atlantic", "Indian", "Arctic", "Pacific"], ans: 3 },
    Q { cat: "Geography", q: "Mount Everest is located in which mountain range?",                choices: ["Alps", "Andes", "Himalayas", "Rockies"], ans: 2 },
    Q { cat: "Tech",      q: "What does HTTP stand for?",                                        choices: ["HyperText Transfer Protocol", "High Transfer Text Protocol", "HyperText Technical Protocol", "Hybrid Text Transfer Protocol"], ans: 0 },
    Q { cat: "Tech",      q: "Who founded Apple Inc.?",                                          choices: ["Bill Gates", "Steve Jobs", "Jeff Bezos", "Linus Torvalds"], ans: 1 },
    Q { cat: "Tech",      q: "What does CPU stand for?",                                         choices: ["Central Processing Unit", "Computer Personal Unit", "Core Processing Utility", "Central Program Unit"], ans: 0 },
    Q { cat: "Tech",      q: "In what year was the first iPhone released?",                      choices: ["2005", "2006", "2007", "2008"],        ans: 2 },
    Q { cat: "Tech",      q: "What programming language was created by Guido van Rossum?",       choices: ["Java", "Ruby", "Python", "Perl"],      ans: 2 },
    Q { cat: "Tech",      q: "What does RAM stand for?",                                         choices: ["Random Access Memory", "Read Access Module", "Rapid Action Memory", "Remote Access Mode"], ans: 0 },
    Q { cat: "General",   q: "How many sides does a hexagon have?",                              choices: ["5", "6", "7", "8"],                   ans: 1 },
    Q { cat: "General",   q: "What is the largest planet in our solar system?",                  choices: ["Saturn", "Uranus", "Neptune", "Jupiter"], ans: 3 },
    Q { cat: "General",   q: "How many strings does a standard guitar have?",                    choices: ["4", "5", "6", "7"],                   ans: 2 },
    Q { cat: "General",   q: "What is the currency of Japan?",                                   choices: ["Yuan", "Won", "Yen", "Baht"],         ans: 2 },
    Q { cat: "General",   q: "What is the square root of 144?",                                  choices: ["11", "12", "13", "14"],               ans: 1 },
    Q { cat: "General",   q: "How many days are in a leap year?",                                choices: ["364", "365", "366", "367"],            ans: 2 },
];

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

struct Game {
    order:    [usize; 30],
    qi:       usize,          // current question index into order
    selected: Option<usize>,  // A=0 B=1 C=2 D=3 selected
    confirmed: bool,
    correct:  u32,
    seed:     u64,
    done:     bool,
}

impl Game {
    fn new(seed: u64) -> Self {
        let mut g = Game {
            order: [0; 30],
            qi: 0,
            selected: None,
            confirmed: false,
            correct: 0,
            seed,
            done: false,
        };
        for i in 0..30 { g.order[i] = i; }
        g.shuffle();
        g
    }

    fn shuffle(&mut self) {
        for i in (1..30).rev() {
            self.seed = lcg(self.seed);
            let j = (self.seed >> 33) as usize % (i + 1);
            self.order.swap(i, j);
        }
    }

    fn q(&self) -> &'static Q { &QUESTIONS[self.order[self.qi]] }

    fn select(&mut self, idx: usize) {
        if self.confirmed { return; }
        self.selected = Some(idx);
    }

    fn confirm(&mut self) {
        if self.confirmed || self.selected.is_none() { return; }
        self.confirmed = true;
        if self.selected == Some(self.q().ans) { self.correct += 1; }
    }

    fn next(&mut self) {
        if !self.confirmed { return; }
        if self.qi + 1 >= 30 { self.done = true; return; }
        self.qi += 1;
        self.selected = None;
        self.confirmed = false;
    }
}

const CHOICE_LABELS: [&str; 4] = ["A", "B", "C", "D"];
const CHOICE_COLORS: [u32; 4]  = [0x58A6FFFF, 0xFFA657FF, 0x3FB950FF, 0xFF7B72FF];

fn draw_wrapped(x: u32, y: u32, max_w: u32, rgba: u32, s: &str) -> u32 {
    let cpl = (max_w / 9).max(1) as usize;
    let mut ly = y;
    let words: Vec<&str> = s.split_whitespace().collect();
    let mut line = String::new();
    for word in &words {
        if line.len() + word.len() + 1 > cpl && !line.is_empty() {
            text(x, ly, rgba, &line);
            ly += 20;
            line.clear();
        }
        if !line.is_empty() { line.push(' '); }
        line.push_str(word);
    }
    if !line.is_empty() { text(x, ly, rgba, &line); ly += 20; }
    ly
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Quiz");

    if g.done {
        // Final score screen
        let pct = g.correct * 100 / 30;
        let col = if pct >= 70 { C_GREEN } else if pct >= 40 { C_ORANGE } else { C_RED };
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, HEADER_H, C_HEADER);
        text(16, 16, C_ORANGE, "Quiz — Complete!");
        text(400, 16, C_HINT, "R:restart");
        let bx = (W - 400) / 2;
        let by = (H - 200) / 2;
        fill(bx, by, 400, 200, C_CARD);
        border(bx, by, 400, 200, col);
        text(bx + 100, by + 30, C_HINT, "Final Score");
        text(bx + 80, by + 70, col, &format!("{} / 30 correct", g.correct));
        text(bx + 100, by + 100, col, &format!("{}%", pct));
        let verdict = if pct >= 90 { "Excellent!" } else if pct >= 70 { "Well done!" } else if pct >= 40 { "Keep practicing" } else { "Better luck next time" };
        text(bx + 60, by + 140, C_HINT, verdict);
        flush();
        return;
    }

    let q = g.q();
    text(160, 16, C_HINT, &format!("Q {}/30  {}  Score: {}", g.qi + 1, q.cat, g.correct));
    if g.confirmed {
        text(600, 16, C_HINT, "Space/Enter:next");
    } else {
        text(600, 16, C_HINT, "A/B/C/D:select  Enter:confirm");
    }

    // Progress bar
    fill(16, HEADER_H + 4, (W - 32) * g.qi as u32 / 30, 4, C_SEL);
    fill(16, HEADER_H + 4, W - 32, 4, 0x0u32);
    border(16, HEADER_H + 4, W - 32, 4, C_BORDER);
    fill(16, HEADER_H + 4, (W - 32) * g.qi as u32 / 30, 4, C_SEL);

    // Question box
    let qx = 24u32;
    let qy = HEADER_H + 20;
    let qw = W - 48;
    fill(qx, qy, qw, 120, C_CARD);
    border(qx, qy, qw, 120, C_BORDER);
    draw_wrapped(qx + 16, qy + 16, qw - 32, C_TEXT, q.q);

    // Choices
    let cy0 = qy + 136;
    let ch_h = 72u32;
    let ch_gap = 12u32;
    let ch_w = (W - 48 - ch_gap) / 2;

    for (i, &choice) in q.choices.iter().enumerate() {
        let row = i / 2;
        let col_i = i % 2;
        let cx = 24 + col_i as u32 * (ch_w + ch_gap);
        let cy = cy0 + row as u32 * (ch_h + ch_gap);

        let is_sel = g.selected == Some(i);
        let is_correct = i == q.ans;

        let bg = if g.confirmed {
            if is_correct { 0x0D2E0DFF }
            else if is_sel { 0x2E0D0DFF }
            else { C_CARD }
        } else if is_sel {
            C_SEL_BG
        } else {
            C_CARD
        };

        let bc = if g.confirmed {
            if is_correct { C_GREEN }
            else if is_sel { C_RED }
            else { C_BORDER }
        } else if is_sel {
            C_SEL
        } else {
            C_BORDER
        };

        fill(cx, cy, ch_w, ch_h, bg);
        border(cx, cy, ch_w, ch_h, bc);

        // Label badge
        fill(cx + 12, cy + 20, 28, 28, CHOICE_COLORS[i]);
        text(cx + 18, cy + 24, C_BG, CHOICE_LABELS[i]);

        // Choice text
        draw_wrapped(cx + 48, cy + 12, ch_w - 56, if g.confirmed && is_correct { C_GREEN } else { C_TEXT }, choice);
    }

    // Result message
    if g.confirmed {
        let is_right = g.selected == Some(q.ans);
        let msg = if is_right { "Correct!" } else { "Wrong!" };
        let mc = if is_right { C_GREEN } else { C_RED };
        let my = cy0 + 2 * (ch_h + ch_gap) + 16;
        text((W - 80) / 2, my, mc, msg);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut seed: u64 = 0xDEADBEEF12345678;
    let mut game = Game::new(seed);

    println!("@supervisor: raise quiz");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "a" | "A" => { game.select(0); }
            "b" | "B" => { game.select(1); }
            "c" | "C" => { game.select(2); }
            "d" | "D" => { game.select(3); }
            "\r" | "" => {
                if !game.confirmed { game.confirm(); }
                else { game.next(); }
            }
            " " => { game.next(); }
            "r" | "R" => { seed = lcg(seed); game = Game::new(seed); }
            _ => {}
        }
        draw(&game);
    }
}
