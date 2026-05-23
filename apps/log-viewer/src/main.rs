use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 760;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_ERR: u32    = 0xFF7B72FF;
const C_WARN: u32   = 0xFFA657FF;
const C_INFO: u32   = 0x58A6FFFF;
const C_OK: u32     = 0x3FB950FF;

const CONTENT_Y: u32    = 52;
const LINE_H: u32       = 16;
const VIS_LINES: usize  = ((H - CONTENT_Y - 36) / LINE_H) as usize;
const MAX_LINE_CHARS: usize = (W - 40) as usize / 8;

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

fn line_color(line: &str) -> u32 {
    let lower = line.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("fatal") || lower.contains("crit") {
        C_ERR
    } else if lower.contains("warn") {
        C_WARN
    } else if lower.contains("info") || lower.contains("debug") {
        C_INFO
    } else if lower.contains("ok") || lower.contains("success") {
        C_OK
    } else {
        C_DIM
    }
}

fn list_logs() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/data") else { return Vec::new() };
    let mut files: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| format!("/data/{}", e.file_name().to_string_lossy()))
        .filter(|n| n.ends_with(".log"))
        .collect();
    files.sort();
    files
}

fn load_log(path: &str) -> Vec<String> {
    match std::fs::read_to_string(path) {
        Ok(s)  => s.lines().map(|l| l.to_string()).collect(),
        Err(e) => vec![format!("Error loading file: {e}")],
    }
}

fn filtered<'a>(lines: &'a [String], filter: &str) -> Vec<&'a str> {
    if filter.is_empty() {
        lines.iter().map(|s| s.as_str()).collect()
    } else {
        lines.iter()
            .filter(|l| l.to_ascii_lowercase().contains(&filter.to_ascii_lowercase()))
            .map(|s| s.as_str())
            .collect()
    }
}

fn draw_list(files: &[String], cursor: usize) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Log Viewer  —  /data/*.log");
    fill(0, 34, W, 1, C_BORDER);
    if files.is_empty() {
        text(20, 60, C_DIM, "No .log files found in /data.");
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

fn draw_log(files: &[String], file_idx: usize, lines: &[String], scroll: usize, filter: &str, follow: bool) {
    fill(0, 0, W, H, C_BG);
    let fname = files.get(file_idx).map(|s| s.as_str()).unwrap_or("(unknown)");
    let all_filtered = filtered(lines, filter);
    let total = all_filtered.len();
    let follow_str = if follow { " [FOLLOW]" } else { "" };
    let filter_str = if filter.is_empty() { String::new() } else { format!("  filter: {filter}") };
    let hdr = format!("{fname}  ({} lines){follow_str}{filter_str}", lines.len());
    text(20, 14, C_ACCENT, &hdr);
    fill(0, 34, W, 1, C_BORDER);

    let end = (scroll + VIS_LINES).min(total);
    for (vis_i, &line) in all_filtered[scroll..end].iter().enumerate() {
        let ly = CONTENT_Y + vis_i as u32 * LINE_H;
        let color = line_color(line);
        let display: String = line.chars().take(MAX_LINE_CHARS).collect();
        text(20, ly, color, &display);
    }

    fill(0, H - 30, W, 1, C_BORDER);
    let status = format!("{}/{} lines  {}", scroll + 1, total, if filter.is_empty() { "type to filter  Esc: clear".to_string() } else { format!("Esc: clear filter") });
    text(20, H - 18, C_HINT, &format!("↑↓: scroll   f: follow   ←/q: back   {status}"));
    flush();
}

enum View {
    List { cursor: usize },
    Log  { file_idx: usize, lines: Vec<String>, scroll: usize, filter: String, follow: bool },
}

fn main() {
    let stdin = io::stdin();
    let files = list_logs();

    let mut view = if files.len() == 1 {
        let lines = load_log(&files[0]);
        let scroll = lines.len().saturating_sub(VIS_LINES);
        View::Log { file_idx: 0, lines, scroll, filter: String::new(), follow: true }
    } else {
        View::List { cursor: 0 }
    };

    match &view {
        View::List { cursor } => draw_list(&files, *cursor),
        View::Log { file_idx, lines, scroll, filter, follow } =>
            draw_log(&files, *file_idx, lines, *scroll, filter, *follow),
    }

    // Start ping loop for auto-refresh
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

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
                            let lines = load_log(path);
                            let scroll = lines.len().saturating_sub(VIS_LINES);
                            view = View::Log { file_idx: idx, lines, scroll, filter: String::new(), follow: true };
                            if let View::Log { file_idx, lines, scroll, filter, follow } = &view {
                                draw_log(&files, *file_idx, lines, *scroll, filter, *follow);
                            }
                        }
                    }
                    _ => {}
                }
            }

            View::Log { file_idx, lines, scroll, filter, follow } => {
                // On REPLY:pong — re-read file if following
                if raw.starts_with("REPLY:pong") || raw.starts_with("REPLY:ping") {
                    println!("@supervisor: ping");
                    let _ = io::stdout().flush();
                    if *follow {
                        if let Some(path) = files.get(*file_idx) {
                            *lines = load_log(path);
                            let all_filtered = filtered(lines, filter);
                            *scroll = all_filtered.len().saturating_sub(VIS_LINES);
                            drop(all_filtered);
                        }
                    }
                    draw_log(&files, *file_idx, lines, *scroll, filter, *follow);
                    continue;
                }

                let all_count = filtered(lines, filter).len();
                match raw.as_str() {
                    "\x03" | "\x1b[D" | "q" => {
                        if files.len() <= 1 {
                            fill(0, 0, W, H, C_BG);
                            flush();
                            std::process::exit(0);
                        }
                        let cur = *file_idx;
                        view = View::List { cursor: cur };
                        if let View::List { cursor } = &view {
                            draw_list(&files, *cursor);
                        }
                    }
                    "\x1b" => {
                        if filter.is_empty() {
                            if files.len() <= 1 {
                                fill(0, 0, W, H, C_BG);
                                flush();
                                std::process::exit(0);
                            }
                            let cur = *file_idx;
                            view = View::List { cursor: cur };
                            if let View::List { cursor } = &view {
                                draw_list(&files, *cursor);
                            }
                        } else {
                            filter.clear();
                            draw_log(&files, *file_idx, lines, *scroll, filter, *follow);
                        }
                    }
                    "\x1b[A" => {
                        if *scroll > 0 { *scroll -= 1; }
                        *follow = false;
                        draw_log(&files, *file_idx, lines, *scroll, filter, *follow);
                    }
                    "\x1b[B" => {
                        if *scroll + VIS_LINES < all_count { *scroll += 1; }
                        draw_log(&files, *file_idx, lines, *scroll, filter, *follow);
                    }
                    "f" | "F" => {
                        *follow = !*follow;
                        if *follow {
                            let fc = filtered(lines, filter).len();
                            *scroll = fc.saturating_sub(VIS_LINES);
                        }
                        draw_log(&files, *file_idx, lines, *scroll, filter, *follow);
                    }
                    "\x7f" => {
                        filter.pop();
                        draw_log(&files, *file_idx, lines, *scroll, filter, *follow);
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' {
                            filter.push(c);
                            *scroll = 0;
                            draw_log(&files, *file_idx, lines, *scroll, filter, *follow);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
