use std::io::{self, BufRead, Write};

const W: u32 = 880;
const H: u32 = 680;
const HEADER_H: u32 = 40;
const STATUS_H: u32 = 24;
const CONTENT_Y: u32 = HEADER_H;
const CONTENT_H: u32 = H - HEADER_H - STATUS_H;
const LIST_W: u32 = 520;
const PREVIEW_X: u32 = LIST_W + 1;
const PREVIEW_W: u32 = W - LIST_W - 1;
const PREVIEW_H: u32 = CONTENT_H / 2;
const CAT_Y: u32 = CONTENT_Y + PREVIEW_H + 1;
const CAT_H: u32 = CONTENT_H - PREVIEW_H - 1;
const LINE_H: u32 = 18;
const CHAR_W: u32 = 8;
const MAX_ENTRIES: usize = 50;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_PURPLE: u32 = 0xBC8CFFFF;
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
enum Category { Text, Code, Url, Number }

impl Category {
    fn detect(s: &str) -> Self {
        let t = s.trim();
        if t.starts_with("http://") || t.starts_with("https://") { return Category::Url; }
        if t.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '-') && !t.is_empty() { return Category::Number; }
        if t.contains('{') || t.contains("fn ") || t.contains("let ") || t.contains("//") { return Category::Code; }
        Category::Text
    }
    fn name(self) -> &'static str {
        match self { Category::Text => "Text", Category::Code => "Code", Category::Url => "URL", Category::Number => "Number" }
    }
    fn color(self) -> u32 {
        match self { Category::Text => C_TEXT, Category::Code => C_GREEN, Category::Url => C_SEL, Category::Number => C_YELLOW }
    }
    fn icon(self) -> &'static str {
        match self { Category::Text => "T", Category::Code => "<>", Category::Url => "@", Category::Number => "#" }
    }
}

#[derive(Clone)]
struct Entry {
    content: String,
    tick:    u64,
    cat:     Category,
    pinned:  bool,
}

impl Entry {
    fn new(content: String, tick: u64) -> Self {
        let cat = Category::detect(&content);
        Entry { content, tick, cat, pinned: false }
    }

    fn display(&self) -> String {
        let s = self.content.replace('\n', "↵");
        if s.len() > 72 { format!("{}…", &s[..71]) } else { s }
    }
}

enum Mode { Normal, Search }

struct App {
    entries:    Vec<Entry>,
    sel:        usize,
    tick:       u64,
    mode:       Mode,
    search_buf: String,
    cat_filter: Option<Category>,
    status:     String,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            entries: Vec::new(),
            sel: 0,
            tick: 0,
            mode: Mode::Normal,
            search_buf: String::new(),
            cat_filter: None,
            status: String::from("Enter=copy  P=pin  Del=remove  /=search  1-4=filter  Ctrl+C=quit"),
        };
        // Pre-populate with demo entries
        let demos: &[&str] = &[
            "https://github.com/hbarve1/vyomaos",
            "fn main() {\n    println!(\"Hello, VyomaOS!\");\n}",
            "VyomaOS — A WASM-first operating system",
            "42",
            "https://wasmtime.dev/",
            "let supervisor = Supervisor::new(boot_toml);",
            "cargo build --target wasm32-wasip2 --release",
            "make run-gui DISPLAY_BACKEND=cocoa",
            "3.14159265358979",
            "The quick brown fox jumps over the lazy dog",
            "https://doc.rust-lang.org/book/",
            "struct App { entries: Vec<Entry>, sel: usize }",
            "echo 'Hello from VyomaOS shell'",
            "256",
            "Contact: honey.barve@gruve.ai",
        ];
        for (i, d) in demos.iter().enumerate() {
            a.entries.push(Entry::new(d.to_string(), i as u64));
        }
        a
    }

    fn visible(&self) -> Vec<usize> {
        self.entries.iter().enumerate()
            .filter(|(_, e)| {
                let cat_ok = self.cat_filter.map(|f| e.cat == f).unwrap_or(true);
                let search_ok = if self.search_buf.is_empty() {
                    true
                } else {
                    e.content.to_lowercase().contains(&self.search_buf.to_lowercase())
                };
                cat_ok && search_ok
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn add(&mut self, content: String) {
        if content.is_empty() { return; }
        // Remove duplicate
        self.entries.retain(|e| e.content != content);
        let e = Entry::new(content, self.tick);
        self.entries.insert(0, e);
        if self.entries.len() > MAX_ENTRIES {
            // Remove last non-pinned
            if let Some(pos) = self.entries.iter().rposition(|e| !e.pinned) {
                self.entries.remove(pos);
            }
        }
        self.sel = 0;
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Clipboard Pro");
    text(180, 12, C_HINT, &format!("{} entries  {} pinned",
        app.entries.len(),
        app.entries.iter().filter(|e| e.pinned).count()));

    let vis = app.visible();
    let list_lines = (CONTENT_H / LINE_H) as usize;

    // Search bar
    let list_start_y = CONTENT_Y;
    match &app.mode {
        Mode::Search => {
            fill(0, list_start_y, LIST_W, LINE_H, 0x1C2D4EFF);
            border(0, list_start_y, LIST_W, LINE_H, C_SEL);
            text(8, list_start_y + 3, C_TEXT, &format!("/ {}▌", app.search_buf));
        }
        Mode::Normal => {
            if !app.search_buf.is_empty() {
                fill(0, list_start_y, LIST_W, LINE_H, C_CARD);
                text(8, list_start_y + 3, C_SEL, &format!("/ {} ({} results)", app.search_buf, vis.len()));
            }
        }
    }
    let list_y0 = list_start_y + if !app.search_buf.is_empty() || matches!(app.mode, Mode::Search) { LINE_H } else { 0 };

    // Entry list
    fill(0, list_y0, LIST_W, CONTENT_H - (list_y0 - CONTENT_Y), C_BG);
    fill(LIST_W, CONTENT_Y, 1, CONTENT_H, C_BORDER);

    let scroll = if vis.is_empty() { 0 } else {
        let sel_pos = vis.iter().position(|&i| i == app.sel.min(app.entries.len().saturating_sub(1))).unwrap_or(0);
        sel_pos.saturating_sub(list_lines - 1)
    };

    for (vi, &ei) in vis.iter().skip(scroll).take(list_lines).enumerate() {
        let e = &app.entries[ei];
        let ly = list_y0 + vi as u32 * LINE_H;
        if ly + LINE_H > CONTENT_Y + CONTENT_H { break; }

        let is_sel = ei == app.sel.min(app.entries.len().saturating_sub(1));
        if is_sel { fill(0, ly, LIST_W, LINE_H, 0x1C2D4EFF); }

        // Category icon
        let icon = e.cat.icon();
        let ic = e.cat.color();
        text(4, ly + 2, ic, icon);

        // Pin star
        if e.pinned { text(20, ly + 2, C_YELLOW, "★"); }

        // Content
        let cx = if e.pinned { 36 } else { 28 };
        let max_chars = ((LIST_W - cx - 80) / CHAR_W) as usize;
        let disp = e.display();
        let shown = if disp.len() > max_chars { &disp[..max_chars] } else { &disp };
        let tc = if is_sel { C_SEL } else { C_TEXT };
        text(cx, ly + 2, tc, shown);

        // Tick (right-aligned)
        let tick_s = format!("t{}", e.tick);
        text(LIST_W - tick_s.len() as u32 * CHAR_W - 4, ly + 2, C_HINT, &tick_s);
    }

    if vis.is_empty() {
        text(8, list_y0 + 8, C_HINT, "No entries match.");
    }

    // Preview panel (top-right)
    fill(PREVIEW_X, CONTENT_Y, PREVIEW_W, PREVIEW_H, C_CARD);
    fill(PREVIEW_X, CONTENT_Y + PREVIEW_H, PREVIEW_W, 1, C_BORDER);
    text(PREVIEW_X + 8, CONTENT_Y + 4, C_HINT, "Preview");
    fill(PREVIEW_X, CONTENT_Y + LINE_H, PREVIEW_W, 1, C_BORDER);

    if let Some(&ei) = vis.get(vis.iter().position(|&i| i == app.sel.min(app.entries.len().saturating_sub(1))).unwrap_or(0).min(vis.len().saturating_sub(1)).into()) {
        // Simple: show the selected entry
        let ei = app.sel.min(app.entries.len().saturating_sub(1));
        if ei < app.entries.len() {
            let e = &app.entries[ei];
            let max_w = ((PREVIEW_W - 16) / CHAR_W) as usize;
            let mut row = 0u32;
            for line in e.content.lines().take(((PREVIEW_H - LINE_H - 8) / LINE_H) as usize) {
                let shown = if line.len() > max_w { &line[..max_w] } else { line };
                text(PREVIEW_X + 8, CONTENT_Y + LINE_H + 4 + row * LINE_H, e.cat.color(), shown);
                row += 1;
            }
            text(PREVIEW_X + 8, CONTENT_Y + PREVIEW_H - LINE_H, C_HINT,
                &format!("{} · {} chars · tick {}", e.cat.name(), e.content.len(), e.tick));
        }
    }

    // Category breakdown panel (bottom-right)
    fill(PREVIEW_X, CAT_Y, PREVIEW_W, CAT_H, C_CARD);
    border(PREVIEW_X, CAT_Y, PREVIEW_W, CAT_H, C_BORDER);
    text(PREVIEW_X + 8, CAT_Y + 4, C_HINT, "Categories  (1-4 to filter)");
    fill(PREVIEW_X, CAT_Y + LINE_H, PREVIEW_W, 1, C_BORDER);

    let cats = [Category::Text, Category::Code, Category::Url, Category::Number];
    for (i, cat) in cats.iter().enumerate() {
        let cy = CAT_Y + LINE_H + 4 + i as u32 * LINE_H;
        let cnt = app.entries.iter().filter(|e| e.cat == *cat).count();
        let is_active = app.cat_filter == Some(*cat);
        let col = if is_active { cat.color() } else { C_HINT };
        let prefix = if is_active { "▶ " } else { "  " };
        text(PREVIEW_X + 8, cy, col, &format!("{}{}: {}  ({})", prefix, i + 1, cat.name(), cnt));
    }
    if app.cat_filter.is_none() {
        text(PREVIEW_X + 8, CAT_Y + LINE_H + 4 + cats.len() as u32 * LINE_H, C_SEL, "  0: All");
    }

    // Keys hint
    let hint_y = CAT_Y + CAT_H - LINE_H - 4;
    text(PREVIEW_X + 8, hint_y, C_HINT, "Enter=copy  P=pin  Del=remove");

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise clipboard-pro");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        app.tick += 1;

        if matches!(app.mode, Mode::Search) {
            match raw.as_str() {
                "\x03" | "\x1b" => {
                    app.mode = Mode::Normal;
                    app.search_buf.clear();
                    app.status = "Search cleared.".to_string();
                }
                "\x7f" => { app.search_buf.pop(); }
                "" => { app.mode = Mode::Normal; }
                s if s.len() == 1 && !s.chars().next().map(|c| c.is_control()).unwrap_or(true) => {
                    app.search_buf.push_str(s);
                }
                _ => {}
            }
            draw(&app);
            continue;
        }

        let vis = app.visible();

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                if app.sel > 0 { app.sel -= 1; }
            }
            "\x1b[B" => {
                if app.sel + 1 < app.entries.len() { app.sel += 1; }
            }
            "\x1b[5~" => { app.sel = app.sel.saturating_sub(5); }
            "\x1b[6~" => { app.sel = (app.sel + 5).min(app.entries.len().saturating_sub(1)); }
            "" => {
                if app.sel < app.entries.len() {
                    let content = app.entries[app.sel].content.clone();
                    let escaped = content.replace('\n', " ");
                    println!("@supervisor: clipboard-set {}", escaped);
                    let _ = io::stdout().flush();
                    app.status = format!("Copied: {}", if escaped.len() > 50 { &escaped[..50] } else { &escaped });
                }
            }
            "p" | "P" => {
                if app.sel < app.entries.len() {
                    app.entries[app.sel].pinned = !app.entries[app.sel].pinned;
                    let pinned = app.entries[app.sel].pinned;
                    app.status = format!("Entry {}pinned.", if pinned { "" } else { "un" });
                }
            }
            "\x7f" | "\x1b[3~" => {
                if app.sel < app.entries.len() && !app.entries[app.sel].pinned {
                    app.entries.remove(app.sel);
                    if app.sel > 0 && app.sel >= app.entries.len() { app.sel -= 1; }
                    app.status = "Entry removed.".to_string();
                } else if app.sel < app.entries.len() {
                    app.status = "Pinned entries cannot be deleted. Press P to unpin first.".to_string();
                }
            }
            "/" => {
                app.mode = Mode::Search;
                app.search_buf.clear();
            }
            "0" => { app.cat_filter = None; app.status = "Filter: All".to_string(); }
            "1" => { app.cat_filter = Some(Category::Text); app.status = "Filter: Text".to_string(); }
            "2" => { app.cat_filter = Some(Category::Code); app.status = "Filter: Code".to_string(); }
            "3" => { app.cat_filter = Some(Category::Url); app.status = "Filter: URL".to_string(); }
            "4" => { app.cat_filter = Some(Category::Number); app.status = "Filter: Number".to_string(); }
            _ => {}
        }
        draw(&app);
    }
}
