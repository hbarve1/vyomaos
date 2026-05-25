// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use super::data::SNIPPETS;

// ── Color constants ───────────────────────────────────────────────────────────

pub const C_BG: u32     = 0x0D1117FF;
pub const C_HEADER: u32 = 0x21262DFF;
pub const C_BORDER: u32 = 0x30363DFF;
pub const C_TEXT: u32   = 0xE6EDF3FF;
pub const C_HINT: u32   = 0x6E7681FF;
pub const C_ORANGE: u32 = 0xFFA657FF;
pub const C_GREEN: u32  = 0x3FB950FF;
pub const C_SEL: u32    = 0x58A6FFFF;
pub const C_YELLOW: u32 = 0xD29922FF;
pub const C_CARD: u32   = 0x161B22FF;

// ── Geometry constants ────────────────────────────────────────────────────────

pub const W: u32 = 1200;
pub const H: u32 = 760;
pub const HEADER_H: u32 = 40;
pub const LIST_W: u32 = 320;
pub const CONTENT_Y: u32 = HEADER_H;
pub const CONTENT_H: u32 = H - HEADER_H - 32;
pub const CODE_X: u32 = LIST_W + 1;
pub const CODE_W: u32 = W - CODE_X;
pub const LINE_H: u32 = 18;
pub const STATUS_H: u32 = 28;

// ── Syntax highlighting ───────────────────────────────────────────────────────

pub fn highlight_line<'a>(line: &'a str, lang: &str) -> Vec<(u32, &'a str)> {
    let trimmed = line.trim_start();

    if lang == "rust" || lang == "shell" {
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            return vec![(C_HINT, line)];
        }
    }
    if lang == "python" && trimmed.starts_with('#') {
        return vec![(C_HINT, line)];
    }
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        return vec![(C_GREEN, line)];
    }

    let rust_keywords = ["fn ", "let ", "use ", "pub ", "struct ", "enum ", "impl ", "match ",
                         "if ", "else", "for ", "while ", "return ", "mod ", "trait ", "type ",
                         "const ", "static ", "mut ", "ref ", "self", "Self", "true", "false"];
    let python_keywords = ["def ", "class ", "import ", "from ", "return ", "if ", "else",
                           "elif ", "for ", "while ", "with ", "as ", "print(", "True", "False"];
    let shell_keywords = ["git ", "docker ", "ssh ", "find ", "grep ", "echo ", "export ",
                          "cd ", "ls ", "mkdir ", "rm ", "cp ", "mv "];

    let keywords: &[&str] = match lang {
        "rust"   => &rust_keywords,
        "python" => &python_keywords,
        "shell"  => &shell_keywords,
        _        => &[],
    };

    for kw in keywords {
        if trimmed.starts_with(kw) {
            return vec![(C_ORANGE, line)];
        }
    }
    vec![(C_TEXT, line)]
}

pub fn lang_color(lang: &str) -> u32 {
    match lang {
        "rust"   => C_ORANGE,
        "python" => C_YELLOW,
        "shell"  => C_GREEN,
        _        => C_HINT,
    }
}

// ── App types ─────────────────────────────────────────────────────────────────

#[derive(PartialEq)]
pub enum Mode { Normal, Search, NewTitle, NewLang, NewCode }

pub struct UserSnippet {
    pub title: String,
    pub lang:  String,
    pub tags:  String,
    pub code:  Vec<String>,
}

pub struct App {
    pub sel:          usize,
    pub scroll:       usize,
    pub code_scroll:  usize,
    pub mode:         Mode,
    pub search_buf:   String,
    pub new_buf:      String,
    pub new_snip:     Option<UserSnippet>,
    pub user_snips:   Vec<UserSnippet>,
    pub status:       String,
}

impl App {
    pub fn new() -> Self {
        App {
            sel: 0,
            scroll: 0,
            code_scroll: 0,
            mode: Mode::Normal,
            search_buf: String::new(),
            new_buf: String::new(),
            new_snip: None,
            user_snips: Vec::new(),
            status: String::from("↑↓=select  C=copy  /=search  Ctrl+N=new  Ctrl+C=quit"),
        }
    }

    pub fn matches(&self, i: usize) -> bool {
        if self.search_buf.is_empty() { return true; }
        let q = self.search_buf.to_lowercase();
        if i < SNIPPETS.len() {
            SNIPPETS[i].title.to_lowercase().contains(&q)
                || SNIPPETS[i].tags.to_lowercase().contains(&q)
                || SNIPPETS[i].lang.to_lowercase().contains(&q)
        } else {
            let u = &self.user_snips[i - SNIPPETS.len()];
            u.title.to_lowercase().contains(&q)
                || u.tags.to_lowercase().contains(&q)
                || u.lang.to_lowercase().contains(&q)
        }
    }

    pub fn visible(&self) -> Vec<usize> {
        let total = SNIPPETS.len() + self.user_snips.len();
        (0..total).filter(|&i| self.matches(i)).collect()
    }
}

// ── Draw ──────────────────────────────────────────────────────────────────────

pub fn draw(app: &App) {
    super::fill(0, 0, W, H, C_BG);

    // Header
    super::fill(0, 0, W, HEADER_H, C_HEADER);
    super::fill(0, HEADER_H - 1, W, 1, C_BORDER);
    super::text(16, 12, C_ORANGE, "Code Snippets");

    if app.mode == Mode::Search {
        super::text(200, 12, C_SEL, &format!("Search: {}_", app.search_buf));
    } else if !app.search_buf.is_empty() {
        super::text(200, 12, C_YELLOW, &format!("Filter: {}  (Esc=clear)", app.search_buf));
    } else {
        super::text(200, 12, C_HINT, "20 snippets · Rust · Python · Shell");
    }

    let vis = app.visible();
    let list_vis = (CONTENT_H / LINE_H) as usize;
    let list_scroll = app.scroll.min(vis.len().saturating_sub(list_vis));

    // Left: snippet list
    super::fill(0, CONTENT_Y, LIST_W, CONTENT_H, C_BG);
    super::fill(0, CONTENT_Y, LIST_W, 1, C_BORDER);

    for (vi, &si) in vis[list_scroll..].iter().take(list_vis).enumerate() {
        let vy = CONTENT_Y + 4 + vi as u32 * LINE_H;
        let is_sel = si == app.sel;

        let (title, lang, _tags) = if si < SNIPPETS.len() {
            (SNIPPETS[si].title, SNIPPETS[si].lang, SNIPPETS[si].tags)
        } else {
            let u = &app.user_snips[si - SNIPPETS.len()];
            (u.title.as_str(), u.lang.as_str(), u.tags.as_str())
        };

        if is_sel {
            super::fill(0, vy - 2, LIST_W, LINE_H, C_CARD);
            super::fill(0, vy - 2, 2, LINE_H, C_SEL);
        }

        let lc = lang_color(lang);
        let badge = &lang[..lang.len().min(2)].to_uppercase();
        let badge_w = badge.len() as u32 * 8 + 4;
        super::fill(4, vy, badge_w, 14, C_HEADER);
        super::text(6, vy, lc, badge);

        let max_title = ((LIST_W - badge_w - 16) / 8) as usize;
        let shown = if title.len() > max_title { &title[..max_title] } else { title };
        let title_col = if is_sel { C_TEXT } else { C_HINT };
        super::text(badge_w + 10, vy, title_col, shown);
    }

    // Divider
    super::fill(LIST_W, CONTENT_Y, 1, CONTENT_H, C_BORDER);

    // Right: code view
    super::fill(CODE_X, CONTENT_Y, CODE_W, CONTENT_H, C_CARD);

    if app.sel < SNIPPETS.len() + app.user_snips.len() {
        let (title, lang, tags, code_lines): (&str, &str, &str, Vec<&str>) = if app.sel < SNIPPETS.len() {
            let s = &SNIPPETS[app.sel];
            (s.title, s.lang, s.tags, s.code.to_vec())
        } else {
            let u = &app.user_snips[app.sel - SNIPPETS.len()];
            (u.title.as_str(), u.lang.as_str(), u.tags.as_str(),
             u.code.iter().map(|s| s.as_str()).collect())
        };

        let lc = lang_color(lang);
        super::text(CODE_X + 8, CONTENT_Y + 6, lc, &format!("[{}] {}", lang.to_uppercase(), title));
        super::text(CODE_X + 8, CONTENT_Y + 24, C_HINT, &format!("tags: {}", tags));
        super::fill(CODE_X, CONTENT_Y + 40, CODE_W, 1, C_BORDER);

        let code_vis = ((CONTENT_H - 50) / LINE_H) as usize;
        let cscroll = app.code_scroll.min(code_lines.len().saturating_sub(code_vis));

        for (li, &line) in code_lines[cscroll..].iter().take(code_vis).enumerate() {
            let ly = CONTENT_Y + 46 + li as u32 * LINE_H;
            super::text(CODE_X + 4, ly, C_HINT, &format!("{:3}", li + 1 + cscroll));
            let spans = highlight_line(line, lang);
            let mut cx = CODE_X + 32;
            for (col, seg) in spans {
                let max_chars = ((CODE_W - 36) / 8) as usize;
                let shown = if seg.len() > max_chars { &seg[..max_chars] } else { seg };
                super::text(cx, ly, col, shown);
                cx += shown.len() as u32 * 8;
            }
        }

        let copy_y = H - STATUS_H - 24;
        super::text(CODE_X + 8, copy_y, C_HINT, "C=copy snippet to clipboard");
    } else {
        super::text(CODE_X + 8, CONTENT_Y + 20, C_HINT, "No snippet selected");
    }

    // New snippet form overlay
    match &app.mode {
        Mode::NewTitle => {
            let oy = H / 2 - 50;
            super::fill(CODE_X + 8, oy, CODE_W - 16, 80, C_HEADER);
            super::border(CODE_X + 8, oy, CODE_W - 16, 80, C_SEL);
            super::text(CODE_X + 16, oy + 10, C_HINT, "New snippet — enter title:");
            super::text(CODE_X + 16, oy + 36, C_TEXT, &format!("{}_", app.new_buf));
        }
        Mode::NewLang => {
            let oy = H / 2 - 50;
            super::fill(CODE_X + 8, oy, CODE_W - 16, 80, C_HEADER);
            super::border(CODE_X + 8, oy, CODE_W - 16, 80, C_SEL);
            super::text(CODE_X + 16, oy + 10, C_HINT, "Language (rust/python/shell):");
            super::text(CODE_X + 16, oy + 36, C_TEXT, &format!("{}_", app.new_buf));
        }
        Mode::NewCode => {
            if let Some(ref ns) = app.new_snip {
                let oy = CONTENT_Y + 50;
                let oh = CONTENT_H - 60;
                super::fill(CODE_X + 8, oy, CODE_W - 16, oh, C_HEADER);
                super::border(CODE_X + 8, oy, CODE_W - 16, oh, C_SEL);
                super::text(CODE_X + 16, oy + 6, C_HINT, &format!("Code for '{}' (Enter=add line, Ctrl+W=save):", ns.title));
                super::fill(CODE_X + 16, oy + 22, CODE_W - 32, 1, C_BORDER);
                for (i, line) in ns.code.iter().enumerate() {
                    super::text(CODE_X + 16, oy + 28 + i as u32 * 16, C_TEXT, line);
                }
                let cur_y = oy + 28 + ns.code.len() as u32 * 16;
                if cur_y < oy + oh - 20 {
                    super::text(CODE_X + 16, cur_y, C_SEL, &format!("{}_", app.new_buf));
                }
            }
        }
        _ => {}
    }

    // Status bar
    let sb_y = H - STATUS_H;
    super::fill(0, sb_y, W, STATUS_H, C_HEADER);
    super::fill(0, sb_y, W, 1, C_BORDER);
    super::text(8, sb_y + 6, C_HINT, &app.status);

    super::flush();
}
