use std::io::{self, BufRead, Write};

const W: i32 = 880;
const H: i32 = 640;

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

// 40 country → capital
static CAPITALS: &[(&str, &str)] = &[
    ("France",        "Paris"),
    ("Germany",       "Berlin"),
    ("Japan",         "Tokyo"),
    ("Brazil",        "Brasilia"),
    ("Australia",     "Canberra"),
    ("Canada",        "Ottawa"),
    ("India",         "New Delhi"),
    ("China",         "Beijing"),
    ("Russia",        "Moscow"),
    ("USA",           "Washington DC"),
    ("UK",            "London"),
    ("Italy",         "Rome"),
    ("Spain",         "Madrid"),
    ("Mexico",        "Mexico City"),
    ("Argentina",     "Buenos Aires"),
    ("South Africa",  "Pretoria"),
    ("Egypt",         "Cairo"),
    ("Nigeria",       "Abuja"),
    ("Kenya",         "Nairobi"),
    ("Ethiopia",      "Addis Ababa"),
    ("Turkey",        "Ankara"),
    ("Iran",          "Tehran"),
    ("Saudi Arabia",  "Riyadh"),
    ("Indonesia",     "Jakarta"),
    ("Pakistan",      "Islamabad"),
    ("Bangladesh",    "Dhaka"),
    ("Thailand",      "Bangkok"),
    ("Vietnam",       "Hanoi"),
    ("Philippines",   "Manila"),
    ("South Korea",   "Seoul"),
    ("Portugal",      "Lisbon"),
    ("Netherlands",   "Amsterdam"),
    ("Poland",        "Warsaw"),
    ("Ukraine",       "Kyiv"),
    ("Sweden",        "Stockholm"),
    ("Norway",        "Oslo"),
    ("Denmark",       "Copenhagen"),
    ("Greece",        "Athens"),
    ("Czech Republic","Prague"),
    ("Romania",       "Bucharest"),
];

// 20 continent → country
static CONT_COUNTRY: &[(&str, &str)] = &[
    ("Africa — largest country by area",     "Algeria"),
    ("Africa — most populous country",       "Nigeria"),
    ("Africa — southernmost country",        "South Africa"),
    ("Asia — most populous country",         "China"),
    ("Asia — largest country by area",       "Russia"),
    ("Asia — smallest country",              "Maldives"),
    ("Europe — largest country by area",     "Russia"),
    ("Europe — most populous country",       "Germany"),
    ("Europe — smallest country",            "Vatican City"),
    ("North America — largest country",      "Canada"),
    ("North America — most populous",        "USA"),
    ("South America — largest country",      "Brazil"),
    ("South America — highest country",      "Bolivia"),
    ("Oceania — largest country",            "Australia"),
    ("Oceania — most populous country",      "Australia"),
    ("Asia — island nation near Japan",      "Philippines"),
    ("Africa — easternmost country",         "Somalia"),
    ("Europe — peninsula country",           "Portugal"),
    ("Asia — country known as Land of Rising Sun", "Japan"),
    ("South America — only Portuguese-speaking",   "Brazil"),
];

struct Question {
    prompt: String,
    answer: String,
    qtype: u8, // 0=capital, 1=continent
}

fn build_questions() -> Vec<Question> {
    let mut qs = Vec::new();
    for &(country, cap) in CAPITALS {
        qs.push(Question {
            prompt: format!("Capital of {}?", country),
            answer: cap.to_string(),
            qtype: 0,
        });
    }
    for &(clue, country) in CONT_COUNTRY {
        qs.push(Question {
            prompt: clue.to_string(),
            answer: country.to_string(),
            qtype: 1,
        });
    }
    qs
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn shuffle(v: &mut Vec<usize>, seed: &mut u64) {
    let n = v.len();
    for i in (1..n).rev() {
        *seed = lcg(*seed);
        let j = (*seed >> 33) as usize % (i + 1);
        v.swap(i, j);
    }
}

fn build_options(q_idx: usize, questions: &[Question], seed: &mut u64, hint_used: bool) -> Vec<String> {
    let correct = questions[q_idx].answer.clone();
    let qtype = questions[q_idx].qtype;

    // Collect pool of wrong answers from same type
    let pool: Vec<String> = questions.iter().enumerate()
        .filter(|(i, qq)| *i != q_idx && qq.qtype == qtype && qq.answer != correct)
        .map(|(_, qq)| qq.answer.clone())
        .collect();

    let mut wrongs: Vec<String> = Vec::new();
    let mut used = vec![false; pool.len()];
    let needed = if hint_used { 1 } else { 3 };
    let mut attempts = 0;
    while wrongs.len() < needed && attempts < 200 {
        *seed = lcg(*seed);
        let idx = (*seed >> 33) as usize % pool.len().max(1);
        if !used[idx] {
            used[idx] = true;
            wrongs.push(pool[idx].clone());
        }
        attempts += 1;
    }
    while wrongs.len() < needed {
        wrongs.push("N/A".to_string());
    }

    let mut opts = wrongs;
    opts.push(correct);
    // shuffle opts
    let n = opts.len();
    for i in (1..n).rev() {
        *seed = lcg(*seed);
        let j = (*seed >> 33) as usize % (i + 1);
        opts.swap(i, j);
    }
    opts
}

#[derive(PartialEq)]
enum Screen { Quiz, End }

struct Mistake {
    prompt: String,
    correct: String,
    given: String,
}

struct App {
    questions: Vec<Question>,
    order: Vec<usize>,
    q_idx: usize,
    options: Vec<String>,
    correct_idx: usize,
    seed: u64,
    score: i32,
    correct: u32,
    wrong: u32,
    streak: u32,
    best_streak: u32,
    timer: u8,
    screen: Screen,
    feedback: Option<bool>, // true=correct false=wrong
    feedback_ticks: u8,
    hint_used: bool,
    mistakes: Vec<Mistake>,
}

impl App {
    fn new() -> Self {
        let questions = build_questions();
        let n = questions.len();
        let mut order: Vec<usize> = (0..n).collect();
        let mut s = 0x6E04_1234_5678_9ABCu64;
        shuffle(&mut order, &mut s);

        let opts = build_options(order[0], &questions, &mut s, false);
        let correct_idx = opts.iter().position(|o| *o == questions[order[0]].answer).unwrap_or(0);

        App {
            questions,
            order,
            q_idx: 0,
            options: opts,
            correct_idx,
            seed: s,
            score: 0,
            correct: 0,
            wrong: 0,
            streak: 0,
            best_streak: 0,
            timer: 15,
            screen: Screen::Quiz,
            feedback: None,
            feedback_ticks: 0,
            hint_used: false,
            mistakes: Vec::new(),
        }
    }

    fn restart(&mut self) {
        let n = self.questions.len();
        let mut order: Vec<usize> = (0..n).collect();
        shuffle(&mut order, &mut self.seed);
        self.order = order;
        self.q_idx = 0;
        self.score = 0;
        self.correct = 0;
        self.wrong = 0;
        self.streak = 0;
        self.best_streak = 0;
        self.timer = 15;
        self.screen = Screen::Quiz;
        self.feedback = None;
        self.feedback_ticks = 0;
        self.hint_used = false;
        self.mistakes.clear();
        self.load_question();
    }

    fn load_question(&mut self) {
        self.hint_used = false;
        self.options = build_options(self.order[self.q_idx], &self.questions, &mut self.seed, false);
        self.correct_idx = self.options.iter()
            .position(|o| *o == self.questions[self.order[self.q_idx]].answer)
            .unwrap_or(0);
        self.timer = 15;
    }

    fn apply_hint(&mut self) {
        if self.hint_used { return; }
        self.hint_used = true;
        self.score -= 5;
        // Rebuild with only 2 options (1 wrong + correct)
        self.options = build_options(self.order[self.q_idx], &self.questions, &mut self.seed, true);
        self.correct_idx = self.options.iter()
            .position(|o| *o == self.questions[self.order[self.q_idx]].answer)
            .unwrap_or(0);
    }

    fn answer(&mut self, opt_idx: usize) {
        if self.feedback.is_some() { return; }
        let q = &self.questions[self.order[self.q_idx]];
        let is_correct = opt_idx == self.correct_idx;
        self.feedback = Some(is_correct);
        self.feedback_ticks = 3;
        if is_correct {
            self.score += 10;
            self.correct += 1;
            self.streak += 1;
            if self.streak > self.best_streak { self.best_streak = self.streak; }
        } else {
            self.wrong += 1;
            self.streak = 0;
            self.mistakes.push(Mistake {
                prompt: q.prompt.clone(),
                correct: q.answer.clone(),
                given: self.options.get(opt_idx).cloned().unwrap_or_default(),
            });
        }
        self.draw();
    }

    fn advance(&mut self) {
        self.q_idx += 1;
        if self.q_idx >= self.order.len() {
            self.screen = Screen::End;
        } else {
            self.feedback = None;
            self.load_question();
        }
        self.draw();
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Geo Quiz");

        match self.screen {
            Screen::Quiz => self.draw_quiz(),
            Screen::End  => self.draw_end(),
        }

        flush();
    }

    fn draw_quiz(&self) {
        let qi = self.q_idx;
        let total = self.order.len();

        // Progress bar
        let pb_w = W - 200;
        fill(12, 36, pb_w, 6, C_CARD);
        let filled = if total > 0 { (qi as i32 * pb_w) / total as i32 } else { 0 };
        if filled > 0 { fill(12, 36, filled, 6, C_SEL); }
        text(pb_w + 16, 30, C_HINT, &format!("{}/{}", qi + 1, total));

        // Score panel (right side)
        fill(W - 188, 46, 176, 130, C_CARD);
        border(W - 188, 46, 176, 130, C_BORDER);
        text(W - 180, 54, C_HINT, "Score");
        let sc_color = if self.score < 0 { C_RED } else { C_GREEN };
        text(W - 180, 70, sc_color, &format!("{}", self.score));
        text(W - 180, 90, C_GREEN, &format!("Correct: {}", self.correct));
        text(W - 180, 106, C_RED,  &format!("Wrong:   {}", self.wrong));
        let pct = if self.correct + self.wrong > 0 {
            self.correct * 100 / (self.correct + self.wrong)
        } else { 0 };
        text(W - 180, 122, C_TEXT, &format!("{}%", pct));
        text(W - 180, 142, C_ORANGE, &format!("Streak: {} / {}", self.streak, self.best_streak));
        text(W - 180, 158, C_HINT, "(cur / best)");

        // Timer bar
        let tb_w = W - 200;
        fill(12, 50, tb_w, 10, C_CARD);
        let tc = if self.timer > 8 { C_GREEN } else if self.timer > 4 { C_YELLOW } else { C_RED };
        let tf = (self.timer as i32 * tb_w) / 15;
        if tf > 0 { fill(12, 50, tf, 10, tc); }
        border(12, 50, tb_w, 10, C_BORDER);
        text(12, 62, C_HINT, &format!("{}s", self.timer));

        // Question
        let q = &self.questions[self.order[self.q_idx]];
        fill(12, 82, W - 200, 70, C_CARD);
        border(12, 82, W - 200, 70, C_BORDER);
        let qtype_label = if q.qtype == 0 { "CAPITAL" } else { "GEOGRAPHY" };
        let qt_color = if q.qtype == 0 { C_SEL } else { C_PURPLE };
        text(20, 90, qt_color, qtype_label);
        // Word-wrap prompt manually (max ~85 chars per line at 8px)
        let prompt = &q.prompt;
        if prompt.len() <= 78 {
            text(20, 108, C_TEXT, prompt);
        } else {
            let mid = prompt[..78].rfind(' ').unwrap_or(78);
            text(20, 104, C_TEXT, &prompt[..mid]);
            text(20, 120, C_TEXT, &prompt[mid + 1..]);
        }

        // Hint button
        let hint_color = if self.hint_used { C_HINT } else { C_YELLOW };
        text(20, 146, hint_color, "H=hint (-5pts)");

        // Options
        let n_opts = self.options.len();
        let labels = ['A', 'B', 'C', 'D'];
        for (i, opt) in self.options.iter().enumerate() {
            let oy = 170 + i as i32 * 70;
            let is_correct_opt = i == self.correct_idx;

            let bg = match self.feedback {
                Some(_) if is_correct_opt => 0x1C3A1CFF,
                _ => C_CARD,
            };
            let bc = match self.feedback {
                Some(_) if is_correct_opt => C_GREEN,
                _ => C_BORDER,
            };

            fill(12, oy, W - 200, 60, bg);
            border(12, oy, W - 200, 60, bc);
            let lc = if i < labels.len() { C_ORANGE } else { C_HINT };
            let label = if i < labels.len() { format!("{}.", labels[i]) } else { "?".to_string() };
            text(20, oy + 20, lc, &label);
            // word-wrap option text
            if opt.len() <= 60 {
                text(44, oy + 20, C_TEXT, opt);
            } else {
                let mid = opt[..60].rfind(' ').unwrap_or(60);
                text(44, oy + 14, C_TEXT, &opt[..mid]);
                text(44, oy + 30, C_TEXT, &opt[mid + 1..]);
            }
        }

        // Feedback overlay
        if let Some(correct) = self.feedback {
            let msg = if correct { "Correct!" } else { "Wrong!" };
            let mc = if correct { C_GREEN } else { C_RED };
            let fx = W / 2 - 80;
            fill(fx, 420, 160, 40, C_HEADER);
            border(fx, 420, 160, 40, mc);
            let mx = fx + (160 - msg.len() as i32 * 8) / 2;
            text(mx, 432, mc, msg);
            if !correct {
                let ans = &self.questions[self.order[self.q_idx]].answer;
                text(fx, 464, C_HINT, &format!("Answer: {}", ans));
            }
        }

        // Bottom hints
        let bh = H - 24;
        text(12, bh, C_HINT, "A/B/C/D = answer   H = hint   Q = quit");

        let _ = n_opts;
    }

    fn draw_end(&self) {
        let total = self.correct + self.wrong;
        let pct = if total > 0 { self.correct * 100 / total } else { 0 };

        fill(W / 2 - 240, 60, 480, 280, C_CARD);
        border(W / 2 - 240, 60, 480, 280, C_SEL);
        text(W / 2 - 60, 80, C_SEL, "Quiz Complete!");
        text(W / 2 - 160, 110, C_TEXT, &format!("Score: {}   ({}/{}  {}%)", self.score, self.correct, total, pct));
        text(W / 2 - 160, 130, C_ORANGE, &format!("Best streak: {}", self.best_streak));

        // Top-5 mistakes
        text(W / 2 - 160, 160, C_YELLOW, "Top mistakes:");
        let show = self.mistakes.len().min(5);
        for (i, m) in self.mistakes.iter().take(show).enumerate() {
            let my = 178 + i as i32 * 22;
            let prompt_short: String = m.prompt.chars().take(40).collect();
            text(W / 2 - 160, my, C_HINT, &format!("{}: {}", prompt_short, m.correct));
        }
        if self.mistakes.is_empty() {
            text(W / 2 - 80, 180, C_GREEN, "No mistakes! Perfect!");
        }

        text(W / 2 - 60, 360, C_GREEN, "R = play again   Q = quit");
    }

    fn tick(&mut self) {
        if self.screen == Screen::End { return; }
        if self.feedback.is_some() {
            self.feedback_ticks = self.feedback_ticks.saturating_sub(1);
            if self.feedback_ticks == 0 {
                self.advance();
                return;
            }
            self.draw();
            return;
        }
        if self.timer > 0 {
            self.timer -= 1;
        }
        if self.timer == 0 {
            // auto-advance: count as wrong
            self.wrong += 1;
            self.streak = 0;
            let q = &self.questions[self.order[self.q_idx]];
            self.mistakes.push(Mistake {
                prompt: q.prompt.clone(),
                correct: q.answer.clone(),
                given: "(timeout)".to_string(),
            });
            self.feedback = Some(false);
            self.feedback_ticks = 3;
        }
        self.draw();
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" {
            self.tick();
            println!("@supervisor: ping");
            return;
        }
        match self.screen {
            Screen::Quiz => {
                if self.feedback.is_some() { return; }
                match line {
                    "a" | "A" => self.answer(0),
                    "b" | "B" => self.answer(1),
                    "c" | "C" => self.answer(2),
                    "d" | "D" => { if self.options.len() > 3 { self.answer(3); } }
                    "h" | "H" => { self.apply_hint(); self.draw(); }
                    "q" | "Q" | "\x03" => std::process::exit(0),
                    _ => {}
                }
            }
            Screen::End => {
                match line {
                    "r" | "R" => self.restart(),
                    "q" | "Q" | "\x03" => std::process::exit(0),
                    _ => {}
                }
                self.draw();
            }
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
