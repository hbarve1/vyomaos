use std::io::{self, BufRead, Write};

const W: u32 = 1100;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 32;
const SIDEBAR_W: u32 = 260;
const CHAR_W: u32 = 8;
const CHAR_H: u32 = 16;
const LINE_H: u32 = 18;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD: u32    = 0x161B22FF;
const C_SEL_BG: u32  = 0x1C2D4EFF;

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

#[derive(Clone)]
struct Note {
    title: String,
    body:  Vec<String>, // lines
}

impl Note {
    fn new(title: &str, body: &str) -> Self {
        Note {
            title: title.to_string(),
            body: body.lines().map(|l| l.to_string()).collect(),
        }
    }
}

#[derive(PartialEq)]
enum Focus { List, Editor }

struct App {
    notes:      Vec<Note>,
    sel:        usize,
    focus:      Focus,
    scroll_ed:  usize, // editor scroll (line index)
    cursor_row: usize, // editor cursor row within note body
    cursor_col: usize, // editor cursor col within current line
    search:     Option<String>, // active search filter
    search_buf: String,
    searching:  bool,
    adding:     bool,
    add_buf:    String,
    status:     String,
}

impl App {
    fn new() -> Self {
        App {
            notes: vec![
                Note::new(
                    "Welcome to Notes",
                    "This is your note-taking app.\n\nUse Tab to switch between the list and editor.\nCtrl+N to create a new note.\nCtrl+W to save all notes to /data/notes.csv.",
                ),
                Note::new(
                    "VyomaOS Architecture",
                    "Key components:\n- Linux kernel (hardware only)\n- Rust supervisor (PID 1)\n- Wasmtime (WASI Preview 2)\n- WASM apps (wasm32-wasip2)\n\nAll apps communicate via VYOMA_DRAW and IPC broker.",
                ),
                Note::new(
                    "Ideas",
                    "Things to build next:\n- Markdown Editor with split preview\n- Terminal Emulator v2 with VT100\n- More games: Pac-Man, Sokoban\n- System dashboard with graphs\n- Plugin system for apps",
                ),
            ],
            sel: 0,
            focus: Focus::List,
            scroll_ed: 0,
            cursor_row: 0,
            cursor_col: 0,
            search: None,
            search_buf: String::new(),
            searching: false,
            adding: false,
            add_buf: String::new(),
            status: String::new(),
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        match &self.search {
            None => (0..self.notes.len()).collect(),
            Some(q) => {
                let ql = q.to_lowercase();
                self.notes.iter().enumerate()
                    .filter(|(_, n)| n.title.to_lowercase().contains(&ql)
                                  || n.body.iter().any(|l| l.to_lowercase().contains(&ql)))
                    .map(|(i, _)| i)
                    .collect()
            }
        }
    }

    fn cur_note(&self) -> Option<&Note> {
        self.notes.get(self.sel)
    }

    fn cur_note_mut(&mut self) -> Option<&mut Note> {
        self.notes.get_mut(self.sel)
    }

    fn clamp_cursor(&mut self) {
        if let Some(n) = self.notes.get(self.sel) {
            if n.body.is_empty() { self.cursor_row = 0; self.cursor_col = 0; return; }
            self.cursor_row = self.cursor_row.min(n.body.len() - 1);
            self.cursor_col = self.cursor_col.min(n.body[self.cursor_row].len());
        }
    }

    fn ensure_body_line(&mut self) {
        if let Some(n) = self.notes.get_mut(self.sel) {
            if n.body.is_empty() { n.body.push(String::new()); }
        }
    }

    fn insert_char(&mut self, c: char) {
        self.ensure_body_line();
        if let Some(n) = self.notes.get_mut(self.sel) {
            let row = self.cursor_row.min(n.body.len().saturating_sub(1));
            let line = &mut n.body[row];
            let col = self.cursor_col.min(line.len());
            line.insert(col, c);
            self.cursor_col = col + 1;
        }
    }

    fn backspace(&mut self) {
        self.ensure_body_line();
        if let Some(n) = self.notes.get_mut(self.sel) {
            let row = self.cursor_row.min(n.body.len().saturating_sub(1));
            if self.cursor_col > 0 {
                let col = self.cursor_col - 1;
                n.body[row].remove(col);
                self.cursor_col = col;
            } else if row > 0 {
                // merge with previous line
                let cur_line = n.body.remove(row);
                let prev_len = n.body[row - 1].len();
                n.body[row - 1].push_str(&cur_line);
                self.cursor_row = row - 1;
                self.cursor_col = prev_len;
            }
        }
    }

    fn newline(&mut self) {
        self.ensure_body_line();
        if let Some(n) = self.notes.get_mut(self.sel) {
            let row = self.cursor_row.min(n.body.len().saturating_sub(1));
            let col = self.cursor_col.min(n.body[row].len());
            let rest = n.body[row].split_off(col);
            n.body.insert(row + 1, rest);
            self.cursor_row = row + 1;
            self.cursor_col = 0;
        }
    }

    fn save(&self) -> Result<(), String> {
        let mut out = String::new();
        for note in &self.notes {
            let body_flat = note.body.join("\\n");
            out.push_str(&format!("{},{}\n", note.title, body_flat));
        }
        std::fs::write("/data/notes.csv", out).map_err(|e| e.to_string())
    }

    fn load(&mut self) -> Result<(), String> {
        let s = std::fs::read_to_string("/data/notes.csv").map_err(|e| e.to_string())?;
        self.notes.clear();
        for line in s.lines() {
            if let Some(comma) = line.find(',') {
                let title = &line[..comma];
                let body_flat = &line[comma + 1..];
                let body: Vec<String> = body_flat.split("\\n").map(|l| l.to_string()).collect();
                self.notes.push(Note { title: title.to_string(), body });
            }
        }
        if self.notes.is_empty() {
            self.notes.push(Note::new("Untitled", ""));
        }
        self.sel = 0;
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.scroll_ed = 0;
        Ok(())
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Notes");
    text(100, 16, C_HINT, "Tab:focus  Ctrl+N:new  Ctrl+D:del  Ctrl+W:save  Ctrl+L:load  Ctrl+F:search");

    let content_h = H - HEADER_H - STATUS_H;

    // Sidebar
    let sidebar_bg = if app.focus == Focus::List { C_CARD } else { C_BG };
    fill(0, HEADER_H, SIDEBAR_W, content_h, sidebar_bg);
    fill(SIDEBAR_W, HEADER_H, 1, content_h, C_BORDER);

    // Search indicator in sidebar header
    let list_label = if let Some(q) = &app.search {
        format!("Notes — search: {}", q)
    } else {
        format!("Notes ({})", app.notes.len())
    };
    let lhdr_col = if app.focus == Focus::List { C_SEL } else { C_HINT };
    text(8, HEADER_H + 8, lhdr_col, &list_label);
    fill(0, HEADER_H + 24, SIDEBAR_W, 1, C_BORDER);

    let list_y = HEADER_H + 26;
    let list_visible = ((content_h - 26) / LINE_H) as usize;
    let indices = app.filtered_indices();

    for (vi, &ni) in indices.iter().enumerate().take(list_visible) {
        let ly = list_y + vi as u32 * LINE_H;
        let is_sel = ni == app.sel;
        if is_sel {
            fill(0, ly, SIDEBAR_W, LINE_H, C_SEL_BG);
        }
        let col = if is_sel { C_TEXT } else { C_HINT };
        let max_t = (SIDEBAR_W - 16) as usize / CHAR_W as usize;
        let n = &app.notes[ni];
        let disp = if n.title.len() > max_t { &n.title[..max_t] } else { &n.title };
        text(8, ly + 2, col, disp);
        if is_sel && app.focus == Focus::List {
            border(0, ly, SIDEBAR_W, LINE_H, C_SEL);
        }
    }

    // Editor panel
    let editor_x = SIDEBAR_W + 1;
    let editor_w = W - SIDEBAR_W - 1;
    fill(editor_x, HEADER_H, editor_w, content_h, C_BG);

    if let Some(note) = app.cur_note() {
        // Note title bar
        fill(editor_x, HEADER_H, editor_w, 28, C_CARD);
        fill(editor_x, HEADER_H + 27, editor_w, 1, C_BORDER);
        let title_col = if app.focus == Focus::Editor { C_SEL } else { C_HINT };
        text(editor_x + 8, HEADER_H + 8, title_col, &note.title);

        // Body lines
        let body_y = HEADER_H + 32;
        let editor_line_h = LINE_H;
        let lines_visible = (content_h - 32) / editor_line_h;
        let max_line_chars = (editor_w - 16) as usize / CHAR_W as usize;

        let scroll = app.scroll_ed;
        for (vi, li) in (scroll..).take(lines_visible as usize).enumerate() {
            if li >= note.body.len() { break; }
            let ly = body_y + vi as u32 * editor_line_h;
            let line = &note.body[li];
            let disp = if line.len() > max_line_chars { &line[..max_line_chars] } else { line };
            text(editor_x + 8, ly + 2, C_TEXT, disp);

            // Cursor
            if app.focus == Focus::Editor && li == app.cursor_row {
                let col = app.cursor_col.min(line.len());
                let cx = editor_x + 8 + col as u32 * CHAR_W;
                fill(cx, ly + 1, 2, editor_line_h - 2, C_SEL);
            }
        }

        // Editor border when focused
        if app.focus == Focus::Editor {
            border(editor_x, HEADER_H, editor_w, content_h, C_SEL);
        }
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);

    if app.adding {
        text(8, sb_y + 8, C_HINT, "New note title: ");
        text(8 + 16 * CHAR_W, sb_y + 8, C_TEXT, &app.add_buf);
        let cx = 8 + (16 + app.add_buf.len() as u32) * CHAR_W;
        fill(cx, sb_y + 6, 2, STATUS_H - 12, C_SEL);
    } else if app.searching {
        text(8, sb_y + 8, C_HINT, "Search: ");
        text(8 + 8 * CHAR_W, sb_y + 8, C_TEXT, &app.search_buf);
        let cx = 8 + (8 + app.search_buf.len() as u32) * CHAR_W;
        fill(cx, sb_y + 6, 2, STATUS_H - 12, C_SEL);
    } else if !app.status.is_empty() {
        text(8, sb_y + 8, C_HINT, &app.status);
    } else {
        let focus_str = if app.focus == Focus::List { "LIST" } else { "EDITOR" };
        let focus_col = if app.focus == Focus::List { C_ORANGE } else { C_SEL };
        text(8, sb_y + 8, focus_col, focus_str);
        if let Some(n) = app.cur_note() {
            let lines = n.body.len();
            let info = format!("  {}  ({} lines)", n.title, lines);
            text(8 + 6 * CHAR_W, sb_y + 8, C_HINT, &info);
        }
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise notes");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        app.status.clear();

        // Search input mode
        if app.searching {
            match raw.as_str() {
                "\x03" | "\x1b" => {
                    app.searching = false;
                    app.search_buf.clear();
                    app.search = None;
                }
                "" => {
                    app.searching = false;
                    if app.search_buf.is_empty() {
                        app.search = None;
                    } else {
                        app.search = Some(app.search_buf.clone());
                    }
                    app.search_buf.clear();
                }
                "\x7f" => { app.search_buf.pop(); }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { app.search_buf.push(b as char); }
                }
                _ => {}
            }
            draw(&app);
            continue;
        }

        // Add-note title input mode
        if app.adding {
            match raw.as_str() {
                "\x03" | "\x1b" => { app.adding = false; app.add_buf.clear(); }
                "\x7f" => { app.add_buf.pop(); }
                "" => {
                    let title = app.add_buf.trim().to_string();
                    if !title.is_empty() {
                        app.notes.push(Note::new(&title, ""));
                        app.sel = app.notes.len() - 1;
                        app.cursor_row = 0;
                        app.cursor_col = 0;
                        app.scroll_ed = 0;
                        app.focus = Focus::Editor;
                        app.status = format!("Created: {}", title);
                    }
                    app.adding = false;
                    app.add_buf.clear();
                }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { app.add_buf.push(b as char); }
                }
                _ => {}
            }
            draw(&app);
            continue;
        }

        // Normal mode
        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\t" => {
                app.focus = if app.focus == Focus::List { Focus::Editor } else { Focus::List };
            }
            "\x0e" => { // Ctrl+N
                app.adding = true;
            }
            "\x04" => { // Ctrl+D
                if !app.notes.is_empty() {
                    let name = app.notes[app.sel].title.clone();
                    app.notes.remove(app.sel);
                    if app.sel >= app.notes.len() && app.sel > 0 { app.sel -= 1; }
                    if app.notes.is_empty() {
                        app.notes.push(Note::new("Untitled", ""));
                        app.sel = 0;
                    }
                    app.cursor_row = 0;
                    app.cursor_col = 0;
                    app.scroll_ed = 0;
                    app.status = format!("Deleted: {}", name);
                }
            }
            "\x17" => { // Ctrl+W
                match app.save() {
                    Ok(()) => app.status = "Saved to /data/notes.csv".to_string(),
                    Err(e) => app.status = format!("Save error: {}", e),
                }
            }
            "\x0c" => { // Ctrl+L
                match app.load() {
                    Ok(()) => app.status = "Loaded from /data/notes.csv".to_string(),
                    Err(e) => app.status = format!("Load error: {}", e),
                }
            }
            "\x06" => { // Ctrl+F
                app.searching = true;
                app.search_buf.clear();
            }
            _ => {
                if app.focus == Focus::List {
                    let indices = app.filtered_indices();
                    let pos = indices.iter().position(|&i| i == app.sel).unwrap_or(0);
                    match raw.as_str() {
                        "\x1b[A" => {
                            if pos > 0 { app.sel = indices[pos - 1]; }
                            else if !indices.is_empty() { app.sel = indices[0]; }
                        }
                        "\x1b[B" => {
                            if pos + 1 < indices.len() { app.sel = indices[pos + 1]; }
                        }
                        "" => { app.focus = Focus::Editor; }
                        _ => {}
                    }
                } else {
                    // Editor mode
                    match raw.as_str() {
                        "\x1b[A" => { // Up
                            if app.cursor_row > 0 {
                                app.cursor_row -= 1;
                                app.clamp_cursor();
                                if app.cursor_row < app.scroll_ed { app.scroll_ed = app.cursor_row; }
                            }
                        }
                        "\x1b[B" => { // Down
                            app.cursor_row += 1;
                            app.clamp_cursor();
                            let editor_h = H - HEADER_H - STATUS_H;
                            let lines_vis = (editor_h - 32) / LINE_H;
                            if app.cursor_row >= app.scroll_ed + lines_vis as usize {
                                app.scroll_ed = app.cursor_row + 1 - lines_vis as usize;
                            }
                        }
                        "\x1b[C" => { // Right
                            if let Some(n) = app.cur_note() {
                                let row = app.cursor_row.min(n.body.len().saturating_sub(1));
                                if app.cursor_col < n.body.get(row).map_or(0, |l| l.len()) {
                                    app.cursor_col += 1;
                                } else if row + 1 < n.body.len() {
                                    app.cursor_row += 1;
                                    app.cursor_col = 0;
                                }
                            }
                        }
                        "\x1b[D" => { // Left
                            if app.cursor_col > 0 {
                                app.cursor_col -= 1;
                            } else if app.cursor_row > 0 {
                                app.cursor_row -= 1;
                                app.cursor_col = app.cur_note()
                                    .and_then(|n| n.body.get(app.cursor_row))
                                    .map_or(0, |l| l.len());
                            }
                        }
                        "\x7f" => { app.backspace(); }
                        "" => { app.newline(); }
                        s => {
                            if s.len() == 1 {
                                let b = s.as_bytes()[0];
                                if b >= 0x20 && b < 0x7f { app.insert_char(b as char); }
                            }
                        }
                    }
                }
            }
        }
        draw(&app);
    }
}
