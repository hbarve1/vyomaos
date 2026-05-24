// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 800;
const HEADER_H: u32 = 48;
const CHAR_W: u32 = 8;
const LINE_H: u32 = 18;
const GUTTER_W: u32 = 44;
const STATUS_H: u32 = 24;
const VISIBLE_LINES: usize = ((H - HEADER_H - STATUS_H) / LINE_H) as usize;
const VISIBLE_COLS: usize = ((W - GUTTER_W) / CHAR_W) as usize;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD: u32    = 0x161B22FF;
const C_CUR_BG: u32  = 0x1C2128FF;
const C_GUTTER: u32  = 0x6E7681FF;

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

const KEYWORDS: &[&str] = &[
    "fn", "let", "mut", "if", "else", "for", "while", "struct", "impl",
    "use", "pub", "match", "return", "const", "static", "type", "enum",
    "trait", "mod", "in", "where", "self", "Self", "super", "crate",
    "move", "ref", "as", "true", "false", "break", "continue", "loop",
    "async", "await", "dyn", "Box", "Vec", "String", "Option", "Result",
    "Some", "None", "Ok", "Err",
];

// Returns a color per char index for the given line
fn colorize(line: &str) -> Vec<u32> {
    let chars: Vec<char> = line.chars().collect();
    let mut colors = vec![C_TEXT; chars.len()];
    let mut i = 0;
    while i < chars.len() {
        // Line comment
        if i + 1 < chars.len() && chars[i] == '/' && chars[i + 1] == '/' {
            for j in i..chars.len() { colors[j] = C_HINT; }
            break;
        }
        // String literal
        if chars[i] == '"' {
            colors[i] = C_GREEN;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                colors[i] = C_GREEN;
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    colors[i] = C_GREEN;
                }
                i += 1;
            }
            if i < chars.len() { colors[i] = C_GREEN; i += 1; }
            continue;
        }
        // Char literal
        if chars[i] == '\'' && i + 1 < chars.len() {
            colors[i] = C_GREEN;
            i += 1;
            if i < chars.len() && chars[i] == '\\' {
                colors[i] = C_GREEN; i += 1;
                if i < chars.len() { colors[i] = C_GREEN; i += 1; }
            } else if i < chars.len() && chars[i] != '\'' {
                colors[i] = C_GREEN; i += 1;
            }
            if i < chars.len() && chars[i] == '\'' { colors[i] = C_GREEN; i += 1; }
            continue;
        }
        // Number literal
        if chars[i].is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') { i += 1; }
            for j in start..i { colors[j] = C_ORANGE; }
            continue;
        }
        // Identifier or keyword
        if chars[i].is_alphabetic() || chars[i] == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') { i += 1; }
            let word: String = chars[start..i].iter().collect();
            let c = if KEYWORDS.contains(&word.as_str()) { C_SEL } else { C_TEXT };
            for j in start..i { colors[j] = c; }
            continue;
        }
        i += 1;
    }
    colors
}

struct Editor {
    lines:      Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
    scroll_row: usize,
    scroll_col: usize,
    filename:   String,
    modified:   bool,
    status:     String,
}

impl Editor {
    fn new() -> Self {
        Editor {
            lines: vec![String::new()],
            cursor_row: 0,
            cursor_col: 0,
            scroll_row: 0,
            scroll_col: 0,
            filename: "/data/code.rs".to_string(),
            modified: false,
            status: String::new(),
        }
    }

    fn clamp_col(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col > line_len { self.cursor_col = line_len; }
    }

    fn adjust_scroll(&mut self) {
        if self.cursor_row < self.scroll_row {
            self.scroll_row = self.cursor_row;
        }
        if self.cursor_row >= self.scroll_row + VISIBLE_LINES {
            self.scroll_row = self.cursor_row - VISIBLE_LINES + 1;
        }
        if self.cursor_col < self.scroll_col {
            self.scroll_col = self.cursor_col;
        }
        if self.cursor_col >= self.scroll_col + VISIBLE_COLS {
            self.scroll_col = self.cursor_col - VISIBLE_COLS + 1;
        }
    }

    fn move_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.clamp_col();
            self.adjust_scroll();
        }
    }

    fn move_down(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.clamp_col();
            self.adjust_scroll();
        }
    }

    fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
        }
        self.adjust_scroll();
    }

    fn move_right(&mut self) {
        let line_len = self.lines[self.cursor_row].len();
        if self.cursor_col < line_len {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
        self.adjust_scroll();
    }

    fn home(&mut self) {
        // Jump to first non-whitespace, or col 0 if already there
        let first_nonws = self.lines[self.cursor_row]
            .chars().take_while(|c| c.is_whitespace()).count();
        self.cursor_col = if self.cursor_col > first_nonws { first_nonws } else { 0 };
        self.adjust_scroll();
    }

    fn end(&mut self) {
        self.cursor_col = self.lines[self.cursor_row].len();
        self.adjust_scroll();
    }

    fn insert_char(&mut self, c: char) {
        self.lines[self.cursor_row].insert(self.cursor_col, c);
        self.cursor_col += 1;
        self.modified = true;
        self.adjust_scroll();
    }

    fn insert_tab(&mut self) {
        for _ in 0..4 { self.insert_char(' '); }
    }

    fn enter(&mut self) {
        let rest = self.lines[self.cursor_row][self.cursor_col..].to_string();
        self.lines[self.cursor_row].truncate(self.cursor_col);
        // Auto-indent: match leading whitespace of current line
        let indent: String = self.lines[self.cursor_row]
            .chars().take_while(|c| *c == ' ' || *c == '\t').collect();
        self.cursor_row += 1;
        self.lines.insert(self.cursor_row, indent + &rest);
        self.cursor_col = self.lines[self.cursor_row]
            .chars().take_while(|c| *c == ' ' || *c == '\t').count();
        self.modified = true;
        self.adjust_scroll();
    }

    fn backspace(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
            self.lines[self.cursor_row].remove(self.cursor_col);
            self.modified = true;
        } else if self.cursor_row > 0 {
            let rest = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.lines[self.cursor_row].len();
            self.lines[self.cursor_row].push_str(&rest);
            self.modified = true;
        }
        self.adjust_scroll();
    }

    fn save(&mut self) {
        let content = self.lines.join("\n");
        match std::fs::write(&self.filename, content.as_bytes()) {
            Ok(_) => {
                self.modified = false;
                self.status = format!("Saved {} ({} lines)", self.filename, self.lines.len());
            }
            Err(e) => { self.status = format!("Save error: {}", e); }
        }
    }

    fn load(&mut self) {
        match std::fs::read_to_string(&self.filename) {
            Ok(content) => {
                self.lines = content.lines().map(|l| l.to_string()).collect();
                if self.lines.is_empty() { self.lines.push(String::new()); }
                self.cursor_row = 0;
                self.cursor_col = 0;
                self.scroll_row = 0;
                self.scroll_col = 0;
                self.modified = false;
                self.status = format!("Loaded {} ({} lines)", self.filename, self.lines.len());
            }
            Err(e) => { self.status = format!("Load error: {}", e); }
        }
    }
}

fn draw(ed: &Editor) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Code Editor");
    let modified_str = if ed.modified { " [+]" } else { "" };
    text(160, 16, C_HINT, &format!("{}{}", ed.filename, modified_str));
    text(600, 16, C_HINT, "Ctrl+W:save  Ctrl+L:load  Tab:indent  Ctrl+C:quit");

    // Gutter background
    fill(0, HEADER_H, GUTTER_W, H - HEADER_H - STATUS_H, C_CARD);

    // Lines
    let end_row = (ed.scroll_row + VISIBLE_LINES).min(ed.lines.len());
    for row in ed.scroll_row..end_row {
        let screen_row = (row - ed.scroll_row) as u32;
        let y = HEADER_H + screen_row * LINE_H;

        // Current line highlight
        if row == ed.cursor_row {
            fill(GUTTER_W, y, W - GUTTER_W, LINE_H, C_CUR_BG);
        }

        // Line number
        let linenum = format!("{:4}", row + 1);
        let gutter_color = if row == ed.cursor_row { C_TEXT } else { C_GUTTER };
        text(2, y + 2, gutter_color, &linenum);

        // Code content with syntax highlighting
        let line = &ed.lines[row];
        let colors = colorize(line);
        let chars: Vec<char> = line.chars().collect();

        let end_col = (ed.scroll_col + VISIBLE_COLS).min(chars.len());
        if ed.scroll_col < chars.len() {
            let mut col = ed.scroll_col;
            let mut x = GUTTER_W;
            while col < end_col {
                let cur_color = colors[col];
                let run_start = col;
                while col < end_col && colors[col] == cur_color { col += 1; }
                let seg: String = chars[run_start..col].iter().collect();
                text(x, y + 2, cur_color, &seg);
                x += (col - run_start) as u32 * CHAR_W;
            }
        }

        // Cursor
        if row == ed.cursor_row {
            let cursor_screen_col = ed.cursor_col.saturating_sub(ed.scroll_col);
            if cursor_screen_col <= VISIBLE_COLS {
                let cx = GUTTER_W + cursor_screen_col as u32 * CHAR_W;
                fill(cx, y, 2, LINE_H, C_SEL);
            }
        }
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let pos = format!("Ln {}, Col {}", ed.cursor_row + 1, ed.cursor_col + 1);
    text(8, sb_y + 4, C_HINT, &pos);
    if !ed.status.is_empty() {
        text(200, sb_y + 4, C_GREEN, &ed.status);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut ed = Editor::new();

    println!("@supervisor: raise code-editor");
    let _ = io::stdout().flush();
    draw(&ed);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        ed.status.clear();

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b" => {}  // bare Escape — ignore
            "\x1b[A" => { ed.move_up(); }
            "\x1b[B" => { ed.move_down(); }
            "\x1b[C" => { ed.move_right(); }
            "\x1b[D" => { ed.move_left(); }
            "\x1b[H" | "\x1b[1~" => { ed.home(); }
            "\x1b[F" | "\x1b[4~" => { ed.end(); }
            "\x1b[5~" => { // Page Up
                for _ in 0..VISIBLE_LINES / 2 { ed.move_up(); }
            }
            "\x1b[6~" => { // Page Down
                for _ in 0..VISIBLE_LINES / 2 { ed.move_down(); }
            }
            "\x7f" => { ed.backspace(); }
            "" => { ed.enter(); }
            "\t" => { ed.insert_tab(); }
            "\x17" => { ed.save(); }    // Ctrl+W
            "\x0c" => { ed.load(); }    // Ctrl+L
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7f {
                    ed.insert_char(b as char);
                }
            }
            _ => {}
        }
        draw(&ed);
    }
}
