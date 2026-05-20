//! VyomaOS file manager (P24)
//!
//! Two-pane state machine:
//!   List view  — scrollable /data directory; ↑/↓ navigate, Enter open, q quit
//!   File view  — first 100 lines of selected file; ↑/↓ scroll, q/Backspace back
//!
//! Launched on demand: `run file-manager` from the shell.
//! Window region covers the full screen (1440×900); declared in vyoma.toml.

use std::fs;
use std::io::{BufRead, Write};
use std::path::Path;

// ── Screen / panel geometry ──────────────────────────────────────────────────

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
// content area height: panel minus title bar, status bar and margins
const INNER_H: u32 = PH - TITLE_H - STATUS_H - 12;
const ITEM_H: u32 = 18;
const MAX_VISIBLE: usize = (INNER_H / ITEM_H) as usize;

const SCROLLBAR_W: u32 = 6;
const LIST_W: u32 = INNER_W - SCROLLBAR_W - 4;

// ── Colours ───────────────────────────────────────────────────────────────────

const C_BG: u32 = 0x0D1117FF;
const C_PANEL: u32 = 0x161B22FF;
const C_TITLE: u32 = 0x21262DFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_WHITE: u32 = 0xFFFFFFFF;
const C_DIM: u32 = 0x8B949EFF;
const C_SELECT: u32 = 0x1F3A5FFF;
const C_GREEN: u32 = 0x3FB950FF;
const C_YELLOW: u32 = 0xE3B341FF;
const C_RED: u32 = 0xF85149FF;

// ── Data model ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Entry {
    name: String,
    is_dir: bool,
    size: u64,
}

enum View {
    List {
        entries: Vec<Entry>,
        selected: usize,
        scroll: usize,
    },
    File {
        name: String,
        lines: Vec<String>,
        scroll: usize,
    },
    Error {
        message: String,
    },
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    let data_path = Path::new("/data");
    let mut view = make_list_view(data_path);
    draw(&view);

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw {
            Ok(l) => l,
            Err(_) => break,
        };
        if handle_input(raw, &mut view, data_path) {
            break;
        }
        draw(&view);
    }
}

// ── Input handler — returns true when the app should exit ─────────────────────

fn handle_input(raw: String, view: &mut View, data_path: &Path) -> bool {
    match view {
        View::List { entries, selected, scroll } => match raw.as_str() {
            "q" | "\x03" => return true,

            "\x1b[A" => {
                // Up arrow
                if *selected > 0 {
                    *selected -= 1;
                    if *selected < *scroll {
                        *scroll = *selected;
                    }
                }
            }
            "\x1b[B" => {
                // Down arrow
                if !entries.is_empty() && *selected + 1 < entries.len() {
                    *selected += 1;
                    if *selected >= *scroll + MAX_VISIBLE {
                        *scroll = selected.saturating_sub(MAX_VISIBLE - 1);
                    }
                }
            }
            "" => {
                // Enter — open selected entry
                if let Some(entry) = entries.get(*selected).cloned() {
                    if !entry.is_dir {
                        let path = data_path.join(&entry.name);
                        *view = open_file(&entry.name, &path);
                    }
                }
            }
            _ => {}
        },

        View::File { lines, scroll, .. } => match raw.as_str() {
            "q" | "\x7f" | "\x03" => {
                *view = make_list_view(data_path);
            }
            "\x1b[A" => {
                if *scroll > 0 {
                    *scroll -= 1;
                }
            }
            "\x1b[B" => {
                if *scroll + MAX_VISIBLE < lines.len() {
                    *scroll += 1;
                }
            }
            _ => {}
        },

        View::Error { .. } => match raw.as_str() {
            "q" | "\x03" | "\x7f" => return true,
            _ => {}
        },
    }
    false
}

// ── State builders ────────────────────────────────────────────────────────────

fn make_list_view(data_path: &Path) -> View {
    View::List {
        entries: read_dir(data_path),
        selected: 0,
        scroll: 0,
    }
}

fn read_dir(path: &Path) -> Vec<Entry> {
    let rd = match fs::read_dir(path) {
        Ok(r) => r,
        Err(e) => {
            return vec![Entry {
                name: format!("(error: {e})"),
                is_dir: false,
                size: 0,
            }]
        }
    };
    let mut entries: Vec<Entry> = rd
        .filter_map(|e| e.ok())
        .map(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            let meta = e.metadata().ok();
            Entry {
                name,
                is_dir: meta.as_ref().map(|m| m.is_dir()).unwrap_or(false),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
            }
        })
        .collect();
    // Directories first, then alphabetical
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    entries
}

fn open_file(name: &str, path: &Path) -> View {
    let lines = match fs::read_to_string(path) {
        Ok(content) => content
            .lines()
            .take(200)
            .map(|s| s.to_string())
            .collect(),
        Err(e) => vec![format!("Cannot read file: {e}")],
    };
    View::File {
        name: name.to_string(),
        lines,
        scroll: 0,
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────

fn draw(view: &View) {
    fill(0, 0, W, H, C_BG);
    fill(PX, PY, PW, PH, C_PANEL);

    match view {
        View::List { entries, selected, scroll } => {
            draw_list_title(entries.len());
            draw_list_body(entries, *selected, *scroll);
            draw_status("↑/↓ navigate   Enter open   q quit");
        }
        View::File { name, lines, scroll } => {
            draw_file_title(name, lines.len());
            draw_file_body(lines, *scroll);
            draw_status("↑/↓ scroll   Backspace/q back to list");
        }
        View::Error { message } => {
            draw_error_title();
            text(INNER_X + 8, INNER_Y + 8, C_RED, message);
            draw_status("q quit");
        }
    }

    flush();
}

fn draw_list_title(count: usize) {
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_ACCENT);
    text(PX + 10, PY + 6, C_ACCENT, "File Manager");
    text(
        PX + 130,
        PY + 6,
        C_DIM,
        &format!("/data  ({count} items)"),
    );
    border(PX, PY, PW, PH, C_ACCENT);
}

fn draw_file_title(name: &str, line_count: usize) {
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_GREEN);
    text(PX + 10, PY + 6, C_GREEN, name);
    text(
        PX + 10 + name.len() as u32 * 8 + 12,
        PY + 6,
        C_DIM,
        &format!("({line_count} lines shown)"),
    );
    border(PX, PY, PW, PH, C_GREEN);
}

fn draw_error_title() {
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_RED);
    text(PX + 10, PY + 6, C_RED, "File Manager — Error");
    border(PX, PY, PW, PH, C_RED);
}

fn draw_status(hint: &str) {
    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_TITLE);
    fill(PX, sy, PW, 1, 0x30363DFF);
    text(INNER_X, sy + 3, C_DIM, hint);
}

fn draw_list_body(entries: &[Entry], selected: usize, scroll: usize) {
    clear_region(INNER_X, INNER_Y, INNER_W, INNER_H);

    if entries.is_empty() {
        let cx = INNER_X + INNER_W / 2 - 80;
        let cy = INNER_Y + INNER_H / 2 - 8;
        text(cx, cy, C_DIM, "(empty — /data has no files)");
        return;
    }

    let visible = entries.iter().skip(scroll).take(MAX_VISIBLE).enumerate();
    for (i, entry) in visible {
        let abs_idx = scroll + i;
        let iy = INNER_Y + i as u32 * ITEM_H;

        if abs_idx == selected {
            fill(INNER_X, iy, LIST_W, ITEM_H, C_SELECT);
        }

        let (color, tag) = if entry.is_dir {
            (C_YELLOW, "DIR")
        } else {
            (C_WHITE, "   ")
        };

        // Tag column (DIR / blank)
        text(INNER_X + 4, iy + 1, color, tag);

        // File name — truncate if too long for available width
        let name_x = INNER_X + 36;
        let max_chars = (LIST_W.saturating_sub(200) / 8) as usize;
        let display_name = if entry.name.len() > max_chars {
            format!("{}…", &entry.name[..max_chars.saturating_sub(1)])
        } else {
            entry.name.clone()
        };
        text(name_x, iy + 1, color, &display_name);

        // Size column (right-aligned)
        if !entry.is_dir {
            let size_str = format_size(entry.size);
            let sx = INNER_X + LIST_W - 88;
            text(sx, iy + 1, C_DIM, &size_str);
        }
    }

    // Scrollbar
    if entries.len() > MAX_VISIBLE {
        let bar_x = INNER_X + INNER_W - SCROLLBAR_W;
        fill(bar_x, INNER_Y, SCROLLBAR_W, INNER_H, C_TITLE);
        let max_scroll = entries.len() - MAX_VISIBLE;
        let frac = scroll as f32 / max_scroll as f32;
        let thumb_h =
            ((INNER_H as f32 * MAX_VISIBLE as f32 / entries.len() as f32) as u32).max(12);
        let thumb_y = INNER_Y + (frac * (INNER_H - thumb_h) as f32) as u32;
        fill(bar_x, thumb_y, SCROLLBAR_W, thumb_h, C_ACCENT);
    }
}

fn draw_file_body(lines: &[String], scroll: usize) {
    clear_region(INNER_X, INNER_Y, INNER_W, INNER_H);

    let line_no_w: u32 = 40;
    let content_x = INNER_X + line_no_w + 4;
    let content_w = INNER_W - line_no_w - 4;

    let visible = lines.iter().skip(scroll).take(MAX_VISIBLE).enumerate();
    for (i, line) in visible {
        let ln = scroll + i + 1;
        let iy = INNER_Y + i as u32 * ITEM_H;
        text(INNER_X + 2, iy + 1, C_DIM, &format!("{ln:4}"));
        // Draw a thin separator between line numbers and content
        fill(INNER_X + line_no_w, iy, 1, ITEM_H, 0x30363DFF);
        text_wrap(content_x, iy + 1, content_w, C_WHITE, line);
    }

    if lines.is_empty() {
        text(INNER_X + 8, INNER_Y + 8, C_DIM, "(empty file)");
    }

    // Scrollbar
    if lines.len() > MAX_VISIBLE {
        let bar_x = INNER_X + INNER_W - SCROLLBAR_W;
        fill(bar_x, INNER_Y, SCROLLBAR_W, INNER_H, C_TITLE);
        let max_scroll = lines.len() - MAX_VISIBLE;
        let frac = scroll as f32 / max_scroll as f32;
        let thumb_h =
            ((INNER_H as f32 * MAX_VISIBLE as f32 / lines.len() as f32) as u32).max(12);
        let thumb_y = INNER_Y + (frac * (INNER_H - thumb_h) as f32) as u32;
        fill(bar_x, thumb_y, SCROLLBAR_W, thumb_h, C_GREEN);
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn format_size(bytes: u64) -> String {
    if bytes < 1_024 {
        format!("{bytes:>6} B")
    } else if bytes < 1_024 * 1_024 {
        format!("{:>5.1} KB", bytes as f64 / 1_024.0)
    } else if bytes < 1_024 * 1_024 * 1_024 {
        format!("{:>5.1} MB", bytes as f64 / (1_024.0 * 1_024.0))
    } else {
        format!("{:>5.1} GB", bytes as f64 / (1_024.0 * 1_024.0 * 1_024.0))
    }
}

// ── VYOMA_DRAW protocol helpers ───────────────────────────────────────────────

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
