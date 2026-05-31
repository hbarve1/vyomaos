// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS text editor (P46)
//!
//! A functional text editor with file open/save, cursor movement, and editing.
//! Default file: `/data/scratch.txt`. Accepts `VYOMA_SYSTEM:open:<path>` on stdin.
//! Ctrl+S save, Ctrl+Q quit, Ctrl+G go-to-line.

use std::fs;
use std::io::{BufRead, Write};

// ── Layout ────────────────────────────────────────────────────────────────────

const W: u32 = 1440;
const H: u32 = 900;
const PX: u32 = 32;
const PY: u32 = 32;
const PW: u32 = W - 64;
const PH: u32 = H - 64;
const TITLE_H: u32 = 28;
const STATUS_H: u32 = 20;
const INNER_X: u32 = PX + 14;
const INNER_Y: u32 = PY + TITLE_H + 6;
const INNER_W: u32 = PW - 28;
const INNER_H: u32 = PH - TITLE_H - STATUS_H - 12;
const LINE_H: u32 = 18;
const MAX_VISIBLE: usize = (INNER_H / LINE_H) as usize;
const LNUM_W: u32 = 44;
const TEXT_X: u32 = INNER_X + LNUM_W + 6;
const TEXT_W: u32 = INNER_W - LNUM_W - 6;

// ── Colours ───────────────────────────────────────────────────────────────────

const C_BG: u32 = 0x0D1117FF;
const C_PANEL: u32 = 0x161B22FF;
const C_TITLE: u32 = 0x21262DFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_WHITE: u32 = 0xFFFFFFFF;
const C_DIM: u32 = 0x8B949EFF;
const C_GREEN: u32 = 0x3FB950FF;
const C_YELLOW: u32 = 0xE3B341FF;
const C_CURLINE: u32 = 0x1C2128FF;
const C_GUTTER: u32 = 0x0D1117FF;
const C_SEP: u32 = 0x30363DFF;

// ── Editor state ──────────────────────────────────────────────────────────────

struct Editor {
    lines: Vec<String>,
    cursor_x: usize,
    cursor_y: usize,
    scroll_y: usize,
    file_path: Option<String>,
    modified: bool,
    message: Option<String>,
    prompt: Option<Prompt>,
}

struct Prompt {
    kind: PromptKind,
    buf: String,
}

enum PromptKind {
    GoToLine,
    QuitConfirm,
}

impl Editor {
    fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_x: 0,
            cursor_y: 0,
            scroll_y: 0,
            file_path: None,
            modified: false,
            message: None,
            prompt: None,
        }
    }

    fn open_file(&mut self, path: &str) {
        let content = fs::read_to_string(path).unwrap_or_default();
        self.lines = if content.is_empty() {
            vec![String::new()]
        } else {
            content.lines().map(|s| s.to_string()).collect()
        };
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.file_path = Some(path.to_string());
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.scroll_y = 0;
        self.modified = false;
        self.message = Some(format!("Opened {path}"));
    }

    fn save(&mut self) -> bool {
        if let Some(ref path) = self.file_path {
            let content = self.lines.join("\n");
            match fs::write(path, &content) {
                Ok(()) => {
                    self.modified = false;
                    let bytes = content.len();
                    self.message = Some(format!("Saved {path} ({bytes} bytes)"));
                    true
                }
                Err(e) => {
                    self.message = Some(format!("Save error: {e}"));
                    false
                }
            }
        } else {
            self.message = Some("No file path set".to_string());
            false
        }
    }

    fn clamp_cursor(&mut self) {
        if self.cursor_y >= self.lines.len() {
            self.cursor_y = self.lines.len().saturating_sub(1);
        }
        let line_len = self.lines[self.cursor_y].len();
        if self.cursor_x > line_len {
            self.cursor_x = line_len;
        }
    }

    fn ensure_visible(&mut self) {
        if self.cursor_y < self.scroll_y {
            self.scroll_y = self.cursor_y;
        }
        if self.cursor_y >= self.scroll_y + MAX_VISIBLE {
            self.scroll_y = self.cursor_y.saturating_sub(MAX_VISIBLE - 1);
        }
    }

    fn file_display_name(&self) -> &str {
        self.file_path.as_deref().unwrap_or("[new]")
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let mut ed = Editor::new();

    // Default: open /data/scratch.txt
    ed.open_file("/data/scratch.txt");
    draw(&ed);

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle VYOMA_SYSTEM:open: protocol
        if raw.starts_with("VYOMA_SYSTEM:open:") {
            let path = &raw["VYOMA_SYSTEM:open:".len()..];
            if !path.is_empty() {
                ed.open_file(path);
                draw(&ed);
                continue;
            }
        }

        if handle_input(&raw, &mut ed) {
            break;
        }
        draw(&ed);
    }
}

// ── Input handling — returns true to exit ─────────────────────────────────────

fn handle_input(raw: &str, ed: &mut Editor) -> bool {
    // If a prompt is active, route input there
    if ed.prompt.is_some() {
        return handle_prompt(raw, ed);
    }

    match raw {
        // Ctrl+S — save
        "\x13" => {
            ed.save();
        }
        // Ctrl+Q — quit (warn if modified)
        "\x11" => {
            if ed.modified {
                ed.prompt = Some(Prompt {
                    kind: PromptKind::QuitConfirm,
                    buf: String::new(),
                });
                ed.message = Some("Unsaved changes! Press y to quit, n to cancel".into());
            } else {
                return true;
            }
        }
        // Ctrl+G — go to line
        "\x07" => {
            ed.prompt = Some(Prompt {
                kind: PromptKind::GoToLine,
                buf: String::new(),
            });
            ed.message = Some("Go to line:".to_string());
        }
        // Arrow up
        "\x1b[A" => {
            if ed.cursor_y > 0 {
                ed.cursor_y -= 1;
                ed.clamp_cursor();
                ed.ensure_visible();
            }
        }
        // Arrow down
        "\x1b[B" => {
            if ed.cursor_y + 1 < ed.lines.len() {
                ed.cursor_y += 1;
                ed.clamp_cursor();
                ed.ensure_visible();
            }
        }
        // Arrow right
        "\x1b[C" => {
            let line_len = ed.lines[ed.cursor_y].len();
            if ed.cursor_x < line_len {
                ed.cursor_x += 1;
            } else if ed.cursor_y + 1 < ed.lines.len() {
                ed.cursor_y += 1;
                ed.cursor_x = 0;
                ed.ensure_visible();
            }
        }
        // Arrow left
        "\x1b[D" => {
            if ed.cursor_x > 0 {
                ed.cursor_x -= 1;
            } else if ed.cursor_y > 0 {
                ed.cursor_y -= 1;
                ed.cursor_x = ed.lines[ed.cursor_y].len();
                ed.ensure_visible();
            }
        }
        // Page Up
        "\x1b[5~" => {
            ed.cursor_y = ed.cursor_y.saturating_sub(MAX_VISIBLE);
            ed.clamp_cursor();
            ed.ensure_visible();
        }
        // Page Down
        "\x1b[6~" => {
            ed.cursor_y = (ed.cursor_y + MAX_VISIBLE).min(ed.lines.len() - 1);
            ed.clamp_cursor();
            ed.ensure_visible();
        }
        // Home
        "\x1b[H" => {
            ed.cursor_x = 0;
        }
        // End
        "\x1b[F" => {
            ed.cursor_x = ed.lines[ed.cursor_y].len();
        }
        // Backspace
        "\x7f" => {
            if ed.cursor_x > 0 {
                ed.lines[ed.cursor_y].remove(ed.cursor_x - 1);
                ed.cursor_x -= 1;
                ed.modified = true;
            } else if ed.cursor_y > 0 {
                let tail = ed.lines.remove(ed.cursor_y);
                ed.cursor_y -= 1;
                ed.cursor_x = ed.lines[ed.cursor_y].len();
                ed.lines[ed.cursor_y].push_str(&tail);
                ed.ensure_visible();
                ed.modified = true;
            }
        }
        // Enter — split line
        "" => {
            let tail = ed.lines[ed.cursor_y].split_off(ed.cursor_x);
            ed.cursor_y += 1;
            ed.lines.insert(ed.cursor_y, tail);
            ed.cursor_x = 0;
            ed.ensure_visible();
            ed.modified = true;
        }
        // Delete key
        "\x1b[3~" => {
            let line_len = ed.lines[ed.cursor_y].len();
            if ed.cursor_x < line_len {
                ed.lines[ed.cursor_y].remove(ed.cursor_x);
                ed.modified = true;
            } else if ed.cursor_y + 1 < ed.lines.len() {
                let tail = ed.lines.remove(ed.cursor_y + 1);
                ed.lines[ed.cursor_y].push_str(&tail);
                ed.modified = true;
            }
        }
        // Printable character
        s if single_printable(s) => {
            ed.lines[ed.cursor_y].insert_str(ed.cursor_x, s);
            ed.cursor_x += s.len();
            ed.modified = true;
            ed.message = None;
        }
        _ => {}
    }
    false
}

fn handle_prompt(raw: &str, ed: &mut Editor) -> bool {
    let prompt = ed.prompt.take().unwrap();
    match prompt.kind {
        PromptKind::QuitConfirm => match raw {
            "y" | "Y" => return true,
            _ => {
                ed.message = Some("Quit cancelled".to_string());
            }
        },
        PromptKind::GoToLine => {
            match raw {
                "\x1b" | "\x03" => {
                    ed.message = None;
                }
                "" => {
                    // Enter: apply the line number
                    if let Ok(n) = prompt.buf.trim().parse::<usize>() {
                        let target = n.saturating_sub(1).min(ed.lines.len() - 1);
                        ed.cursor_y = target;
                        ed.cursor_x = 0;
                        ed.clamp_cursor();
                        ed.ensure_visible();
                        ed.message = Some(format!("Jumped to line {}", target + 1));
                    } else {
                        ed.message = Some("Invalid line number".to_string());
                    }
                }
                s if single_printable(s) => {
                    let mut buf = prompt.buf;
                    buf.push_str(s);
                    ed.message = Some(format!("Go to line: {buf}"));
                    ed.prompt = Some(Prompt {
                        kind: PromptKind::GoToLine,
                        buf,
                    });
                }
                "\x7f" => {
                    let mut buf = prompt.buf;
                    buf.pop();
                    ed.message = Some(format!("Go to line: {buf}"));
                    ed.prompt = Some(Prompt {
                        kind: PromptKind::GoToLine,
                        buf,
                    });
                }
                _ => {
                    // Keep prompt alive for unrecognised input
                    ed.prompt = Some(prompt);
                }
            }
        }
    }
    false
}

// ── Drawing ───────────────────────────────────────────────────────────────────

fn draw(ed: &Editor) {
    fill(0, 0, W, H, C_BG);
    fill(PX, PY, PW, PH, C_PANEL);
    draw_editor(ed);
    flush();
}

fn draw_editor(ed: &Editor) {
    let name = ed.file_display_name();
    let accent = if ed.modified { C_YELLOW } else { C_GREEN };

    // Title bar
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, accent);
    let title = if ed.modified {
        format!("* {name}")
    } else {
        name.to_string()
    };
    text(PX + 10, PY + 6, accent, &title);
    border(PX, PY, PW, PH, accent);

    // Gutter + content area
    clear_region(INNER_X, INNER_Y, INNER_W, INNER_H);
    fill(INNER_X, INNER_Y, LNUM_W, INNER_H, C_GUTTER);
    fill(INNER_X + LNUM_W, INNER_Y, 1, INNER_H, C_SEP);

    let visible = ed.lines.iter().skip(ed.scroll_y).take(MAX_VISIBLE);
    for (i, line) in visible.enumerate() {
        let abs_ln = ed.scroll_y + i;
        let iy = INNER_Y + i as u32 * LINE_H;

        // Cursor-line highlight
        if abs_ln == ed.cursor_y {
            fill(INNER_X, iy, INNER_W, LINE_H, C_CURLINE);
        }

        // Line number
        let ln_color = if abs_ln == ed.cursor_y { C_WHITE } else { C_DIM };
        text(INNER_X + 2, iy + 1, ln_color, &format!("{:>4}", abs_ln + 1));

        // Content — sanitise control chars
        let display: String = line
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        text_wrap(TEXT_X, iy + 1, TEXT_W, C_WHITE, &display);

        // Cursor block
        if abs_ln == ed.cursor_y {
            let cx = ed.cursor_x.min(line.len());
            let cursor_px = TEXT_X + (cx as u32) * 8;
            fill(cursor_px, iy + 1, 8, 14, C_ACCENT);
            if cx < line.len() {
                let ch: String = line.chars().nth(cx).into_iter().collect();
                text(cursor_px, iy + 1, C_BG, &ch);
            }
        }
    }

    // Status bar
    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_TITLE);
    fill(PX, sy, PW, 1, C_SEP);

    // Left: message or hints
    let left = if let Some(ref msg) = ed.message {
        msg.clone()
    } else if ed.modified {
        "Ctrl+S save  Ctrl+Q quit  Ctrl+G goto".to_string()
    } else {
        "Ctrl+Q quit  Ctrl+G goto".to_string()
    };
    let msg_color = if ed.message.is_some() { C_ACCENT } else { C_DIM };
    text(INNER_X, sy + 3, msg_color, &left);

    // Right: position + file size
    let total_bytes: usize = ed.lines.iter().map(|l| l.len() + 1).sum();
    let right = format!(
        "Ln {},{} Col {}  {} bytes",
        ed.cursor_y + 1,
        ed.lines.len(),
        ed.cursor_x + 1,
        total_bytes
    );
    let rw = right.len() as u32 * 8;
    let rx = PX + PW - 14 - rw;
    text(rx, sy + 3, C_DIM, &right);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn single_printable(s: &str) -> bool {
    s.len() == 1 && s.bytes().next().map(|b| (0x20..=0x7E).contains(&b)).unwrap_or(false)
}

// ── VYOMA_DRAW helpers ────────────────────────────────────────────────────────

#[inline]
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}
#[inline]
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}
#[inline]
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba}");
}
#[inline]
fn clear_region(x: u32, y: u32, w: u32, h: u32) {
    println!("VYOMA_DRAW:clear_region:{x},{y},{w},{h}");
}
#[inline]
fn text_wrap(x: u32, y: u32, max_w: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text_wrap:{x},{y},{max_w},{rgba},m,{s}");
}
#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}
