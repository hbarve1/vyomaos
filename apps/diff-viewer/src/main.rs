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
const C_ERR: u32    = 0xFF7B72FF;
const C_ADD: u32    = 0x3FB950FF;
const C_DEL: u32    = 0xFF7B72FF;
const C_SAME: u32   = 0x8B949EFF;
const C_FIELD: u32  = 0x21262DFF;

const CONTENT_Y: u32   = 52;
const LINE_H: u32      = 16;
const VIS_LINES: usize = ((H - CONTENT_Y - 36) / LINE_H) as usize;
const MAX_CHARS: usize = (W - 52) as usize / 8;

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
enum DiffLine {
    Same(String),
    Add(String),
    Del(String),
}

impl DiffLine {
    fn color(&self) -> u32 {
        match self {
            DiffLine::Same(_) => C_SAME,
            DiffLine::Add(_)  => C_ADD,
            DiffLine::Del(_)  => C_DEL,
        }
    }
    fn prefix(&self) -> &'static str {
        match self {
            DiffLine::Same(_) => "  ",
            DiffLine::Add(_)  => "+ ",
            DiffLine::Del(_)  => "- ",
        }
    }
    fn content(&self) -> &str {
        match self {
            DiffLine::Same(s) | DiffLine::Add(s) | DiffLine::Del(s) => s,
        }
    }
}

// LCS-based diff: returns sequence of DiffLine
fn diff(a: &[String], b: &[String]) -> Vec<DiffLine> {
    // Cap input size to avoid O(N²) blowup
    let a_len = a.len().min(500);
    let b_len = b.len().min(500);
    let a = &a[..a_len];
    let b = &b[..b_len];

    // Build LCS table
    let mut dp = vec![vec![0u32; b_len + 1]; a_len + 1];
    for i in (0..a_len).rev() {
        for j in (0..b_len).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }

    // Reconstruct diff
    let mut result = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;
    while i < a_len && j < b_len {
        if a[i] == b[j] {
            result.push(DiffLine::Same(a[i].clone()));
            i += 1; j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            result.push(DiffLine::Del(a[i].clone()));
            i += 1;
        } else {
            result.push(DiffLine::Add(b[j].clone()));
            j += 1;
        }
    }
    while i < a_len { result.push(DiffLine::Del(a[i].clone())); i += 1; }
    while j < b_len { result.push(DiffLine::Add(b[j].clone())); j += 1; }
    result
}

fn read_file(path: &str) -> Result<Vec<String>, String> {
    std::fs::read_to_string(path)
        .map(|s| s.lines().map(|l| l.to_string()).collect())
        .map_err(|e| format!("{e}"))
}

enum Mode {
    PathA { input: String, error: String },
    PathB { path_a: String, input: String, error: String },
    DiffView { path_a: String, path_b: String, lines: Vec<DiffLine>, scroll: usize },
}

fn draw_path_input(step: u32, label: &str, input: &str, error: &str) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Diff Viewer");
    fill(0, 34, W, 1, C_BORDER);
    let step_str = format!("Step {step}/2 — {label}");
    text(20, 52, C_DIM, &step_str);
    fill(20, 72, W - 40, 32, C_FIELD);
    border(20, 72, W - 40, 32, C_BORDER);
    let display = if input.is_empty() { "/data/" } else { input };
    let tc = if input.is_empty() { C_HINT } else { C_DIM };
    text(30, 80, tc, display);
    if !error.is_empty() {
        text(20, 116, C_ERR, error);
    }
    text(20, H - 18, C_HINT, "Enter: confirm   Esc/Ctrl+C: quit");
    flush();
}

fn draw_diff(path_a: &str, path_b: &str, lines: &[DiffLine], scroll: usize) {
    fill(0, 0, W, H, C_BG);
    let adds  = lines.iter().filter(|l| matches!(l, DiffLine::Add(_))).count();
    let dels  = lines.iter().filter(|l| matches!(l, DiffLine::Del(_))).count();
    let hdr = format!("Diff: {path_a} ↔ {path_b}  [+{adds} -{dels}]");
    let hdr_disp: String = hdr.chars().take(160).collect();
    text(20, 14, C_ACCENT, &hdr_disp);
    fill(0, 34, W, 1, C_BORDER);

    let end = (scroll + VIS_LINES).min(lines.len());
    for (vis_i, dl) in lines[scroll..end].iter().enumerate() {
        let ly = CONTENT_Y + vis_i as u32 * LINE_H;
        let color  = dl.color();
        let prefix = dl.prefix();
        // Highlight add/del rows with background tint
        match dl {
            DiffLine::Add(_) => fill(0, ly, W, LINE_H, 0x0D2818FF),
            DiffLine::Del(_) => fill(0, ly, W, LINE_H, 0x2D1010FF),
            DiffLine::Same(_) => {}
        }
        let full = format!("{prefix}{}", dl.content());
        let display: String = full.chars().take(MAX_CHARS).collect();
        text(20, ly + 1, color, &display);
    }

    fill(0, H - 30, W, 1, C_BORDER);
    let info = format!("{}/{} lines", scroll + 1, lines.len());
    text(20, H - 18, C_HINT, &format!("↑↓: scroll   Esc/q: back   {info}"));
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut mode = Mode::PathA { input: String::new(), error: String::new() };

    draw_path_input(1, "First file (A)", "", "");

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match &mut mode {
            Mode::PathA { input, error } => {
                match raw.as_str() {
                    "\x03" | "\x1b" => {
                        fill(0, 0, W, H, C_BG);
                        flush();
                        std::process::exit(0);
                    }
                    "\x7f" => {
                        input.pop();
                        error.clear();
                        let i = input.clone();
                        draw_path_input(1, "First file (A)", &i, "");
                    }
                    "" => {
                        let path = input.clone();
                        match read_file(&path) {
                            Ok(_) => {
                                mode = Mode::PathB { path_a: path, input: String::new(), error: String::new() };
                                draw_path_input(2, "Second file (B)", "", "");
                            }
                            Err(e) => {
                                *error = format!("Error: {e}");
                                let i = input.clone();
                                let err = error.clone();
                                draw_path_input(1, "First file (A)", &i, &err);
                            }
                        }
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' {
                            input.push(c);
                            error.clear();
                            let i = input.clone();
                            draw_path_input(1, "First file (A)", &i, "");
                        }
                    }
                    _ => {}
                }
            }

            Mode::PathB { path_a, input, error } => {
                match raw.as_str() {
                    "\x03" | "\x1b" => {
                        mode = Mode::PathA { input: String::new(), error: String::new() };
                        draw_path_input(1, "First file (A)", "", "");
                    }
                    "\x7f" => {
                        input.pop();
                        error.clear();
                        let i = input.clone();
                        draw_path_input(2, "Second file (B)", &i, "");
                    }
                    "" => {
                        let pa = path_a.clone();
                        let pb = input.clone();
                        match (read_file(&pa), read_file(&pb)) {
                            (Ok(a_lines), Ok(b_lines)) => {
                                let diff_lines = diff(&a_lines, &b_lines);
                                mode = Mode::DiffView { path_a: pa, path_b: pb, lines: diff_lines, scroll: 0 };
                                if let Mode::DiffView { path_a, path_b, lines, scroll } = &mode {
                                    draw_diff(path_a, path_b, lines, *scroll);
                                }
                            }
                            (Err(e), _) => {
                                *error = format!("Error reading A: {e}");
                                let i = input.clone();
                                let err = error.clone();
                                draw_path_input(2, "Second file (B)", &i, &err);
                            }
                            (_, Err(e)) => {
                                *error = format!("Error reading B: {e}");
                                let i = input.clone();
                                let err = error.clone();
                                draw_path_input(2, "Second file (B)", &i, &err);
                            }
                        }
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' {
                            input.push(c);
                            error.clear();
                            let i = input.clone();
                            draw_path_input(2, "Second file (B)", &i, "");
                        }
                    }
                    _ => {}
                }
            }

            Mode::DiffView { scroll, lines, .. } => {
                match raw.as_str() {
                    "\x03" | "\x1b" | "q" => {
                        mode = Mode::PathA { input: String::new(), error: String::new() };
                        draw_path_input(1, "First file (A)", "", "");
                    }
                    "\x1b[A" => {
                        if *scroll > 0 { *scroll -= 1; }
                        if let Mode::DiffView { path_a, path_b, lines, scroll } = &mode {
                            draw_diff(path_a, path_b, lines, *scroll);
                        }
                    }
                    "\x1b[B" => {
                        if let Mode::DiffView { path_a, path_b, lines, scroll } = &mut mode {
                            if *scroll + VIS_LINES < lines.len() { *scroll += 1; }
                            draw_diff(path_a, path_b, lines, *scroll);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
