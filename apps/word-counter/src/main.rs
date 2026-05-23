use std::io::{self, BufRead, Write};

const W: u32 = 880;
const H: u32 = 640;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_CARD: u32    = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;

const TEXT_X: u32 = 16;
const TEXT_Y: u32 = HEADER_H + 8;
const TEXT_W: u32 = 620;
const TEXT_H: u32 = H - TEXT_Y - 16;
const STATS_X: u32 = TEXT_X + TEXT_W + 16;
const STATS_W: u32 = W - STATS_X - 12;
const CHAR_W: u32 = 9;
const LINE_H: u32 = 18;
const COLS: usize = (TEXT_W / CHAR_W) as usize; // ~68
const ROWS: usize = (TEXT_H / LINE_H) as usize;  // ~32

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

struct Editor {
    // Store text as array of lines
    lines:    Vec<Vec<u8>>,
    cur_row:  usize,
    cur_col:  usize,
    scroll:   usize,
}

impl Editor {
    fn new() -> Self {
        Editor {
            lines:   vec![Vec::new()],
            cur_row: 0,
            cur_col: 0,
            scroll:  0,
        }
    }

    fn insert(&mut self, b: u8) {
        let col = self.cur_col;
        self.lines[self.cur_row].insert(col, b);
        self.cur_col += 1;
    }

    fn newline(&mut self) {
        let rest: Vec<u8> = self.lines[self.cur_row].split_off(self.cur_col);
        self.cur_row += 1;
        self.lines.insert(self.cur_row, rest);
        self.cur_col = 0;
        if self.cur_row >= self.scroll + ROWS { self.scroll += 1; }
    }

    fn backspace(&mut self) {
        if self.cur_col > 0 {
            self.cur_col -= 1;
            self.lines[self.cur_row].remove(self.cur_col);
        } else if self.cur_row > 0 {
            let line = self.lines.remove(self.cur_row);
            self.cur_row -= 1;
            self.cur_col = self.lines[self.cur_row].len();
            self.lines[self.cur_row].extend_from_slice(&line);
            if self.scroll > 0 && self.cur_row < self.scroll { self.scroll -= 1; }
        }
    }

    fn clear(&mut self) {
        self.lines = vec![Vec::new()];
        self.cur_row = 0;
        self.cur_col = 0;
        self.scroll = 0;
    }

    fn full_text(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, line) in self.lines.iter().enumerate() {
            out.extend_from_slice(line);
            if i + 1 < self.lines.len() { out.push(b'\n'); }
        }
        out
    }

    fn word_count(&self) -> usize {
        let txt = self.full_text();
        let s = std::str::from_utf8(&txt).unwrap_or("");
        s.split_whitespace().count()
    }

    fn char_count_total(&self) -> usize {
        self.lines.iter().map(|l| l.len()).sum::<usize>() + self.lines.len().saturating_sub(1)
    }

    fn char_count_no_space(&self) -> usize {
        self.lines.iter().map(|l| l.iter().filter(|&&b| b != b' ').count()).sum()
    }

    fn line_count(&self) -> usize { self.lines.len() }

    fn sentence_count(&self) -> usize {
        let txt = self.full_text();
        txt.iter().filter(|&&b| b == b'.' || b == b'!' || b == b'?').count().max(if txt.is_empty() { 0 } else { 1 })
    }

    fn para_count(&self) -> usize {
        let mut paras = 0;
        let mut in_para = false;
        for line in &self.lines {
            if line.is_empty() { in_para = false; } else { if !in_para { paras += 1; } in_para = true; }
        }
        paras
    }

    fn avg_word_len(&self) -> usize {
        let wc = self.word_count();
        if wc == 0 { return 0; }
        self.char_count_no_space() / wc
    }

    fn avg_sent_len(&self) -> usize {
        let sc = self.sentence_count();
        if sc == 0 { return 0; }
        self.word_count() / sc
    }
}

fn draw(ed: &Editor) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Word Counter");
    text(180, 16, C_HINT, &format!("Words: {}  Chars: {}", ed.word_count(), ed.char_count_total()));
    text(520, 16, C_HINT, "Ctrl+A:clear");

    // Text area
    fill(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, C_CARD);
    border(TEXT_X, TEXT_Y, TEXT_W, TEXT_H, C_BORDER);

    for (vi, li) in (ed.scroll..).zip(0..ROWS) {
        if vi >= ed.lines.len() { break; }
        let line = &ed.lines[vi];
        let ly = TEXT_Y + li as u32 * LINE_H + 4;

        // Display line (truncate at COLS)
        let display = &line[..line.len().min(COLS)];
        if let Ok(s) = std::str::from_utf8(display) {
            text(TEXT_X + 4, ly, C_TEXT, s);
        }

        // Cursor
        if vi == ed.cur_row {
            let cx = TEXT_X + 4 + ed.cur_col.min(COLS) as u32 * CHAR_W;
            fill(cx, ly, 2, LINE_H - 2, C_SEL);
        }
    }

    // Stats panel
    fill(STATS_X, TEXT_Y, STATS_W, TEXT_H, C_CARD);
    border(STATS_X, TEXT_Y, STATS_W, TEXT_H, C_BORDER);

    let stats = [
        ("Words",       ed.word_count().to_string(),           C_GREEN),
        ("Characters",  ed.char_count_total().to_string(),     C_TEXT),
        ("No spaces",   ed.char_count_no_space().to_string(),  C_TEXT),
        ("Lines",       ed.line_count().to_string(),           C_TEXT),
        ("Sentences",   ed.sentence_count().to_string(),       C_TEXT),
        ("Paragraphs",  ed.para_count().to_string(),           C_ORANGE),
        ("Avg word len",ed.avg_word_len().to_string(),         C_HINT),
        ("Avg sent len",ed.avg_sent_len().to_string(),         C_HINT),
    ];

    let mut sy = TEXT_Y + 12;
    for &(label, ref val, col) in &stats {
        text(STATS_X + 8, sy, C_HINT, label);
        sy += 16;
        text(STATS_X + 8, sy, col, val);
        sy += 24;
        fill(STATS_X + 8, sy, STATS_W - 16, 1, C_BORDER);
        sy += 8;
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut ed = Editor::new();

    println!("@supervisor: raise word-counter");
    let _ = io::stdout().flush();
    draw(&ed);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x01" => { ed.clear(); } // Ctrl+A
            "\r" | "" => { ed.newline(); }
            "\x7f" => { ed.backspace(); }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7F { ed.insert(b); }
            }
            _ => {}
        }
        draw(&ed);
    }
}
