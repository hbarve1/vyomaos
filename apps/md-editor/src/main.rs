use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 800;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 32;
const CHAR_W: u32 = 8;
const LINE_H: u32 = 18;
const PANE_W: u32 = (W - 4) / 2; // 678 each, 4px divider

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_YELLOW: u32 = 0xD29922FF;

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

const EXAMPLE: &str = "\
# VyomaOS — A WASM-First OS

A lightweight, capability-secure operating system built on WebAssembly.

## Architecture

The **supervisor** (Rust PID 1) manages every WASM app:

- App lifecycle and restart policies
- IPC routing between apps
- Display via `VYOMA_DRAW:` protocol
- Keyboard input dispatch

## Display Protocol

Apps write commands to stdout:

```
VYOMA_DRAW:fill_rect:x,y,w,h,rgba
VYOMA_DRAW:draw_text:x,y,rgba,m,text
VYOMA_DRAW:flush
```

### Capabilities

Each app declares what it needs in `vyoma.toml`:

- `stdio` — keyboard input and stdout
- `display` — framebuffer access
- `filesystem` — mount /data
- `network` — TCP sockets

## Status

Over 150 apps complete. Building toward macOS-like desktop.
";

struct Editor {
    lines:      Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
    ed_scroll:  usize,
    pr_scroll:  usize,
    focus_ed:   bool,
    filename:   String,
    opening:    bool,
    open_buf:   String,
    status:     String,
}

impl Editor {
    fn new() -> Self {
        let lines = EXAMPLE.lines().map(|l| l.to_string()).collect();
        Editor {
            lines,
            cursor_row: 0,
            cursor_col: 0,
            ed_scroll: 0,
            pr_scroll: 0,
            focus_ed: true,
            filename: "md-editor.md".to_string(),
            opening: false,
            open_buf: String::new(),
            status: String::new(),
        }
    }

    fn clamp(&mut self) {
        if self.lines.is_empty() { self.lines.push(String::new()); }
        self.cursor_row = self.cursor_row.min(self.lines.len() - 1);
        self.cursor_col = self.cursor_col.min(self.lines[self.cursor_row].len());
    }

    fn insert_char(&mut self, c: char) {
        self.clamp();
        let (r, c_) = (self.cursor_row, self.cursor_col);
        self.lines[r].insert(c_, c);
        self.cursor_col += 1;
    }

    fn backspace(&mut self) {
        self.clamp();
        let (r, c_) = (self.cursor_row, self.cursor_col);
        if c_ > 0 {
            self.lines[r].remove(c_ - 1);
            self.cursor_col -= 1;
        } else if r > 0 {
            let cur = self.lines.remove(r);
            let prev_len = self.lines[r - 1].len();
            self.lines[r - 1].push_str(&cur);
            self.cursor_row -= 1;
            self.cursor_col = prev_len;
        }
    }

    fn newline(&mut self) {
        self.clamp();
        let (r, c_) = (self.cursor_row, self.cursor_col);
        let rest = self.lines[r].split_off(c_);
        self.lines.insert(r + 1, rest);
        self.cursor_row += 1;
        self.cursor_col = 0;
    }

    fn save(&self) -> Result<(), String> {
        let content = self.lines.join("\n");
        let path = format!("/data/{}", self.filename);
        std::fs::write(&path, content).map_err(|e| e.to_string())
    }

    fn load_file(&mut self, fname: &str) -> Result<(), String> {
        let path = format!("/data/{}", fname);
        let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        self.lines = content.lines().map(|l| l.to_string()).collect();
        if self.lines.is_empty() { self.lines.push(String::new()); }
        self.filename = fname.to_string();
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.ed_scroll = 0;
        self.pr_scroll = 0;
        Ok(())
    }
}

fn render_md_line(line: &str, in_code: &mut bool) -> (u32, String) {
    let t = line.trim_end();
    if t.starts_with("```") {
        *in_code = !*in_code;
        return (C_HINT, "─".repeat(38));
    }
    if *in_code {
        return (C_GREEN, format!("  {}", t));
    }
    if let Some(rest) = t.strip_prefix("### ") {
        return (C_YELLOW, format!("   {}", rest));
    }
    if let Some(rest) = t.strip_prefix("## ") {
        return (C_SEL, format!("  {}", rest));
    }
    if let Some(rest) = t.strip_prefix("# ") {
        return (C_ORANGE, rest.to_string());
    }
    if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        return (C_TEXT, format!("  • {}", rest));
    }
    if t.is_empty() {
        return (0x00000000, String::new());
    }
    // Inline bold: strip ** markers
    if t.contains("**") {
        return (C_TEXT, t.replace("**", ""));
    }
    // Inline code: strip backticks
    if t.contains('`') {
        return (C_GREEN, t.replace('`', ""));
    }
    (C_HINT, t.to_string())
}

const CONTENT_H: u32 = H - HEADER_H - STATUS_H; // 720
const ED_LABEL_H: u32 = 20;
const ED_TEXT_Y: u32 = HEADER_H + ED_LABEL_H + 2;
const ED_LINES_H: u32 = CONTENT_H - ED_LABEL_H - 2;

fn draw(ed: &Editor) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Markdown Editor");
    text(240, 16, C_HINT, "Tab:focus  Ctrl+W:save  Ctrl+L:reload  Ctrl+O:open  Ctrl+C:exit");

    let lines_vis = (ED_LINES_H / LINE_H) as usize;
    let ed_max_chars = (PANE_W - 36) as usize / CHAR_W as usize;
    let pr_max_chars = (PANE_W - 16) as usize / CHAR_W as usize;

    // ── Left pane (editor) ──────────────────────────────────────────────────
    let ed_bg = if ed.focus_ed { C_CARD } else { 0x0F1318FF };
    fill(0, HEADER_H, PANE_W, CONTENT_H, ed_bg);
    let lc = if ed.focus_ed { C_SEL } else { C_HINT };
    text(8, HEADER_H + 4, lc, "EDITOR");
    fill(0, HEADER_H + ED_LABEL_H, PANE_W, 1, C_BORDER);

    for (vi, li) in (ed.ed_scroll..).take(lines_vis).enumerate() {
        if li >= ed.lines.len() { break; }
        let ly = ED_TEXT_Y + vi as u32 * LINE_H;
        let line = &ed.lines[li];
        let lnum = format!("{:3}", li + 1);
        text(4, ly, C_HINT, &lnum);
        let disp = if line.len() > ed_max_chars { &line[..ed_max_chars] } else { line };
        text(32, ly, C_TEXT, disp);
        if ed.focus_ed && li == ed.cursor_row {
            let col = ed.cursor_col.min(line.len());
            fill(32 + col as u32 * CHAR_W, ly - 1, 2, LINE_H, C_SEL);
        }
    }
    if ed.focus_ed {
        border(0, HEADER_H, PANE_W, CONTENT_H, C_SEL);
    }

    // ── Divider ─────────────────────────────────────────────────────────────
    fill(PANE_W, HEADER_H, 4, CONTENT_H, C_BORDER);

    // ── Right pane (preview) ────────────────────────────────────────────────
    let pr_x = PANE_W + 4;
    let pr_w = W - pr_x;
    let pr_bg = if !ed.focus_ed { C_CARD } else { 0x0F1318FF };
    fill(pr_x, HEADER_H, pr_w, CONTENT_H, pr_bg);
    let pc = if !ed.focus_ed { C_SEL } else { C_HINT };
    text(pr_x + 8, HEADER_H + 4, pc, "PREVIEW");
    fill(pr_x, HEADER_H + ED_LABEL_H, pr_w, 1, C_BORDER);

    // Build rendered lines
    let mut rendered: Vec<(u32, String)> = Vec::with_capacity(ed.lines.len());
    let mut in_code = false;
    for line in &ed.lines {
        rendered.push(render_md_line(line, &mut in_code));
    }

    for (vi, ri) in (ed.pr_scroll..).take(lines_vis).enumerate() {
        if ri >= rendered.len() { break; }
        let (col, ref txt) = rendered[ri];
        if txt.is_empty() { continue; }
        let ly = ED_TEXT_Y + vi as u32 * LINE_H;
        let disp = if txt.len() > pr_max_chars { &txt[..pr_max_chars] } else { txt };
        text(pr_x + 8, ly, col, disp);
        // Underline H1
        if col == C_ORANGE {
            fill(pr_x + 8, ly + LINE_H, disp.len() as u32 * CHAR_W, 1, C_ORANGE);
        }
    }
    if !ed.focus_ed {
        border(pr_x, HEADER_H, pr_w, CONTENT_H, C_SEL);
    }

    // ── Status bar ──────────────────────────────────────────────────────────
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);

    if ed.opening {
        text(8, sb_y + 8, C_HINT, "Open /data/: ");
        text(8 + 13 * CHAR_W, sb_y + 8, C_TEXT, &ed.open_buf);
        let cx = 8 + (13 + ed.open_buf.len() as u32) * CHAR_W;
        fill(cx, sb_y + 6, 2, STATUS_H - 12, C_SEL);
    } else if !ed.status.is_empty() {
        text(8, sb_y + 8, C_HINT, &ed.status);
    } else {
        let fl = if ed.focus_ed { "EDITOR" } else { "PREVIEW" };
        let fc = if ed.focus_ed { C_SEL } else { C_YELLOW };
        text(8, sb_y + 8, fc, fl);
        let info = format!("  {}  {}:{}", ed.filename, ed.cursor_row + 1, ed.cursor_col + 1);
        text(8 + 8 * CHAR_W, sb_y + 8, C_HINT, &info);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut ed = Editor::new();

    println!("@supervisor: raise md-editor");
    let _ = io::stdout().flush();
    draw(&ed);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        ed.status.clear();

        // Open-file input mode
        if ed.opening {
            match raw.as_str() {
                "\x03" | "\x1b" => { ed.opening = false; ed.open_buf.clear(); }
                "\x7f" => { ed.open_buf.pop(); }
                "" => {
                    let fname = ed.open_buf.trim().to_string();
                    ed.opening = false;
                    ed.open_buf.clear();
                    if !fname.is_empty() {
                        match ed.load_file(&fname) {
                            Ok(()) => ed.status = format!("Loaded: {}", fname),
                            Err(e) => ed.status = format!("Error: {}", e),
                        }
                    }
                }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { ed.open_buf.push(b as char); }
                }
                _ => {}
            }
            draw(&ed);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\t" => { ed.focus_ed = !ed.focus_ed; }
            "\x17" => { // Ctrl+W
                match ed.save() {
                    Ok(()) => ed.status = format!("Saved /data/{}", ed.filename),
                    Err(e) => ed.status = format!("Save error: {}", e),
                }
            }
            "\x0c" => { // Ctrl+L
                let fname = ed.filename.clone();
                match ed.load_file(&fname) {
                    Ok(()) => ed.status = format!("Loaded /data/{}", ed.filename),
                    Err(e) => ed.status = format!("Load error: {}", e),
                }
            }
            "\x0f" => { ed.opening = true; } // Ctrl+O
            _ => {
                if ed.focus_ed {
                    let vis = (ED_LINES_H / LINE_H) as usize;
                    match raw.as_str() {
                        "\x1b[A" => {
                            if ed.cursor_row > 0 {
                                ed.cursor_row -= 1;
                                ed.clamp();
                                if ed.cursor_row < ed.ed_scroll { ed.ed_scroll = ed.cursor_row; }
                            }
                        }
                        "\x1b[B" => {
                            if ed.cursor_row + 1 < ed.lines.len() {
                                ed.cursor_row += 1;
                                ed.clamp();
                                if ed.cursor_row >= ed.ed_scroll + vis {
                                    ed.ed_scroll = ed.cursor_row + 1 - vis;
                                }
                            }
                        }
                        "\x1b[C" => {
                            ed.clamp();
                            let (r, c_) = (ed.cursor_row, ed.cursor_col);
                            if c_ < ed.lines[r].len() {
                                ed.cursor_col += 1;
                            } else if r + 1 < ed.lines.len() {
                                ed.cursor_row += 1;
                                ed.cursor_col = 0;
                            }
                        }
                        "\x1b[D" => {
                            if ed.cursor_col > 0 {
                                ed.cursor_col -= 1;
                            } else if ed.cursor_row > 0 {
                                ed.cursor_row -= 1;
                                ed.clamp();
                                ed.cursor_col = ed.lines[ed.cursor_row].len();
                            }
                        }
                        "\x1b[5~" => { ed.ed_scroll = ed.ed_scroll.saturating_sub(vis); }
                        "\x1b[6~" => {
                            let max_scroll = ed.lines.len().saturating_sub(vis);
                            ed.ed_scroll = (ed.ed_scroll + vis).min(max_scroll);
                        }
                        "\x7f" => { ed.backspace(); }
                        "" => { ed.newline(); }
                        s => {
                            if s.len() == 1 {
                                let b = s.as_bytes()[0];
                                if b >= 0x20 && b < 0x7f { ed.insert_char(b as char); }
                            }
                        }
                    }
                } else {
                    let vis = (ED_LINES_H / LINE_H) as usize;
                    match raw.as_str() {
                        "\x1b[A" | "\x1b[5~" => {
                            ed.pr_scroll = ed.pr_scroll.saturating_sub(1);
                        }
                        "\x1b[B" | "\x1b[6~" => {
                            let max_pr = ed.lines.len().saturating_sub(vis);
                            ed.pr_scroll = (ed.pr_scroll + 1).min(max_pr);
                        }
                        _ => {}
                    }
                }
            }
        }
        draw(&ed);
    }
}
