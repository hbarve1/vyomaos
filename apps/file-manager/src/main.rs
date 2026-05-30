// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS file manager (P43)
//!
//! Functional file browser for /data with navigation, preview, and file ops.
//!   List view  — scrollable directory listing; arrows navigate, Enter opens
//!   File view  — first 20 lines of selected file; arrows scroll, Backspace back
//!   Input mode — text prompt for new-file / rename operations
//!   Confirm    — y/n confirmation for delete

use std::fs;
use std::io::{BufRead, Write};
use std::path::PathBuf;

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
const INNER_H: u32 = PH - TITLE_H - STATUS_H - 12;
const ITEM_H: u32 = 18;
const MAX_VISIBLE: usize = (INNER_H / ITEM_H) as usize;
const SCROLLBAR_W: u32 = 6;
const LIST_W: u32 = INNER_W - SCROLLBAR_W - 4;

// ── Colours ─────────────────────────────────────────────────────────────────

const C_BG: u32 = 0x0D1117FF;
const C_PANEL: u32 = 0x161B22FF;
const C_TITLE: u32 = 0x21262DFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_WHITE: u32 = 0xFFFFFFFF;
const C_DIM: u32 = 0x8B949EFF;
const C_SELECT: u32 = 0x1F3A5FFF;
const C_GREEN: u32 = 0x3FB950FF;
const C_YELLOW: u32 = 0xE3B341FF;

// ── Data model ──────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Entry {
    name: String,
    is_dir: bool,
    size: u64,
}

enum InputMode {
    None,
    NewFile { buf: String },
    Rename { old_name: String, buf: String },
    ConfirmDelete { name: String, is_dir: bool },
}

enum View {
    List { entries: Vec<Entry>, selected: usize, scroll: usize },
    File { name: String, lines: Vec<String>, scroll: usize },
}

struct State { cwd: PathBuf, view: View, input: InputMode, status_msg: Option<String> }

// ── Entry point ─────────────────────────────────────────────────────────────

fn main() {
    let root = PathBuf::from("/data");
    let mut st = State { view: make_list_view(&root), cwd: root,
        input: InputMode::None, status_msg: None };
    draw(&st);
    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let line = match raw { Ok(l) => l, Err(_) => break };
        if handle_input(&line, &mut st) { break; }
        draw(&st);
    }
}

// ── Input handling ──────────────────────────────────────────────────────────

fn handle_input(raw: &str, st: &mut State) -> bool {
    // If in an input mode, route there first
    if !matches!(st.input, InputMode::None) {
        return handle_input_mode(raw, st);
    }

    match &mut st.view {
        View::List { entries, selected, scroll } => match raw {
            "q" | "\x03" => return true,
            "\x1b[A" => {
                if *selected > 0 {
                    *selected -= 1;
                    if *selected < *scroll { *scroll = *selected; }
                }
            }
            "\x1b[B" => {
                if !entries.is_empty() && *selected + 1 < entries.len() {
                    *selected += 1;
                    if *selected >= *scroll + MAX_VISIBLE {
                        *scroll = selected.saturating_sub(MAX_VISIBLE - 1);
                    }
                }
            }
            "" => {
                // Enter: open file or navigate into directory
                if let Some(e) = entries.get(*selected).cloned() {
                    if e.is_dir {
                        st.cwd.push(&e.name);
                        st.view = make_list_view(&st.cwd);
                    } else {
                        let path = st.cwd.join(&e.name);
                        st.view = open_file(&e.name, &path);
                    }
                }
            }
            "\x7f" | "\x1b[D" => {
                // Backspace or Left arrow: go up (stop at /data)
                go_up(st);
            }
            "n" => {
                st.input = InputMode::NewFile { buf: String::new() };
                st.status_msg = Some("New file name (Enter to create, Esc to cancel):".into());
            }
            "d" => {
                if let Some(e) = entries.get(*selected).cloned() {
                    st.input = InputMode::ConfirmDelete {
                        name: e.name.clone(),
                        is_dir: e.is_dir,
                    };
                    let kind = if e.is_dir { "directory" } else { "file" };
                    st.status_msg =
                        Some(format!("Delete {kind} '{}'? (y/n)", e.name));
                }
            }
            "r" => {
                if let Some(e) = entries.get(*selected).cloned() {
                    st.input = InputMode::Rename {
                        old_name: e.name.clone(),
                        buf: String::new(),
                    };
                    st.status_msg =
                        Some(format!("Rename '{}' to (Enter to confirm, Esc cancel):", e.name));
                }
            }
            _ => {}
        },
        View::File { lines, scroll, .. } => match raw {
            "q" | "\x7f" | "\x03" => {
                st.view = make_list_view(&st.cwd);
            }
            "\x1b[A" => { if *scroll > 0 { *scroll -= 1; } }
            "\x1b[B" => { if *scroll + MAX_VISIBLE < lines.len() { *scroll += 1; } }
            _ => {}
        },
    }
    false
}

fn handle_input_mode(raw: &str, st: &mut State) -> bool {
    match &mut st.input {
        InputMode::NewFile { buf } => match raw {
            "\x1b" => { st.input = InputMode::None; st.status_msg = None; }
            "" => {
                let name = buf.clone();
                st.input = InputMode::None;
                if !name.is_empty() {
                    let path = st.cwd.join(&name);
                    match fs::File::create(&path) {
                        Ok(_) => st.status_msg = Some(format!("Created '{name}'")),
                        Err(e) => st.status_msg = Some(format!("Error: {e}")),
                    }
                    st.view = make_list_view(&st.cwd);
                } else {
                    st.status_msg = None;
                }
            }
            "\x7f" => { buf.pop(); }
            _ => buf.push_str(raw),
        },
        InputMode::Rename { old_name, buf } => match raw {
            "\x1b" => { st.input = InputMode::None; st.status_msg = None; }
            "" => {
                let new_name = buf.clone();
                let old = old_name.clone();
                st.input = InputMode::None;
                if !new_name.is_empty() {
                    let from = st.cwd.join(&old);
                    let to = st.cwd.join(&new_name);
                    match fs::rename(&from, &to) {
                        Ok(_) => st.status_msg = Some(format!("Renamed '{old}' -> '{new_name}'")),
                        Err(e) => st.status_msg = Some(format!("Error: {e}")),
                    }
                    st.view = make_list_view(&st.cwd);
                } else {
                    st.status_msg = None;
                }
            }
            "\x7f" => { buf.pop(); }
            _ => buf.push_str(raw),
        },
        InputMode::ConfirmDelete { name, is_dir } => {
            let n = name.clone();
            let dir = *is_dir;
            st.input = InputMode::None;
            if raw == "y" {
                let path = st.cwd.join(&n);
                let res = if dir {
                    fs::remove_dir_all(&path)
                } else {
                    fs::remove_file(&path)
                };
                match res {
                    Ok(_) => st.status_msg = Some(format!("Deleted '{n}'")),
                    Err(e) => st.status_msg = Some(format!("Error: {e}")),
                }
                st.view = make_list_view(&st.cwd);
            } else {
                st.status_msg = Some("Delete cancelled".into());
            }
        }
        InputMode::None => {}
    }
    false
}

fn go_up(st: &mut State) {
    let root = PathBuf::from("/data");
    if st.cwd != root {
        st.cwd.pop();
        st.view = make_list_view(&st.cwd);
    }
}

// ── State builders ──────────────────────────────────────────────────────────

fn make_list_view(dir: &PathBuf) -> View {
    View::List { entries: read_dir(dir), selected: 0, scroll: 0 }
}

fn read_dir(path: &PathBuf) -> Vec<Entry> {
    let rd = match fs::read_dir(path) {
        Ok(r) => r,
        Err(e) => {
            return vec![Entry { name: format!("(error: {e})"), is_dir: false, size: 0 }];
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
    entries.sort_by(|a, b| match (a.is_dir, b.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.name.cmp(&b.name),
    });
    entries
}

fn open_file(name: &str, path: &PathBuf) -> View {
    let lines = match fs::read_to_string(path) {
        Ok(content) => content.lines().take(20).map(|s| s.to_string()).collect(),
        Err(e) => vec![format!("Cannot read file: {e}")],
    };
    View::File { name: name.to_string(), lines, scroll: 0 }
}

// ── Rendering ───────────────────────────────────────────────────────────────

fn draw(st: &State) {
    fill(0, 0, W, H, C_BG);
    fill(PX, PY, PW, PH, C_PANEL);

    match &st.view {
        View::List { entries, selected, scroll } => {
            draw_list_title(&st.cwd, entries.len());
            draw_list_body(entries, *selected, *scroll);
            // Draw input prompt or status
            match &st.input {
                InputMode::NewFile { buf } => {
                    draw_input_bar(&st.status_msg, buf);
                }
                InputMode::Rename { buf, .. } => {
                    draw_input_bar(&st.status_msg, buf);
                }
                InputMode::ConfirmDelete { .. } => {
                    draw_status_with_msg(&st.status_msg);
                }
                InputMode::None => {
                    if let Some(msg) = &st.status_msg {
                        draw_status(msg);
                    } else {
                        let hint = "Arrows:nav  Enter:open  Bksp:up  n:new  d:del  r:rename  q:quit";
                        draw_status(hint);
                    }
                }
            }
        }
        View::File { name, lines, scroll } => {
            draw_file_title(name, lines.len());
            draw_file_body(lines, *scroll);
            draw_status("Up/Down scroll   Backspace/q back to list");
        }
    }
    flush();
}

fn draw_list_title(cwd: &PathBuf, count: usize) {
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_ACCENT);
    text(PX + 10, PY + 6, C_ACCENT, "File Manager");
    let path_str = cwd.to_string_lossy();
    text(PX + 130, PY + 6, C_DIM, &format!("{path_str}  ({count} items)"));
    border(PX, PY, PW, PH, C_ACCENT);
}

fn draw_file_title(name: &str, line_count: usize) {
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_GREEN);
    text(PX + 10, PY + 6, C_GREEN, name);
    let nx = PX + 10 + name.len() as u32 * 8 + 12;
    text(nx, PY + 6, C_DIM, &format!("({line_count} lines)"));
    border(PX, PY, PW, PH, C_GREEN);
}

fn draw_status(hint: &str) {
    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_TITLE);
    fill(PX, sy, PW, 1, 0x30363DFF);
    text(INNER_X, sy + 3, C_DIM, hint);
}

fn draw_status_with_msg(msg: &Option<String>) {
    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_TITLE);
    if let Some(m) = msg { text(INNER_X, sy + 3, C_YELLOW, m); }
}

fn draw_input_bar(prompt: &Option<String>, buf: &str) {
    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_BG);
    fill(PX, sy, PW, 1, C_ACCENT);
    let prompt_str = prompt.as_deref().unwrap_or("Input:");
    text(INNER_X, sy + 3, C_ACCENT, prompt_str);
    // Draw the input buffer after the prompt
    let buf_x = INNER_X + prompt_str.len() as u32 * 8 + 8;
    text(buf_x, sy + 3, C_WHITE, buf);
    // Cursor indicator
    let cur_x = buf_x + buf.len() as u32 * 8;
    text(cur_x, sy + 3, C_ACCENT, "_");
}

fn draw_list_body(entries: &[Entry], selected: usize, scroll: usize) {
    fill(INNER_X, INNER_Y, INNER_W, INNER_H, C_PANEL);

    if entries.is_empty() {
        let cx = INNER_X + INNER_W / 2 - 80;
        let cy = INNER_Y + INNER_H / 2 - 8;
        text(cx, cy, C_DIM, "(empty directory)");
        return;
    }

    let visible = entries.iter().skip(scroll).take(MAX_VISIBLE).enumerate();
    for (i, entry) in visible {
        let abs_idx = scroll + i;
        let iy = INNER_Y + i as u32 * ITEM_H;

        if abs_idx == selected {
            fill(INNER_X, iy, LIST_W, ITEM_H, C_SELECT);
        }

        let (color, tag) = if entry.is_dir { (C_YELLOW, "DIR") } else { (C_WHITE, "   ") };

        text(INNER_X + 4, iy + 1, color, tag);

        let name_x = INNER_X + 36;
        let max_chars = (LIST_W.saturating_sub(200) / 8) as usize;
        let display_name = if entry.name.len() > max_chars {
            format!("{}...", &entry.name[..max_chars.saturating_sub(3)])
        } else {
            entry.name.clone()
        };
        text(name_x, iy + 1, color, &display_name);

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
    fill(INNER_X, INNER_Y, INNER_W, INNER_H, C_PANEL);

    let line_no_w: u32 = 40;
    let content_x = INNER_X + line_no_w + 4;
    let content_w = INNER_W - line_no_w - 4;

    let visible = lines.iter().skip(scroll).take(MAX_VISIBLE).enumerate();
    for (i, line) in visible {
        let ln = scroll + i + 1;
        let iy = INNER_Y + i as u32 * ITEM_H;
        text(INNER_X + 2, iy + 1, C_DIM, &format!("{ln:4}"));
        fill(INNER_X + line_no_w, iy, 1, ITEM_H, 0x30363DFF);
        text_wrap(content_x, iy + 1, content_w, C_WHITE, line);
    }

    if lines.is_empty() {
        text(INNER_X + 8, INNER_Y + 8, C_DIM, "(empty file)");
    }

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

// ── Helpers ─────────────────────────────────────────────────────────────────

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

// ── VYOMA_DRAW protocol helpers ─────────────────────────────────────────────

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
fn text_wrap(x: u32, y: u32, max_w: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text_wrap:{x},{y},{max_w},{rgba},m,{s}");
}

#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}
