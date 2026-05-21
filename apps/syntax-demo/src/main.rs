use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HDR: u32 = 40;
const SB_H: u32 = 28;
const PANE_W: u32 = W / 2;     // 600 each
const CONTENT_Y: u32 = HDR;
const CONTENT_H: u32 = H - HDR - SB_H;
const LINE_H: u32 = 18;
const GUTTER_W: u32 = 40;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
const C_PURPLE: u32 = 0xBC8CFFFF;
const C_RED: u32    = 0xFF7B72FF;
const C_CARD: u32   = 0x161B22FF;

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

#[derive(Clone, Copy, PartialEq)]
enum Lang { Rust, Python, Json }

impl Lang {
    fn name(self) -> &'static str {
        match self { Lang::Rust => "Rust", Lang::Python => "Python", Lang::Json => "JSON" }
    }
    fn color(self) -> u32 {
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

const RUST_EXAMPLE: &[&str] = &[
    "use std::collections::HashMap;",
    "",
    "struct Cache {",
    "    data: HashMap<String, Vec<u32>>,",
    "    max_size: usize,",
    "}",
    "",
    "impl Cache {",
    "    fn new(max_size: usize) -> Self {",
    "        Cache { data: HashMap::new(), max_size }",
    "    }",
    "",
    "    fn insert(&mut self, key: String, val: u32) {",
    "        // Evict if at capacity",
    "        if self.data.len() >= self.max_size {",
    "            self.data.clear();",
    "        }",
    "        self.data.entry(key).or_default().push(val);",
    "    }",
    "",
    "    fn get(&self, key: &str) -> Option<&Vec<u32>> {",
    "        self.data.get(key)",
    "    }",
    "}",
    "",
    "fn main() {",
    "    let mut cache = Cache::new(100);",
    "    cache.insert(\"hits\".to_string(), 42);",
    "    if let Some(v) = cache.get(\"hits\") {",
    "        println!(\"found: {:?}\", v);",
    "    }",
    "}",
];

const PYTHON_EXAMPLE: &[&str] = &[
    "from dataclasses import dataclass, field",
    "from typing import Optional, List",
    "",
    "@dataclass",
    "class Node:",
    "    value: int",
    "    left: Optional['Node'] = None",
    "    right: Optional['Node'] = None",
    "",
    "class BST:",
    "    def __init__(self):",
    "        self.root = None",
    "",
    "    def insert(self, val: int) -> None:",
    "        # Recursive insert",
    "        def _insert(node, v):",
    "            if node is None:",
    "                return Node(v)",
    "            if v < node.value:",
    "                node.left = _insert(node.left, v)",
    "            else:",
    "                node.right = _insert(node.right, v)",
    "            return node",
    "        self.root = _insert(self.root, val)",
    "",
    "    def inorder(self) -> List[int]:",
    "        result = []",
    "        def _walk(n):",
    "            if n: _walk(n.left); result.append(n.value); _walk(n.right)",
    "        _walk(self.root)",
    "        return result",
    "",
    "if __name__ == '__main__':",
    "    t = BST()",
    "    for x in [5, 3, 7, 1, 4, 6, 8]:",
    "        t.insert(x)",
    "    print(t.inorder())  # [1, 3, 4, 5, 6, 7, 8]",
];

const JSON_EXAMPLE: &[&str] = &[
    "{",
    "  \"app\": \"VyomaOS\",",
    "  \"version\": \"1.0.0\",",
    "  \"architecture\": \"wasm32-wasip2\",",
    "  \"capabilities\": {",
    "    \"display\": true,",
    "    \"stdio\": true,",
    "    \"filesystem\": false,",
    "    \"network\": false",
    "  },",
    "  \"window\": {",
    "    \"x\": 60,",
    "    \"y\": 30,",
    "    \"width\": 1200,",
    "    \"height\": 760",
    "  },",
    "  \"dependencies\": [",
    "    \"wasmtime-43.0.0\",",
    "    \"linux-5.10-allnoconfig\",",
    "    \"musl-1.2.4\"",
    "  ],",
    "  \"build\": {",
    "    \"rust\": \"1.87.0\",",
    "    \"target\": \"wasm32-wasip2\",",
    "    \"profile\": \"release\",",
    "    \"opt_level\": \"z\",",
    "    \"strip\": true",
    "  }",
    "}",
];

struct App {
    lang:    Lang,
    lines:   Vec<String>,   // finalized lines
    cur:     String,        // current input buffer
    scroll:  usize,
    status:  String,
}

impl App {
    fn new() -> Self {
        let lines: Vec<String> = RUST_EXAMPLE.iter().map(|s| s.to_string()).collect();
        App {
            lang: Lang::Rust,
            lines,
            cur: String::new(),
            scroll: 0,
            status: String::from("Type to edit  Enter=newline  Backspace=del  1/2/3=lang  ↑↓=scroll  Ctrl+W=clear  Ctrl+C=quit"),
        }
    }

    fn all_lines(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.lines.iter().map(|s| s.as_str()).collect();
        v.push(self.cur.as_str());
        v
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HDR, C_HEADER);
    fill(0, HDR - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Syntax Highlighter");

    // Language tabs
    let tabs = [Lang::Rust, Lang::Python, Lang::Json];
    let mut tx = 200u32;
    for (i, &l) in tabs.iter().enumerate() {
        let active = l == app.lang;
        let col = if active { l.color() } else { C_HINT };
        let label = format!("[{}]{}", i + 1, l.name());
        if active {
            fill(tx - 2, 6, label.len() as u32 * 8 + 8, 26, C_CARD);
            border(tx - 2, 6, label.len() as u32 * 8 + 8, 26, l.color());
        }
        text(tx, 12, col, &label);
        tx += label.len() as u32 * 8 + 20;
    }

    // Pane headers
    fill(0, CONTENT_Y, PANE_W, 20, C_CARD);
    fill(PANE_W, CONTENT_Y, PANE_W, 20, C_CARD);
    text(GUTTER_W + 4, CONTENT_Y + 3, C_HINT, "Editor (editable)");
    text(PANE_W + GUTTER_W + 4, CONTENT_Y + 3, app.lang.color(), &format!("{} — Highlighted", app.lang.name()));
    fill(0, CONTENT_Y + 20, W, 1, C_BORDER);

    let code_y = CONTENT_Y + 21;
    let code_h = CONTENT_H - 21;
    let vis_lines = (code_h / LINE_H) as usize;

    fill(0, code_y, PANE_W, code_h, C_BG);
    fill(PANE_W, code_y, PANE_W, code_h, C_CARD);
    fill(PANE_W, code_y, 1, code_h, C_BORDER);

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
            fill(0, ly, PANE_W, LINE_H, 0x1A2030FF);
            fill(PANE_W, ly, PANE_W, LINE_H, 0x1A2030FF);
        }

        // Gutter
        fill(0, ly, GUTTER_W - 2, LINE_H, C_CARD);
        text(4, ly + 2, C_HINT, &format!("{:3}", li + 1));
        fill(GUTTER_W - 2, ly, 2, LINE_H, C_BORDER);

        fill(PANE_W, ly, GUTTER_W - 2, LINE_H, 0x0F1419FF);
        text(PANE_W + 4, ly + 2, C_HINT, &format!("{:3}", li + 1));
        fill(PANE_W + GUTTER_W - 2, ly, 2, LINE_H, C_BORDER);

        // Left pane: raw text
        let max_c = ((PANE_W - GUTTER_W - 4) / 8) as usize;
        let raw_shown = if line.len() > max_c { &line[..max_c] } else { line };
        let raw_col = if li == last_idx { C_TEXT } else { C_HINT };
        text(GUTTER_W + 4, ly + 2, raw_col, raw_shown);

        // Cursor blink on last line
        if li == last_idx {
            let cx = GUTTER_W + 4 + line.len() as u32 * 8;
            if cx < PANE_W - 4 {
                fill(cx, ly + 2, 2, 14, C_SEL);
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
                text(hx, ly + 2, *col, shown);
                hx += shown.len() as u32 * 8;
            }
        }
    }

    // Stats
    let stat_y = H - SB_H - 20;
    text(8, stat_y, C_HINT, &format!("Lines: {}  Scroll: {}/{}", all.len(), scroll + 1, all.len()));

    let sb_y = H - SB_H;
    fill(0, sb_y, W, SB_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise syntax-demo");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        let all_count = app.lines.len() + 1;
        let vis_lines = ((CONTENT_H - 21) / LINE_H) as usize;

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

            "\x7f" => {
                if !app.cur.is_empty() {
                    app.cur.pop();
                } else if !app.lines.is_empty() {
                    app.cur = app.lines.pop().unwrap_or_default();
                }
            }

            "" => {
                // Enter — finalize line
                app.lines.push(std::mem::take(&mut app.cur));
                // Auto-scroll to bottom
                if all_count + 1 > vis_lines {
                    app.scroll = all_count + 1 - vis_lines;
                }
            }

            // Scroll
            "\x1b[A" => { if app.scroll > 0 { app.scroll -= 1; } }
            "\x1b[B" => {
                let max = (app.lines.len() + 1).saturating_sub(vis_lines);
                if app.scroll < max { app.scroll += 1; }
            }

            // Language switch
            "1" => {
                app.lang = Lang::Rust;
                app.lines = RUST_EXAMPLE.iter().map(|s| s.to_string()).collect();
                app.cur.clear(); app.scroll = 0;
                app.status = "Switched to Rust example.".to_string();
            }
            "2" => {
                app.lang = Lang::Python;
                app.lines = PYTHON_EXAMPLE.iter().map(|s| s.to_string()).collect();
                app.cur.clear(); app.scroll = 0;
                app.status = "Switched to Python example.".to_string();
            }
            "3" => {
                app.lang = Lang::Json;
                app.lines = JSON_EXAMPLE.iter().map(|s| s.to_string()).collect();
                app.cur.clear(); app.scroll = 0;
                app.status = "Switched to JSON example.".to_string();
            }

            // Ctrl+W = clear
            "\x17" => {
                app.lines.clear();
                app.cur.clear();
                app.scroll = 0;
                app.status = "Cleared.".to_string();
            }

            s if s.len() == 1 && s.chars().next().map_or(false, |c| !c.is_control()) => {
                app.cur.push_str(s);
            }

            _ => {}
        }

        draw(&app);
    }
}
