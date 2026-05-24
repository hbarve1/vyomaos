// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1240;
const H: u32 = 760;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_H1: u32     = 0xFFFFFFFF;
const C_H2: u32     = 0xE6EDF3FF;
const C_H3: u32     = 0xCDD9E5FF;
const C_BOLD: u32   = 0x58A6FFFF;
const C_CODE: u32   = 0x3FB950FF;
const C_BULLET: u32 = 0xFF7B72FF;
const C_RULE: u32   = 0x30363DFF;

const CONTENT_X: u32 = 24;
const CONTENT_Y: u32 = 52;
const LINE_H: u32    = 20;
const VIS_LINES: usize = ((H - CONTENT_Y - 36) / LINE_H) as usize;

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
enum Span {
    Normal(String),
    Bold(String),
    Code(String),
    Bullet(String),
    H1(String),
    H2(String),
    H3(String),
    Rule,
    Blank,
}

impl Span {
    fn color(&self) -> u32 {
        match self {
            Span::Normal(_) => C_DIM,
            Span::Bold(_)   => C_BOLD,
            Span::Code(_)   => C_CODE,
            Span::Bullet(_) => C_DIM,
            Span::H1(_)     => C_H1,
            Span::H2(_)     => C_H2,
            Span::H3(_)     => C_H3,
            Span::Rule      => C_RULE,
            Span::Blank     => C_BG,
        }
    }

    fn text_content(&self) -> Option<&str> {
        match self {
            Span::Normal(s) | Span::Bold(s) | Span::Code(s) |
            Span::H1(s) | Span::H2(s) | Span::H3(s) => Some(s),
            Span::Bullet(s) => Some(s),
            _ => None,
        }
    }

    fn prefix(&self) -> &str {
        match self {
            Span::H1(_) => "# ",
            Span::H2(_) => "## ",
            Span::H3(_) => "### ",
            Span::Bullet(_) => "• ",
            _ => "",
        }
    }

    fn indent(&self) -> u32 {
        match self {
            Span::Bullet(_) => 16,
            _ => 0,
        }
    }
}

fn parse_inline(raw: &str) -> Vec<Span> {
    // Very simple inline parser: split on `**...**` and `` `...` ``
    let mut spans: Vec<Span> = Vec::new();
    let mut s = raw;
    while !s.is_empty() {
        if let Some(start) = s.find("**") {
            if start > 0 {
                spans.push(Span::Normal(s[..start].to_string()));
            }
            let rest = &s[start + 2..];
            if let Some(end) = rest.find("**") {
                spans.push(Span::Bold(rest[..end].to_string()));
                s = &rest[end + 2..];
            } else {
                spans.push(Span::Normal(s[start..].to_string()));
                break;
            }
        } else if let Some(start) = s.find('`') {
            if start > 0 {
                spans.push(Span::Normal(s[..start].to_string()));
            }
            let rest = &s[start + 1..];
            if let Some(end) = rest.find('`') {
                spans.push(Span::Code(rest[..end].to_string()));
                s = &rest[end + 1..];
            } else {
                spans.push(Span::Normal(s[start..].to_string()));
                break;
            }
        } else {
            spans.push(Span::Normal(s.to_string()));
            break;
        }
    }
    spans
}

fn parse_md(content: &str) -> Vec<Span> {
    let mut lines: Vec<Span> = Vec::new();
    for raw_line in content.lines() {
        let line = raw_line.trim_end();
        if line.starts_with("### ") {
            lines.push(Span::H3(line[4..].to_string()));
        } else if line.starts_with("## ") {
            lines.push(Span::H2(line[3..].to_string()));
        } else if line.starts_with("# ") {
            lines.push(Span::H1(line[2..].to_string()));
        } else if line.starts_with("- ") || line.starts_with("* ") {
            lines.push(Span::Bullet(line[2..].to_string()));
        } else if line.starts_with("---") || line.starts_with("===") {
            lines.push(Span::Rule);
        } else if line.is_empty() {
            lines.push(Span::Blank);
        } else {
            // Inline parse for bold/code
            let spans = parse_inline(line);
            for sp in spans {
                lines.push(sp);
            }
        }
    }
    lines
}

fn list_md() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/data") else { return Vec::new() };
    let mut files: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| format!("/data/{}", e.file_name().to_string_lossy()))
        .filter(|n| n.ends_with(".md"))
        .collect();
    files.sort();
    files
}

fn draw_list(files: &[String], cursor: usize) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Markdown Viewer  —  /data/*.md");
    fill(0, 34, W, 1, C_BORDER);
    if files.is_empty() {
        text(20, 60, C_DIM, "No .md files found in /data.");
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

fn draw_doc(files: &[String], idx: usize, spans: &[Span], scroll: usize) {
    fill(0, 0, W, H, C_BG);
    let fname = files.get(idx).map(|s| s.as_str()).unwrap_or("(unknown)");
    let hdr = format!("{fname}  [{}/{}]", idx + 1, files.len());
    text(20, 14, C_ACCENT, &hdr);
    fill(0, 34, W, 1, C_BORDER);

    let end = (scroll + VIS_LINES).min(spans.len());
    for (vis_i, span) in spans[scroll..end].iter().enumerate() {
        let ly = CONTENT_Y + vis_i as u32 * LINE_H;
        match span {
            Span::Rule => {
                fill(CONTENT_X, ly + LINE_H / 2, W - 48, 1, C_RULE);
            }
            Span::Blank => {}
            _ => {
                let prefix = span.prefix();
                let indent  = span.indent();
                let color   = span.color();
                if let Some(content) = span.text_content() {
                    let full = format!("{prefix}{content}");
                    // Truncate to avoid overflowing window width
                    let max_chars = ((W - CONTENT_X - indent - 24) / 8) as usize;
                    let display: String = full.chars().take(max_chars).collect();
                    text(CONTENT_X + indent, ly + 2, color, &display);
                }
            }
        }
    }

    fill(0, H - 30, W, 1, C_BORDER);
    let info = format!("line {}/{}", scroll + 1, spans.len());
    text(20, H - 18, C_HINT,
        &format!("↑↓: scroll   ←→: prev/next   Esc/q: back   {info}"));
    flush();
}

enum View {
    List { cursor: usize },
    Doc  { idx: usize, spans: Vec<Span>, scroll: usize },
}

fn load_doc(path: &str) -> Vec<Span> {
    match std::fs::read_to_string(path) {
        Ok(s)  => parse_md(&s),
        Err(e) => vec![Span::Normal(format!("Error: {e}"))],
    }
}

fn main() {
    let stdin = io::stdin();
    let files = list_md();

    let mut view = if files.len() == 1 {
        let spans = load_doc(&files[0]);
        View::Doc { idx: 0, spans, scroll: 0 }
    } else {
        View::List { cursor: 0 }
    };

    match &view {
        View::List { cursor }         => draw_list(&files, *cursor),
        View::Doc  { idx, spans, scroll } => draw_doc(&files, *idx, spans, *scroll),
    }

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match &mut view {
            View::List { cursor } => {
                match raw.as_str() {
                    "\x03" => {
                        fill(0, 0, W, H, C_BG);
                        flush();
                        std::process::exit(0);
                    }
                    "\x1b[A" => {
                        if *cursor > 0 { *cursor -= 1; }
                        draw_list(&files, *cursor);
                    }
                    "\x1b[B" => {
                        if *cursor + 1 < files.len() { *cursor += 1; }
                        draw_list(&files, *cursor);
                    }
                    "" => {
                        let idx = *cursor;
                        if let Some(path) = files.get(idx) {
                            let spans = load_doc(path);
                            view = View::Doc { idx, spans, scroll: 0 };
                            if let View::Doc { idx, spans, scroll } = &view {
                                draw_doc(&files, *idx, spans, *scroll);
                            }
                        }
                    }
                    _ => {}
                }
            }
            View::Doc { idx, spans, scroll } => {
                let total = spans.len();
                match raw.as_str() {
                    "\x03" | "\x1b" | "q" => {
                        if files.len() <= 1 {
                            fill(0, 0, W, H, C_BG);
                            flush();
                            std::process::exit(0);
                        }
                        let cur = *idx;
                        view = View::List { cursor: cur };
                        if let View::List { cursor } = &view {
                            draw_list(&files, *cursor);
                        }
                    }
                    "\x1b[A" => {
                        if *scroll > 0 { *scroll -= 1; }
                        draw_doc(&files, *idx, spans, *scroll);
                    }
                    "\x1b[B" => {
                        if *scroll + VIS_LINES < total { *scroll += 1; }
                        draw_doc(&files, *idx, spans, *scroll);
                    }
                    "\x1b[D" => {
                        let new_idx = if *idx > 0 { *idx - 1 } else { files.len().saturating_sub(1) };
                        if let Some(path) = files.get(new_idx) {
                            let new_spans = load_doc(path);
                            *idx = new_idx;
                            *spans = new_spans;
                            *scroll = 0;
                        }
                        draw_doc(&files, *idx, spans, *scroll);
                    }
                    "\x1b[C" => {
                        let new_idx = (*idx + 1) % files.len().max(1);
                        if let Some(path) = files.get(new_idx) {
                            let new_spans = load_doc(path);
                            *idx = new_idx;
                            *spans = new_spans;
                            *scroll = 0;
                        }
                        draw_doc(&files, *idx, spans, *scroll);
                    }
                    _ => {}
                }
            }
        }
    }
}
