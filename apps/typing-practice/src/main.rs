use std::io::{self, BufRead, Write};

const W: i32 = 1040;
const H: i32 = 640;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_CARD: u32   = 0x161B22FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_YELLOW: u32 = 0xD29922FF;

static PASSAGES: [&str; 20] = [
    "The quick brown fox jumps over the lazy dog",
    "Pack my box with five dozen liquor jugs",
    "How vexingly quick daft zebras jump",
    "The five boxing wizards jump quickly",
    "Sphinx of black quartz judge my vow",
    "Jackdaws love my big sphinx of quartz",
    "The job requires extra pluck and zeal from every young wage earner",
    "A mad boxer shot a quick gloved jab to the jaw of his dizzy opponent",
    "Sixty zippers were quickly picked from the woven jute bag",
    "We promptly judged antique ivory buckles for the next prize",
    "How razorback jumping frogs can level six piqued gymnasts",
    "Crazy Fredrick bought many very exquisite opal jewels",
    "Quick zephyrs blow vexing daft Jim",
    "Two driven jocks help fax my big quiz",
    "Five quacking zephyrs jolt my wax bed",
    "The quick onyx goblin jumps over the lazy dwarf",
    "Blowzy red vixens fight for a quick jump",
    "Jumpy halfback vows to scold beefy pawn quiz",
    "Flummoxed by job task quaver wren",
    "Pack my red box with five dozen quality jugs",
];

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

enum State { Typing, Finished }

struct App {
    passage_idx:   usize,
    typed:         Vec<char>,
    elapsed_ticks: u64,
    state:         State,
    bests:         Vec<Option<(usize, usize)>>,
    started:       bool,
}

impl App {
    fn new() -> Self {
        App {
            passage_idx:   0,
            typed:         Vec::new(),
            elapsed_ticks: 0,
            state:         State::Typing,
            bests:         vec![None; PASSAGES.len()],
            started:       false,
        }
    }

    fn passage(&self) -> &str { PASSAGES[self.passage_idx] }

    fn passage_chars(&self) -> Vec<char> { self.passage().chars().collect() }

    fn correct_chars(&self) -> usize {
        let p = self.passage_chars();
        self.typed.iter().enumerate().filter(|&(i, &c)| i < p.len() && c == p[i]).count()
    }

    fn accuracy(&self) -> usize {
        if self.typed.is_empty() { return 100; }
        self.correct_chars() * 100 / self.typed.len()
    }

    fn wpm(&self) -> usize {
        let secs = self.elapsed_ticks.max(1) as usize;
        (self.correct_chars() * 60) / (5 * secs)
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Typing Practice");
        text(W - 440, 8, C_HINT, "Tab=next  Ctrl+R=restart  Q=quit");

        let p_chars = self.passage_chars();
        let total = p_chars.len();
        let cursor_pos = self.typed.len().min(total);

        // Passage card
        let py = 44i32;
        fill(8, py, W - 16, 64, C_CARD);
        border(8, py, W - 16, 64, C_BORDER);

        let ch_w = 8i32;
        let text_w = total as i32 * ch_w;
        let tx = ((W - text_w) / 2).max(16);
        let ty = py + 24;

        // Cursor highlight
        if cursor_pos < total {
            fill(tx + cursor_pos as i32 * ch_w, ty - 2, ch_w, 20, 0x1F3A5FFF);
        }

        // Draw chars grouped by color
        let mut i = 0usize;
        while i < total {
            let color_of = |idx: usize| {
                if idx < self.typed.len() {
                    if self.typed[idx] == p_chars[idx] { C_GREEN } else { C_RED }
                } else { C_HINT }
            };
            let c = color_of(i);
            let mut j = i + 1;
            while j < total && color_of(j) == c { j += 1; }
            let s: String = p_chars[i..j].iter().collect();
            text(tx + i as i32 * ch_w, ty, c, &s);
            i = j;
        }

        // Progress bar at bottom of card
        let bar_w = (cursor_pos as i32 * (W - 16)) / total.max(1) as i32;
        fill(8, py + 56, W - 16, 4, C_CARD);
        if bar_w > 0 { fill(8, py + 56, bar_w, 4, C_SEL); }

        // Stats row
        let sy = 120i32;
        fill(8, sy, W - 16, 76, C_CARD);
        border(8, sy, W - 16, 76, C_BORDER);

        let wpm = self.wpm();
        let acc = self.accuracy();
        let secs = self.elapsed_ticks;

        let stat_boxes: &[(i32, &str, u32, String)] = &[
            (20,  "WPM",
             if wpm > 80 { C_GREEN } else if wpm > 50 { C_ORANGE } else { C_TEXT },
             format!("{}", wpm)),
            (280, "ACCURACY",
             if acc > 95 { C_GREEN } else if acc > 80 { C_ORANGE } else { C_RED },
             format!("{}%", acc)),
            (540, "TIME",   C_TEXT,  format!("{}s", secs)),
            (800, "CHARS",  C_SEL,   format!("{}/{}", cursor_pos, total)),
        ];
        for &(bx, label, vc, ref val) in stat_boxes {
            fill(bx, sy + 8, 220, 60, C_HEADER);
            text(bx + 10, sy + 14, C_HINT, label);
            text(bx + 10, sy + 38, vc, val);
        }

        // PB row
        let pb_y = 208i32;
        fill(8, pb_y, W - 16, 48, C_CARD);
        border(8, pb_y, W - 16, 48, C_BORDER);
        text(16, pb_y + 16, C_HINT, "PERSONAL BEST:");
        match self.bests[self.passage_idx] {
            Some((pb_wpm, pb_acc)) => {
                let c = if wpm > 0 && wpm >= pb_wpm { C_YELLOW } else { C_SEL };
                text(168, pb_y + 16, c, &format!("{} WPM  {}% acc", pb_wpm, pb_acc));
            }
            None => { text(168, pb_y + 16, C_HINT, "No record yet"); }
        }
        let status = if self.started { "Typing..." } else { "Start typing to begin" };
        text(W - 220, pb_y + 16, C_HINT, status);

        // Passage list
        let list_y = 268i32;
        let list_h = H - list_y - 28;
        fill(8, list_y, W - 16, list_h, C_CARD);
        border(8, list_y, W - 16, list_h, C_BORDER);
        text(16, list_y + 8, C_HINT, &format!("PASSAGES ({}/{})", self.passage_idx + 1, PASSAGES.len()));

        for di in 0..((list_h - 28) / 22) as usize {
            let pi = (self.passage_idx + di) % PASSAGES.len();
            let ly = list_y + 26 + di as i32 * 22;
            let nc = if di == 0 { C_SEL } else { C_HINT };
            let prefix = if di == 0 { ">" } else { " " };
            let p = PASSAGES[pi];
            let max_c = (W as usize - 240) / 8;
            let display = if p.len() > max_c { &p[..max_c] } else { p };
            text(16, ly, nc, &format!("{} {:2}. {}", prefix, pi + 1, display));
            if let Some((bwpm, bacc)) = self.bests[pi] {
                text(W - 220, ly, C_GREEN, &format!("PB: {} WPM {}%", bwpm, bacc));
            }
        }

        // Finished overlay
        if matches!(self.state, State::Finished) {
            let ow = 400i32;
            let oh = 160i32;
            let ox = (W - ow) / 2;
            let oy = (H - oh) / 2;
            fill(ox, oy, ow, oh, C_HEADER);
            border(ox, oy, ow, oh, C_BORDER);
            text(ox + ow/2 - 48, oy + 20, C_GREEN, "COMPLETED!");
            let wpm_c = if wpm > 80 { C_GREEN } else if wpm > 50 { C_ORANGE } else { C_TEXT };
            text(ox + 20, oy + 50, wpm_c, &format!("{} WPM  {}% accuracy  {}s", wpm, acc, secs));
            let pb_msg = match self.bests[self.passage_idx] {
                Some((pb_wpm, _)) if wpm > pb_wpm => "New Personal Best!",
                _ => "Tab=next  Ctrl+R=retry",
            };
            text(ox + 20, oy + 80, C_YELLOW, pb_msg);
            text(ox + 20, oy + 110, C_HINT, "Tab=next  Ctrl+R=retry  Q=quit");
        }

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("Passage {}/{}  |  {} WPM  {}% acc  {}s  |  {}",
                 self.passage_idx + 1, PASSAGES.len(), wpm, acc, secs,
                 if self.started { "typing" } else { "ready" }));

        flush();
    }

    fn finish(&mut self) {
        self.state = State::Finished;
        let wpm = self.wpm();
        let acc = self.accuracy();
        let better = match self.bests[self.passage_idx] {
            None => true,
            Some((pb, _)) => wpm > pb,
        };
        if better { self.bests[self.passage_idx] = Some((wpm, acc)); }
        self.draw();
    }

    fn restart(&mut self) {
        self.typed.clear();
        self.elapsed_ticks = 0;
        self.started = false;
        self.state = State::Typing;
        self.draw();
    }

    fn next_passage(&mut self) {
        self.passage_idx = (self.passage_idx + 1) % PASSAGES.len();
        self.restart();
    }

    fn tick(&mut self) {
        if self.started && matches!(self.state, State::Typing) {
            self.elapsed_ticks += 1;
            self.draw();
        }
    }

    fn handle(&mut self, line: &str) {
        if line == "REPLY:pong" { self.tick(); return; }
        match line {
            "\t"              => { self.next_passage(); }
            "\x12"            => { self.restart(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ if matches!(self.state, State::Finished) => {}
            "\x7f" => {
                if !self.typed.is_empty() { self.typed.pop(); self.draw(); }
            }
            s if s.len() == 1 && !s.starts_with('\x1b') => {
                let p_chars = self.passage_chars();
                if self.typed.len() < p_chars.len() {
                    if !self.started { self.started = true; }
                    self.typed.push(s.chars().next().unwrap());
                    if self.typed.len() == p_chars.len() { self.finish(); } else { self.draw(); }
                }
            }
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
        if line == "REPLY:pong" {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }
    }
}
