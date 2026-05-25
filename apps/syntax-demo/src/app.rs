// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use super::{
    C_BG, C_HEADER, C_BORDER, C_TEXT, C_HINT, C_ORANGE, C_GREEN, C_SEL, C_YELLOW, C_PURPLE,
    C_RED, C_CARD, W, H, HDR, SB_H, PANE_W, CONTENT_Y, CONTENT_H, LINE_H, GUTTER_W,
    RUST_EXAMPLE, PYTHON_EXAMPLE, JSON_EXAMPLE,
};

#[derive(Clone, Copy, PartialEq)]
pub enum Lang { Rust, Python, Json }

impl Lang {
    pub fn name(self) -> &'static str {
        match self { Lang::Rust => "Rust", Lang::Python => "Python", Lang::Json => "JSON" }
    }
    pub fn color(self) -> u32 {
        match self { Lang::Rust => C_ORANGE, Lang::Python => C_YELLOW, Lang::Json => C_SEL }
    }
    fn comment_start(self) -> &'static str {
        match self { Lang::Rust => "//", Lang::Python => "#", Lang::Json => "" }
    }
}

const RUST_KW: &[&str] = &[
    "fn","let","mut","const","static","struct","enum","impl","trait","type","use","pub",
    "mod","match","if","else","for","while","loop","return","break","continue","self",
    "Self","true","false","in","as","where","crate","super","ref","move","unsafe","async",
    "await","dyn","extern","derive","macro_rules",
];
const RUST_TYPES: &[&str] = &[
    "i8","i16","i32","i64","i128","isize","u8","u16","u32","u64","u128","usize",
    "f32","f64","bool","char","String","str","Vec","Option","Result","Box","Arc","Rc",
    "HashMap","HashSet","BTreeMap","BTreeSet","Mutex","RwLock",
];
const PY_KW: &[&str] = &[
    "def","class","import","from","return","if","else","elif","for","while","with","as",
    "in","not","and","or","True","False","None","pass","break","continue","lambda","yield",
    "try","except","finally","raise","del","global","nonlocal","assert","is",
];
const PY_BUILTIN: &[&str] = &[
    "print","len","range","list","dict","set","tuple","str","int","float","bool","type",
    "isinstance","enumerate","zip","map","filter","sorted","reversed","open","input",
    "max","min","sum","abs","round","super","property","staticmethod","classmethod",
];
const JSON_KW: &[&str] = &["true","false","null"];

// A highlighted span: (color, text)
type Span = (u32, String);

fn word_color(w: &str, lang: Lang) -> Option<u32> {
    match lang {
        Lang::Rust => {
            if RUST_KW.contains(&w)    { Some(C_ORANGE) }
            else if RUST_TYPES.contains(&w) { Some(C_YELLOW) }
            else { None }
        }
        Lang::Python => {
            if PY_KW.contains(&w)      { Some(C_ORANGE) }
            else if PY_BUILTIN.contains(&w) { Some(C_YELLOW) }
            else { None }
        }
        Lang::Json => {
            if JSON_KW.contains(&w)    { Some(C_ORANGE) }
            else { None }
        }
    }
}

fn highlight_line(line: &str, lang: Lang) -> Vec<Span> {
    let mut spans: Vec<Span> = Vec::new();
    let comment = lang.comment_start();

    // Full-line comment
    if !comment.is_empty() {
        let trimmed = line.trim_start();
        if trimmed.starts_with(comment) {
            return vec![(C_HINT, line.to_string())];
        }
    }

    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut cur_text = String::new();
    let mut cur_col = C_TEXT;

    macro_rules! flush_span {
        () => {
            if !cur_text.is_empty() {
                spans.push((cur_col, std::mem::take(&mut cur_text)));
                cur_col = C_TEXT;
            }
        };
    }

    while i < len {
        // Inline comment (Rust //)
        if lang == Lang::Rust && i + 1 < len && bytes[i] == b'/' && bytes[i+1] == b'/' {
            flush_span!();
            spans.push((C_HINT, line[i..].to_string()));
            return spans;
        }
        // Inline comment (Python #)
        if lang == Lang::Python && bytes[i] == b'#' {
            flush_span!();
            spans.push((C_HINT, line[i..].to_string()));
            return spans;
        }

        // String literal " or '
        if bytes[i] == b'"' || (bytes[i] == b'\'' && lang != Lang::Rust) {
            flush_span!();
            let quote = bytes[i];
            let mut s = String::new();
            s.push(bytes[i] as char);
            i += 1;
            while i < len {
                s.push(bytes[i] as char);
                if bytes[i] == quote && (i == 0 || bytes[i-1] != b'\\') {
                    i += 1;
                    break;
                }
                i += 1;
            }
            spans.push((C_GREEN, s));
            continue;
        }

        // Number
        if bytes[i].is_ascii_digit() {
            flush_span!();
            let mut num = String::new();
            while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'.' || bytes[i] == b'x' || bytes[i] == b'_') {
                num.push(bytes[i] as char);
                i += 1;
            }
            spans.push((C_PURPLE, num));
            continue;
        }

        // Identifier / keyword
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            flush_span!();
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word = &line[start..i];
            let col = word_color(word, lang).unwrap_or(C_TEXT);
            spans.push((col, word.to_string()));
            continue;
        }

        // JSON key (inside quotes before colon)
        if lang == Lang::Json && bytes[i] == b'"' {
            let mut s = String::new();
            s.push('"');
            i += 1;
            while i < len && bytes[i] != b'"' {
                s.push(bytes[i] as char);
                i += 1;
            }
            if i < len { s.push('"'); i += 1; }
            // peek for ":"
            let peek_i = i;
            let mut j = peek_i;
            while j < len && bytes[j] == b' ' { j += 1; }
            let col = if j < len && bytes[j] == b':' { C_SEL } else { C_GREEN };
            flush_span!();
            spans.push((col, s));
            continue;
        }

        // Operators
        let op_col = match bytes[i] {
            b'+' | b'-' | b'*' | b'/' | b'=' | b'<' | b'>' | b'!' | b'&' | b'|' | b'^' | b'%' | b'~' => C_SEL,
            b'(' | b')' | b'[' | b']' | b'{' | b'}' => C_TEXT,
            b':' | b',' | b';' | b'.' => C_HINT,
            _ => C_TEXT,
        };

        if op_col != cur_col {
            flush_span!();
            cur_col = op_col;
        }
        cur_text.push(bytes[i] as char);
        i += 1;
    }

    flush_span!();
    spans
}

pub struct App {
    pub lang:    Lang,
    pub lines:   Vec<String>,   // finalized lines
    pub cur:     String,        // current input buffer
    pub scroll:  usize,
    pub status:  String,
}

impl App {
    pub fn new() -> Self {
        let lines: Vec<String> = RUST_EXAMPLE.iter().map(|s| s.to_string()).collect();
        App {
            lang: Lang::Rust,
            lines,
            cur: String::new(),
            scroll: 0,
            status: String::from("Type to edit  Enter=newline  Backspace=del  1/2/3=lang  ↑↓=scroll  Ctrl+W=clear  Ctrl+C=quit"),
        }
    }

    pub fn all_lines(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.lines.iter().map(|s| s.as_str()).collect();
        v.push(self.cur.as_str());
        v
    }
}

pub fn draw(app: &App) {
    super::fill(0, 0, W, H, C_BG);

    // Header
    super::fill(0, 0, W, HDR, C_HEADER);
    super::fill(0, HDR - 1, W, 1, C_BORDER);
    super::text(16, 12, C_ORANGE, "Syntax Highlighter");

    // Language tabs
    let tabs = [Lang::Rust, Lang::Python, Lang::Json];
    let mut tx = 200u32;
    for (i, &l) in tabs.iter().enumerate() {
        let active = l == app.lang;
        let col = if active { l.color() } else { C_HINT };
        let label = format!("[{}]{}", i + 1, l.name());
        if active {
            super::fill(tx - 2, 6, label.len() as u32 * 8 + 8, 26, C_CARD);
            super::border(tx - 2, 6, label.len() as u32 * 8 + 8, 26, l.color());
        }
        super::text(tx, 12, col, &label);
        tx += label.len() as u32 * 8 + 20;
    }

    // Pane headers
    super::fill(0, CONTENT_Y, PANE_W, 20, C_CARD);
    super::fill(PANE_W, CONTENT_Y, PANE_W, 20, C_CARD);
    super::text(GUTTER_W + 4, CONTENT_Y + 3, C_HINT, "Editor (editable)");
    super::text(PANE_W + GUTTER_W + 4, CONTENT_Y + 3, app.lang.color(), &format!("{} — Highlighted", app.lang.name()));
    super::fill(0, CONTENT_Y + 20, W, 1, C_BORDER);

    let code_y = CONTENT_Y + 21;
    let code_h = CONTENT_H - 21;
    let vis_lines = (code_h / LINE_H) as usize;

    super::fill(0, code_y, PANE_W, code_h, C_BG);
    super::fill(PANE_W, code_y, PANE_W, code_h, C_CARD);
    super::fill(PANE_W, code_y, 1, code_h, C_BORDER);

    let all = app.all_lines();
    let scroll = app.scroll.min(all.len().saturating_sub(1));
    let last_idx = all.len() - 1;

    for vi in 0..vis_lines {
        let li = scroll + vi;
        if li >= all.len() { break; }
        let line = all[li];
        let ly = code_y + vi as u32 * LINE_H;

        // Current line highlight
        if li == last_idx {
            super::fill(0, ly, PANE_W, LINE_H, 0x1A2030FF);
            super::fill(PANE_W, ly, PANE_W, LINE_H, 0x1A2030FF);
        }

        // Gutter
        super::fill(0, ly, GUTTER_W - 2, LINE_H, C_CARD);
        super::text(4, ly + 2, C_HINT, &format!("{:3}", li + 1));
        super::fill(GUTTER_W - 2, ly, 2, LINE_H, C_BORDER);

        super::fill(PANE_W, ly, GUTTER_W - 2, LINE_H, 0x0F1419FF);
        super::text(PANE_W + 4, ly + 2, C_HINT, &format!("{:3}", li + 1));
        super::fill(PANE_W + GUTTER_W - 2, ly, 2, LINE_H, C_BORDER);

        // Left pane: raw text
        let max_c = ((PANE_W - GUTTER_W - 4) / 8) as usize;
        let raw_shown = if line.len() > max_c { &line[..max_c] } else { line };
        let raw_col = if li == last_idx { C_TEXT } else { C_HINT };
        super::text(GUTTER_W + 4, ly + 2, raw_col, raw_shown);

        // Cursor blink on last line
        if li == last_idx {
            let cx = GUTTER_W + 4 + line.len() as u32 * 8;
            if cx < PANE_W - 4 {
                super::fill(cx, ly + 2, 2, 14, C_SEL);
            }
        }

        // Right pane: highlighted
        let spans = highlight_line(line, app.lang);
        let mut hx = PANE_W + GUTTER_W + 4;
        let max_right = PANE_W + W - 4;
        for (col, seg) in &spans {
            if hx >= max_right { break; }
            let avail = ((max_right - hx) / 8) as usize;
            let shown = if seg.len() > avail { &seg[..avail] } else { seg.as_str() };
            if !shown.is_empty() {
                super::text(hx, ly + 2, *col, shown);
                hx += shown.len() as u32 * 8;
            }
        }
    }

    // Stats
    let stat_y = H - SB_H - 20;
    super::text(8, stat_y, C_HINT, &format!("Lines: {}  Scroll: {}/{}", all.len(), scroll + 1, all.len()));

    let sb_y = H - SB_H;
    super::fill(0, sb_y, W, SB_H, C_HEADER);
    super::fill(0, sb_y, W, 1, C_BORDER);
    super::text(8, sb_y + 6, C_HINT, &app.status);

    super::flush();
}
