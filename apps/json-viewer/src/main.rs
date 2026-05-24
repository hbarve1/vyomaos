// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 760;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_KEY: u32    = 0x58A6FFFF; // keys: blue
const C_STR: u32    = 0x3FB950FF; // strings: green
const C_NUM: u32    = 0xFFA657FF; // numbers: orange
const C_BOOL: u32   = 0x79C0FFFF; // booleans: light blue
const C_NULL: u32   = 0x8B949EFF; // null: dim
const C_PUNC: u32   = 0x6E7681FF; // braces/brackets: dim

const CONTENT_Y: u32   = 52;
const LINE_H: u32      = 16;
const VIS_LINES: usize = ((H - CONTENT_Y - 36) / LINE_H) as usize;
const MAX_CHARS: usize = (W - 24) as usize / 8;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

#[derive(Clone)]
struct Line {
    indent: u32,
    content: String,
    color:   u32,
}

// Very simple JSON pretty-printer — no serde, pure character scan
fn pretty_print(src: &str) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut indent  = 0u32;
    let mut chars   = src.chars().peekable();
    let mut cur     = String::new();
    let mut cur_col = C_PUNC;
    let mut in_str  = false;
    let mut escape  = false;

    let push_line = |lines: &mut Vec<Line>, indent: u32, content: String, color: u32| {
        if !content.trim().is_empty() {
            lines.push(Line { indent, content, color });
        }
    };

    while let Some(c) = chars.next() {
        if escape { cur.push(c); escape = false; continue; }
        if in_str {
            if c == '\\' { cur.push(c); escape = true; continue; }
            if c == '"'  {
                cur.push('"');
                in_str = false;
                // Peek: if next non-ws is ':', this is a key
                let peek: String = chars.clone().take(10).collect();
                let peek_trim = peek.trim_start();
                if peek_trim.starts_with(':') {
                    cur_col = C_KEY;
                } else {
                    cur_col = C_STR;
                }
                continue;
            }
            cur.push(c);
            continue;
        }

        match c {
            ' ' | '\t' | '\r' | '\n' => {
                // Collapse whitespace — we control layout
            }
            '"' => {
                if !cur.trim().is_empty() {
                    push_line(&mut lines, indent, cur.clone(), cur_col);
                    cur = String::new();
                }
                cur.push('"');
                in_str = true;
                cur_col = C_STR;
            }
            '{' | '[' => {
                let is_open_brace = c == '{';
                if !cur.trim().is_empty() {
                    let combined = format!("{cur} {c}");
                    push_line(&mut lines, indent, combined, cur_col);
                    cur = String::new();
                } else {
                    push_line(&mut lines, indent, c.to_string(), C_PUNC);
                }
                indent += 1;
                let _ = is_open_brace;
            }
            '}' | ']' => {
                if !cur.trim().is_empty() {
                    push_line(&mut lines, indent, cur.clone(), cur_col);
                    cur = String::new();
                }
                indent = indent.saturating_sub(1);
                // Check for trailing comma
                let peek: String = chars.clone().take(5).collect();
                let trailing = if peek.trim_start().starts_with(',') { "," } else { "" };
                push_line(&mut lines, indent, format!("{c}{trailing}"), C_PUNC);
                // Skip the comma we already consumed visually
                if !trailing.is_empty() {
                    // consume the actual comma
                    for nc in chars.by_ref() {
                        if nc == ',' { break; }
                        if !nc.is_whitespace() { cur.push(nc); break; }
                    }
                }
            }
            ':' => {
                cur.push_str(": ");
            }
            ',' => {
                let combined = format!("{cur},");
                push_line(&mut lines, indent, combined, cur_col);
                cur = String::new();
                cur_col = C_PUNC;
            }
            other => {
                if cur.is_empty() {
                    // Determine color of new token
                    cur_col = match other {
                        't' | 'f' => C_BOOL,
                        'n'       => C_NULL,
                        '-' | '0'..='9' => C_NUM,
                        _ => C_DIM,
                    };
                }
                cur.push(other);
            }
        }
    }
    if !cur.trim().is_empty() {
        push_line(&mut lines, indent, cur, cur_col);
    }
    lines
}

fn list_json() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/data") else { return Vec::new() };
    let mut files: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| format!("/data/{}", e.file_name().to_string_lossy()))
        .filter(|n| n.ends_with(".json"))
        .collect();
    files.sort();
    files
}

fn draw_list(files: &[String], cursor: usize) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "JSON Viewer  —  /data/*.json");
    fill(0, 34, W, 1, C_BORDER);
    if files.is_empty() {
        text(20, 60, C_DIM, "No .json files found in /data.");
    } else {
        for (i, f) in files.iter().enumerate() {
            let ty = 48 + i as u32 * 20;
            let tc = if i == cursor { C_ACCENT } else { C_DIM };
            text(20, ty, tc, f);
        }
        text(20, H - 18, C_HINT, "↑↓: select   Enter: open   Ctrl+C: quit");
    }
    flush();
}

fn draw_doc(files: &[String], idx: usize, lines: &[Line], scroll: usize) {
    fill(0, 0, W, H, C_BG);
    let fname = files.get(idx).map(|s| s.as_str()).unwrap_or("(unknown)");
    let hdr = format!("{fname}  ({} lines)  [{}/{}]", lines.len(), idx + 1, files.len());
    text(20, 14, C_ACCENT, &hdr);
    fill(0, 34, W, 1, C_BORDER);

    let end = (scroll + VIS_LINES).min(lines.len());
    for (vis_i, ln) in lines[scroll..end].iter().enumerate() {
        let ly = CONTENT_Y + vis_i as u32 * LINE_H;
        let lx = 20 + ln.indent * 16; // 2 spaces × 8px per space
        let full = format!("{}", ln.content);
        let avail = if lx < W { (W - lx) / 8 } else { 0 } as usize;
        let disp: String = full.chars().take(avail.min(MAX_CHARS)).collect();
        text(lx, ly + 1, ln.color, &disp);
    }

    fill(0, H - 30, W, 1, C_BORDER);
    text(20, H - 18, C_HINT,
        &format!("↑↓: scroll   ←→: prev/next   q/Esc: back   line {}/{}", scroll + 1, lines.len()));
    flush();
}

enum View {
    List { cursor: usize },
    Doc  { idx: usize, lines: Vec<Line>, scroll: usize },
}

fn load_doc(path: &str) -> Vec<Line> {
    match std::fs::read_to_string(path) {
        Ok(s)  => pretty_print(&s),
        Err(e) => vec![Line { indent: 0, content: format!("Error: {e}"), color: 0xFF7B72FF }],
    }
}

fn main() {
    let stdin = io::stdin();
    let files = list_json();

    let mut view = if files.len() == 1 {
        View::Doc { idx: 0, lines: load_doc(&files[0]), scroll: 0 }
    } else {
        View::List { cursor: 0 }
    };

    match &view {
        View::List { cursor }          => draw_list(&files, *cursor),
        View::Doc  { idx, lines, scroll } => draw_doc(&files, *idx, lines, *scroll),
    }

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match &mut view {
            View::List { cursor } => match raw.as_str() {
                "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
                "\x1b[A" => { if *cursor > 0 { *cursor -= 1; } draw_list(&files, *cursor); }
                "\x1b[B" => { if *cursor + 1 < files.len() { *cursor += 1; } draw_list(&files, *cursor); }
                "" => {
                    let idx = *cursor;
                    if files.get(idx).is_some() {
                        let lines = load_doc(&files[idx]);
                        view = View::Doc { idx, lines, scroll: 0 };
                        if let View::Doc { idx, lines, scroll } = &view {
                            draw_doc(&files, *idx, lines, *scroll);
                        }
                    }
                }
                _ => {}
            },
            View::Doc { idx, lines, scroll } => match raw.as_str() {
                "\x03" | "q" | "\x1b" => {
                    if files.len() <= 1 { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
                    let cur = *idx;
                    view = View::List { cursor: cur };
                    if let View::List { cursor } = &view { draw_list(&files, *cursor); }
                }
                "\x1b[A" => { if *scroll > 0 { *scroll -= 1; } draw_doc(&files, *idx, lines, *scroll); }
                "\x1b[B" => {
                    if *scroll + VIS_LINES < lines.len() { *scroll += 1; }
                    draw_doc(&files, *idx, lines, *scroll);
                }
                "\x1b[D" => {
                    let ni = if *idx > 0 { *idx - 1 } else { files.len().saturating_sub(1) };
                    *idx = ni; *lines = load_doc(&files[ni]); *scroll = 0;
                    draw_doc(&files, *idx, lines, *scroll);
                }
                "\x1b[C" => {
                    let ni = (*idx + 1) % files.len().max(1);
                    *idx = ni; *lines = load_doc(&files[ni]); *scroll = 0;
                    draw_doc(&files, *idx, lines, *scroll);
                }
                _ => {}
            },
        }
    }
}
