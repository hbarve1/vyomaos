// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use super::{
    C_BG, C_HEADER, C_BORDER, C_TEXT, C_HINT, C_ORANGE, C_SEL, C_CARD, C_SEL_BG,
    W, H, HEADER_H, STATUS_H, SIDEBAR_W, CHAR_W, LINE_H,
};

#[derive(Clone)]
pub struct Note {
    pub title: String,
    pub body:  Vec<String>, // lines
}

impl Note {
    pub fn new(title: &str, body: &str) -> Self {
        Note {
            title: title.to_string(),
            body: body.lines().map(|l| l.to_string()).collect(),
        }
    }
}

#[derive(PartialEq)]
pub enum Focus { List, Editor }

pub struct App {
    pub notes:      Vec<Note>,
    pub sel:        usize,
    pub focus:      Focus,
    pub scroll_ed:  usize, // editor scroll (line index)
    pub cursor_row: usize, // editor cursor row within note body
    pub cursor_col: usize, // editor cursor col within current line
    pub search:     Option<String>, // active search filter
    pub search_buf: String,
    pub searching:  bool,
    pub adding:     bool,
    pub add_buf:    String,
    pub status:     String,
}

impl App {
    pub fn new() -> Self {
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

    pub fn filtered_indices(&self) -> Vec<usize> {
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

    pub fn cur_note(&self) -> Option<&Note> {
        self.notes.get(self.sel)
    }

    pub fn cur_note_mut(&mut self) -> Option<&mut Note> {
        self.notes.get_mut(self.sel)
    }

    pub fn clamp_cursor(&mut self) {
        if let Some(n) = self.notes.get(self.sel) {
            if n.body.is_empty() { self.cursor_row = 0; self.cursor_col = 0; return; }
            self.cursor_row = self.cursor_row.min(n.body.len() - 1);
            self.cursor_col = self.cursor_col.min(n.body[self.cursor_row].len());
        }
    }

    pub fn ensure_body_line(&mut self) {
        if let Some(n) = self.notes.get_mut(self.sel) {
            if n.body.is_empty() { n.body.push(String::new()); }
        }
    }

    pub fn insert_char(&mut self, c: char) {
        self.ensure_body_line();
        if let Some(n) = self.notes.get_mut(self.sel) {
            let row = self.cursor_row.min(n.body.len().saturating_sub(1));
            let line = &mut n.body[row];
            let col = self.cursor_col.min(line.len());
            line.insert(col, c);
            self.cursor_col = col + 1;
        }
    }

    pub fn backspace(&mut self) {
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

    pub fn newline(&mut self) {
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

    pub fn save(&self) -> Result<(), String> {
        let mut out = String::new();
        for note in &self.notes {
            let body_flat = note.body.join("\\n");
            out.push_str(&format!("{},{}\n", note.title, body_flat));
        }
        std::fs::write("/data/notes.csv", out).map_err(|e| e.to_string())
    }

    pub fn load(&mut self) -> Result<(), String> {
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

pub fn draw(app: &App) {
    super::fill(0, 0, W, H, C_BG);

    // Header
    super::fill(0, 0, W, HEADER_H, C_HEADER);
    super::fill(0, HEADER_H - 1, W, 1, C_BORDER);
    super::text(16, 16, C_ORANGE, "Notes");
    super::text(100, 16, C_HINT, "Tab:focus  Ctrl+N:new  Ctrl+D:del  Ctrl+W:save  Ctrl+L:load  Ctrl+F:search");

    let content_h = H - HEADER_H - STATUS_H;

    // Sidebar
    let sidebar_bg = if app.focus == Focus::List { C_CARD } else { C_BG };
    super::fill(0, HEADER_H, SIDEBAR_W, content_h, sidebar_bg);
    super::fill(SIDEBAR_W, HEADER_H, 1, content_h, C_BORDER);

    // Search indicator in sidebar header
    let list_label = if let Some(q) = &app.search {
        format!("Notes — search: {}", q)
    } else {
        format!("Notes ({})", app.notes.len())
    };
    let lhdr_col = if app.focus == Focus::List { C_SEL } else { C_HINT };
    super::text(8, HEADER_H + 8, lhdr_col, &list_label);
    super::fill(0, HEADER_H + 24, SIDEBAR_W, 1, C_BORDER);

    let list_y = HEADER_H + 26;
    let list_visible = ((content_h - 26) / LINE_H) as usize;
    let indices = app.filtered_indices();

    for (vi, &ni) in indices.iter().enumerate().take(list_visible) {
        let ly = list_y + vi as u32 * LINE_H;
        let is_sel = ni == app.sel;
        if is_sel {
            super::fill(0, ly, SIDEBAR_W, LINE_H, C_SEL_BG);
        }
        let col = if is_sel { C_TEXT } else { C_HINT };
        let max_t = (SIDEBAR_W - 16) as usize / CHAR_W as usize;
        let n = &app.notes[ni];
        let disp = if n.title.len() > max_t { &n.title[..max_t] } else { &n.title };
        super::text(8, ly + 2, col, disp);
        if is_sel && app.focus == Focus::List {
            super::border(0, ly, SIDEBAR_W, LINE_H, C_SEL);
        }
    }

    // Editor panel
    let editor_x = SIDEBAR_W + 1;
    let editor_w = W - SIDEBAR_W - 1;
    super::fill(editor_x, HEADER_H, editor_w, content_h, C_BG);

    if let Some(note) = app.cur_note() {
        // Note title bar
        super::fill(editor_x, HEADER_H, editor_w, 28, C_CARD);
        super::fill(editor_x, HEADER_H + 27, editor_w, 1, C_BORDER);
        let title_col = if app.focus == Focus::Editor { C_SEL } else { C_HINT };
        super::text(editor_x + 8, HEADER_H + 8, title_col, &note.title);

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
            super::text(editor_x + 8, ly + 2, C_TEXT, disp);

            // Cursor
            if app.focus == Focus::Editor && li == app.cursor_row {
                let col = app.cursor_col.min(line.len());
                let cx = editor_x + 8 + col as u32 * CHAR_W;
                super::fill(cx, ly + 1, 2, editor_line_h - 2, C_SEL);
            }
        }

        // Editor border when focused
        if app.focus == Focus::Editor {
            super::border(editor_x, HEADER_H, editor_w, content_h, C_SEL);
        }
    }

    // Status bar
    let sb_y = H - STATUS_H;
    super::fill(0, sb_y, W, STATUS_H, C_HEADER);
    super::fill(0, sb_y, W, 1, C_BORDER);

    if app.adding {
        super::text(8, sb_y + 8, C_HINT, "New note title: ");
        super::text(8 + 16 * CHAR_W, sb_y + 8, C_TEXT, &app.add_buf);
        let cx = 8 + (16 + app.add_buf.len() as u32) * CHAR_W;
        super::fill(cx, sb_y + 6, 2, STATUS_H - 12, C_SEL);
    } else if app.searching {
        super::text(8, sb_y + 8, C_HINT, "Search: ");
        super::text(8 + 8 * CHAR_W, sb_y + 8, C_TEXT, &app.search_buf);
        let cx = 8 + (8 + app.search_buf.len() as u32) * CHAR_W;
        super::fill(cx, sb_y + 6, 2, STATUS_H - 12, C_SEL);
    } else if !app.status.is_empty() {
        super::text(8, sb_y + 8, C_HINT, &app.status);
    } else {
        let focus_str = if app.focus == Focus::List { "LIST" } else { "EDITOR" };
        let focus_col = if app.focus == Focus::List { C_ORANGE } else { C_SEL };
        super::text(8, sb_y + 8, focus_col, focus_str);
        if let Some(n) = app.cur_note() {
            let line_count = n.body.len();
            let word_count: usize = n.body.iter()
                .map(|l| l.split_whitespace().count())
                .sum();
            let info = format!("  {}  |  Words: {}  |  Lines: {}", n.title, word_count, line_count);
            super::text(8 + 6 * CHAR_W, sb_y + 8, C_HINT, &info);
        }
    }

    super::flush();
}
