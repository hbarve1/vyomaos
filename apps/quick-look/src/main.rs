use std::io::{self, BufRead, Write};

const W: u32 = 1040;
const H: u32 = 680;
const HEADER_H: u32 = 44;
const FOOTER_H: u32 = 28;
const CONTENT_H: u32 = H - HEADER_H - FOOTER_H;

const C_BG: u32     = 0x1C2128F4;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_H1: u32     = 0x58A6FFFF;
const C_H2: u32     = 0x79C0FFFF;
const C_CODE: u32   = 0xFFA657FF;
const C_KEY: u32    = 0x58A6FFFF;
const C_STR: u32    = 0x3FB950FF;
const C_NUM: u32    = 0xFFA657FF;
const C_BOOL: u32   = 0x79C0FFFF;
const C_ERR: u32    = 0xFF7B72FF;
const C_WARN: u32   = 0xFFA657FF;

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

struct Line {
    content: String,
    color:   u32,
    indent:  u32,
}

fn render_md(content: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for raw in content.lines().take(60) {
        let (color, indent, text) = if raw.starts_with("# ") {
            (C_H1, 0, raw[2..].to_string())
        } else if raw.starts_with("## ") {
            (C_H2, 0, raw[3..].to_string())
        } else if raw.starts_with("### ") {
            (C_H2, 8, raw[4..].to_string())
        } else if raw.starts_with("```") {
            (C_CODE, 0, raw.to_string())
        } else if raw.starts_with("- ") || raw.starts_with("* ") {
            (C_DIM, 8, format!("• {}", &raw[2..]))
        } else if raw.starts_with("  - ") {
            (C_DIM, 20, format!("  · {}", &raw[4..]))
        } else if raw.is_empty() {
            (C_DIM, 0, String::new())
        } else {
            (C_TEXT, 0, raw.to_string())
        };
        let content: String = text.chars().take(120).collect();
        lines.push(Line { content, color, indent });
    }
    lines
}

fn render_json(content: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut depth = 0u32;
    for raw in content.lines().take(80) {
        let trimmed = raw.trim();
        let indent = depth * 14;
        let (color, txt) = if trimmed.starts_with('"') && trimmed.contains(':') {
            let colon = trimmed.find(':').unwrap_or(0);
            (C_KEY, trimmed[..colon + 1].trim_matches('"').to_string() + &trimmed[colon + 1..])
        } else if trimmed.starts_with('"') {
            (C_STR, trimmed.to_string())
        } else if trimmed.parse::<f64>().is_ok() {
            (C_NUM, trimmed.to_string())
        } else if trimmed == "true" || trimmed == "false" {
            (C_BOOL, trimmed.to_string())
        } else if trimmed == "null" {
            (C_HINT, trimmed.to_string())
        } else {
            (C_DIM, trimmed.to_string())
        };
        if trimmed.ends_with('{') || trimmed.ends_with('[') { depth += 1; }
        if trimmed.starts_with('}') || trimmed.starts_with(']') { depth = depth.saturating_sub(1); }
        let content: String = txt.chars().take(100).collect();
        lines.push(Line { content, color, indent: indent.min(80) });
    }
    lines
}

fn render_csv(content: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    for (i, raw) in content.lines().take(25).enumerate() {
        let cols: Vec<&str> = raw.split(',').collect();
        let color = if i == 0 { C_H1 } else if i % 2 == 0 { C_TEXT } else { C_DIM };
        let joined: String = cols.iter().map(|c| {
            let t = c.trim_matches('"').trim();
            format!("{:<20}", if t.len() > 18 { &t[..18] } else { t })
        }).collect::<Vec<_>>().join(" ");
        let content: String = joined.chars().take(120).collect();
        lines.push(Line { content, color, indent: 0 });
    }
    lines
}

fn render_log(content: &str) -> Vec<Line> {
    let all: Vec<&str> = content.lines().collect();
    let start = if all.len() > 40 { all.len() - 40 } else { 0 };
    all[start..].iter().map(|raw| {
        let low = raw.to_ascii_lowercase();
        let color = if low.contains("error") || low.contains("err") { C_ERR }
            else if low.contains("warn") { C_WARN }
            else if low.contains("info") { C_STR }
            else { C_DIM };
        let content: String = raw.chars().take(120).collect();
        Line { content, color, indent: 0 }
    }).collect()
}

fn render_text(content: &str) -> Vec<Line> {
    content.lines().take(60).map(|raw| {
        let content: String = raw.chars().take(120).collect();
        Line { content, color: C_TEXT, indent: 0 }
    }).collect()
}

fn ext_type(name: &str) -> &str {
    if name.ends_with(".md")   { "Markdown" }
    else if name.ends_with(".json") { "JSON" }
    else if name.ends_with(".csv")  { "CSV" }
    else if name.ends_with(".log")  { "Log" }
    else if name.ends_with(".toml") { "TOML" }
    else { "Text" }
}

fn draw(filename: &str, lines: &[Line], scroll: usize) {
    fill(0, 0, W, H, C_BG);
    border(0, 0, W, H, C_BORDER);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);

    let fname: String = filename.chars().take(50).collect();
    text(16, 14, C_TEXT, &fname);

    let kind = ext_type(filename);
    let kw = kind.len() as u32 * 8 + 16;
    fill(W - kw - 60, 10, kw, 24, C_BORDER);
    text(W - kw - 52, 14, C_DIM, kind);

    // Content
    let visible = (CONTENT_H / 18) as usize;
    let start = scroll;
    let end = (start + visible).min(lines.len());

    for (i, line) in lines[start..end].iter().enumerate() {
        let ly = HEADER_H + 4 + i as u32 * 18;
        if !line.content.is_empty() {
            text(16 + line.indent, ly, line.color, &line.content);
        }
    }

    // Footer
    fill(0, H - FOOTER_H, W, FOOTER_H, C_HEADER);
    fill(0, H - FOOTER_H, W, 1, C_BORDER);
    let scroll_info = format!("Lines {}-{} of {}", start + 1, end, lines.len());
    text(16, H - FOOTER_H + 8, C_HINT, &scroll_info);
    text(W - 240, H - FOOTER_H + 8, C_HINT, "↑↓: scroll  Space/Esc: close");

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut filename = String::from("(no file)");
    let mut lines: Vec<Line> = Vec::new();
    let mut scroll = 0usize;
    let mut loaded = false;

    println!("@supervisor: raise quick-look");
    let _ = io::stdout().flush();

    draw(&filename, &lines, scroll);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("PREVIEW:") && !loaded {
            let path = raw.trim_start_matches("PREVIEW:").trim().to_string();
            filename = path.split('/').last().unwrap_or(&path).to_string();
            // Read file content — use @supervisor: read-file if available, else show placeholder
            println!("@supervisor: read-file {path}");
            let _ = io::stdout().flush();
            loaded = true;
            draw(&filename, &lines, scroll);
            continue;
        }

        if raw.starts_with("REPLY:read-file:") {
            let content = raw.trim_start_matches("REPLY:read-file:");
            let kind = ext_type(&filename);
            lines = match kind {
                "Markdown" => render_md(content),
                "JSON"     => render_json(content),
                "CSV"      => render_csv(content),
                "Log"      => render_log(content),
                _          => render_text(content),
            };
            scroll = 0;
            draw(&filename, &lines, scroll);
            continue;
        }

        if raw.starts_with("REPLY:") { continue; }

        let visible = (CONTENT_H / 18) as usize;
        match raw.as_str() {
            " " | "\x1b" | "\x03" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if scroll > 0 { scroll -= 1; }
                draw(&filename, &lines, scroll);
            }
            "\x1b[B" => {
                if scroll + visible < lines.len() { scroll += 1; }
                draw(&filename, &lines, scroll);
            }
            _ => {}
        }
    }
}
