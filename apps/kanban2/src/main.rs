use std::io::{self, BufRead, Write};

const W: i32 = 1100;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
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

// Priority: 0=low(green), 1=medium(orange), 2=high(red)
fn priority_color(p: u8) -> u32 {
    match p { 2 => C_RED, 1 => C_ORANGE, _ => C_GREEN }
}
fn priority_label(p: u8) -> &'static str {
    match p { 2 => "HIGH", 1 => "MED", _ => "LOW" }
}

const COL_NAMES: &[&str] = &["Backlog", "Todo", "Doing", "Done"];

#[derive(Clone)]
struct Card {
    title:    String,
    priority: u8,
}

const SEED_CARDS: &[(&str, u8, usize)] = &[
    ("Design system architecture",      2, 0),
    ("Write project proposal",          1, 0),
    ("Research competitor products",    0, 0),
    ("Set up CI/CD pipeline",           2, 0),
    ("Define API contracts",            1, 0),
    ("Create wireframes",               1, 1),
    ("Set up dev environment",          2, 1),
    ("Write unit tests",                1, 1),
    ("Implement auth flow",             2, 2),
    ("Build dashboard UI",             1, 2),
    ("Integrate payment gateway",       2, 2),
    ("Add dark mode support",           0, 2),
    ("Performance optimisation",        1, 2),
    ("Deploy to staging",               2, 3),
    ("Fix login bug",                   2, 3),
    ("Update documentation",            0, 3),
    ("User acceptance testing",         1, 3),
    ("Security audit",                  2, 3),
    ("Mobile responsive fixes",         1, 0),
    ("Add analytics tracking",          0, 1),
];

struct App {
    cols:     [Vec<Card>; 4],
    col:      usize,
    row:      usize,
    new_mode: bool,
    new_buf:  String,
    new_pri:  u8,
}

impl App {
    fn new() -> Self {
        let mut cols: [Vec<Card>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
        for &(title, pri, col) in SEED_CARDS {
            cols[col].push(Card { title: title.to_string(), priority: pri });
        }
        App { cols, col: 0, row: 0, new_mode: false, new_buf: String::new(), new_pri: 1 }
    }

    fn clamp_row(&mut self) {
        let len = self.cols[self.col].len();
        if len == 0 { self.row = 0; } else if self.row >= len { self.row = len - 1; }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Kanban Board v2");
        text(200, 8, C_HINT, "Tab/Shift-Tab:col  Arrows:card  Enter:promote  D:demote  N:new  Q:quit");

        let col_w = (W - 20) / 4;
        let col_x = [10, 10 + col_w, 10 + col_w * 2, 10 + col_w * 3];
        let col_y = 52i32;
        let col_h = H - col_y - 28;

        for ci in 0..4usize {
            let cx = col_x[ci];
            let sel_col = ci == self.col;
            let bg = if sel_col { 0x12181EFF } else { C_CARD };
            fill(cx, col_y, col_w - 4, col_h, bg);
            let bc = if sel_col { C_SEL } else { C_BORDER };
            border(cx, col_y, col_w - 4, col_h, bc);

            // Column header
            let hc = [C_HINT, C_ORANGE, C_SEL, C_GREEN][ci];
            text(cx + 8, col_y + 8, hc, COL_NAMES[ci]);
            let cnt = self.cols[ci].len();
            text(cx + col_w - 36, col_y + 8, C_HINT, &format!("({})", cnt));
            fill(cx + 4, col_y + 24, col_w - 12, 1, C_BORDER);

            // Cards
            for (ri, card) in self.cols[ci].iter().enumerate() {
                let cy = col_y + 30 + ri as i32 * 52;
                if cy + 48 > col_y + col_h { break; }
                let sel = sel_col && ri == self.row;
                let card_bg = if sel { 0x1C2433FF } else { 0x0D1117FF };
                fill(cx + 4, cy, col_w - 12, 48, card_bg);
                let cbc = if sel { C_SEL } else { C_BORDER };
                border(cx + 4, cy, col_w - 12, 48, cbc);

                // Priority indicator strip
                let pc = priority_color(card.priority);
                fill(cx + 4, cy, 3, 48, pc);

                // Card title (truncate to fit)
                let max_chars = ((col_w - 28) / 8) as usize;
                let title: String = card.title.chars().take(max_chars).collect();
                let tc = if sel { C_TEXT } else { C_HINT };
                text(cx + 12, cy + 8, tc, &title);

                // Priority badge
                let pl = priority_label(card.priority);
                text(cx + 12, cy + 28, pc, pl);
            }

            // New card input box (in active column only)
            if self.new_mode && ci == self.col {
                let ny = col_y + 30 + cnt as i32 * 52;
                if ny + 48 <= col_y + col_h {
                    fill(cx + 4, ny, col_w - 12, 48, 0x1A2030FF);
                    border(cx + 4, ny, col_w - 12, 48, C_SEL);
                    fill(cx + 4, ny, 3, 48, priority_color(self.new_pri));
                    let cursor_str = format!("{}|", self.new_buf);
                    let max_chars = ((col_w - 28) / 8) as usize;
                    let disp: String = cursor_str.chars().take(max_chars).collect();
                    text(cx + 12, ny + 8, C_TEXT, &disp);
                    text(cx + 12, ny + 28, C_HINT, &format!("Pri:{} (1-3)", self.new_pri + 1));
                }
            }
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        if self.new_mode {
            text(12, H - 18, C_ORANGE, &format!("New card: \"{}\"  Enter=save  Esc=cancel  1-3=priority", self.new_buf));
        } else {
            let col_name = COL_NAMES[self.col];
            let cnt = self.cols[self.col].len();
            text(12, H - 18, C_HINT, &format!("Column: {}  Cards: {}  N=new  Enter=promote right  D=demote left", col_name, cnt));
        }
        flush();
    }

    fn handle(&mut self, line: &str) {
        if self.new_mode {
            match line {
                "\x1b[A" | "\x1b[B" => {}
                "\r" | "" => {
                    if !self.new_buf.is_empty() {
                        self.cols[self.col].push(Card { title: self.new_buf.clone(), priority: self.new_pri });
                    }
                    self.new_mode = false;
                    self.new_buf.clear();
                    self.row = self.cols[self.col].len().saturating_sub(1);
                }
                "\x1b" | "\x03" => { self.new_mode = false; self.new_buf.clear(); }
                "\x7f" => { self.new_buf.pop(); }
                "1" => self.new_pri = 0,
                "2" => self.new_pri = 1,
                "3" => self.new_pri = 2,
                _ => {
                    if line.len() == 1 {
                        let b = line.as_bytes()[0];
                        if b.is_ascii_graphic() || b == b' ' { self.new_buf.push(b as char); }
                    }
                }
            }
            self.draw();
            return;
        }

        match line {
            "\t" => {
                self.col = (self.col + 1) % 4;
                self.clamp_row();
            }
            "\x1b[Z" => {
                self.col = (self.col + 3) % 4;
                self.clamp_row();
            }
            "\x1b[A" => {
                if self.row > 0 { self.row -= 1; }
            }
            "\x1b[B" => {
                if self.row + 1 < self.cols[self.col].len() { self.row += 1; }
            }
            "\r" | "" => {
                // promote: move selected card to next column
                if self.col < 3 && !self.cols[self.col].is_empty() {
                    let card = self.cols[self.col].remove(self.row);
                    self.cols[self.col + 1].push(card);
                    self.clamp_row();
                }
            }
            "d" | "D" => {
                // demote: move selected card to prev column
                if self.col > 0 && !self.cols[self.col].is_empty() {
                    let card = self.cols[self.col].remove(self.row);
                    self.cols[self.col - 1].push(card);
                    self.col -= 1;
                    self.row = self.cols[self.col].len() - 1;
                }
            }
            "n" | "N" => {
                self.new_mode = true;
                self.new_buf.clear();
                self.new_pri = 1;
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
