use std::io::{self, BufRead, Write};

const W: u32 = 1440;
const H: u32 = 880;
const URL_X: u32 = 20;
const URL_Y: u32 = 12;
const URL_W: u32 = W - 40;
const URL_H: u32 = 34;
const CONTENT_X: u32 = 20;
const CONTENT_Y: u32 = 58;
const CONTENT_H: u32 = H - 100;
const LINE_H: u32 = 18;
const MAX_LINES: usize = (CONTENT_H / LINE_H) as usize;
const STATUS_Y: u32 = H - 28;
const MARGIN: u32 = 20;

const C_BG: u32      = 0xF6F8FAFF;
const C_TITLE: u32   = 0x1F2328FF;
const C_ACCENT: u32  = 0x0969DAFF;
const C_URL_BG: u32  = 0xFFFFFFFF;
const C_BORDER: u32  = 0xD0D7DEFF;
const C_STATUS: u32  = 0x6E7781FF;
const C_LINK: u32    = 0x0969DAFF;
const C_TOOLBAR: u32 = 0xEAEEF2FF;
const C_DIM: u32     = 0x8B949EFF;

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

fn strip_html(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut prev_space = false;
    for ch in s.chars() {
        match ch {
            '<' => { in_tag = true; }
            '>' => { in_tag = false; }
            _ if in_tag => {}
            '&' => { out.push(' '); }
            ch => {
                let is_space = ch.is_whitespace();
                if is_space {
                    if !prev_space { out.push(' '); }
                } else {
                    out.push(ch);
                }
                prev_space = is_space;
            }
        }
    }
    out
}

fn wrap_lines(text: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        let raw_line = raw_line.trim();
        if raw_line.is_empty() {
            lines.push(String::new());
            continue;
        }
        // Word-wrap at max_chars
        let mut cur = String::new();
        for word in raw_line.split_whitespace() {
            if cur.is_empty() {
                cur.push_str(word);
            } else if cur.len() + 1 + word.len() <= max_chars {
                cur.push(' ');
                cur.push_str(word);
            } else {
                lines.push(cur.clone());
                cur = word.to_string();
            }
        }
        if !cur.is_empty() { lines.push(cur); }
    }
    lines
}

fn draw(url_input: &str, url_focused: bool, page_lines: &[String], scroll: usize, status: &str, loading: bool) {
    // Background
    fill(0, 0, W, H, C_BG);

    // Toolbar
    fill(0, 0, W, URL_Y + URL_H + 12, C_TOOLBAR);
    fill(0, URL_Y + URL_H + 12, W, 1, C_BORDER);

    // URL bar
    fill(URL_X, URL_Y, URL_W, URL_H, C_URL_BG);
    border(URL_X, URL_Y, URL_W, URL_H, if url_focused { C_ACCENT } else { C_BORDER });
    let display_url = if url_input.is_empty() && !url_focused { "http://" } else { url_input };
    let url_color = if url_input.is_empty() && !url_focused { C_DIM } else { C_TITLE };
    text(URL_X + 10, URL_Y + 9, url_color, display_url);

    // Loading indicator
    if loading {
        text(URL_X, CONTENT_Y, C_DIM, "Loading...");
        flush();
        return;
    }

    // Page content
    let vis = page_lines.iter().skip(scroll).take(MAX_LINES);
    for (i, line) in vis.enumerate() {
        let y = CONTENT_Y + i as u32 * LINE_H;
        if !line.is_empty() {
            let clipped: &str = if line.len() > 160 { &line[..160] } else { line };
            // Detect links (simple: starts with http)
            let color = if clipped.trim_start().starts_with("http") { C_LINK } else { C_TITLE };
            text(CONTENT_X, y, color, clipped);
        }
    }

    // Status bar
    fill(0, STATUS_Y - 4, W, H - STATUS_Y + 4, C_TOOLBAR);
    fill(0, STATUS_Y - 5, W, 1, C_BORDER);
    let full_status = if url_focused {
        format!("Ctrl+C: quit  Enter: load  Ctrl+L: focus URL")
    } else {
        format!("{status}  |  Up/Down: scroll  Ctrl+L: URL bar  Ctrl+C: quit")
    };
    text(URL_X, STATUS_Y + 2, C_STATUS, &full_status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut url_input = String::from("http://");
    let mut url_focused = true;
    let mut page_lines: Vec<String> = Vec::new();
    let mut scroll = 0usize;
    let mut status = String::from("Ready");
    let mut loading = false;

    draw(&url_input, url_focused, &page_lines, scroll, &status, loading);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle http-get reply
        if let Some(rest) = raw.strip_prefix("REPLY:http-get ") {
            loading = false;
            if let Some(err) = rest.strip_prefix("error ") {
                status = format!("Error: {err}");
                page_lines = vec![format!("Failed to load: {err}")];
            } else {
                let (code, body) = rest.split_once(' ').unwrap_or((rest, ""));
                status = format!("HTTP {code}");
                // Unescape \n, strip HTML, wrap
                let unescaped = body.replace("\\n", "\n");
                let stripped = strip_html(&unescaped);
                page_lines = wrap_lines(&stripped, 140);
            }
            scroll = 0;
            draw(&url_input, url_focused, &page_lines, scroll, &status, loading);
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        // Ctrl+C
        if raw == "\x03" {
            fill(0, 0, W, H, 0x0D1117FF);
            flush();
            std::process::exit(0);
        }

        // Ctrl+L — focus URL bar
        if raw == "\x0C" {
            url_focused = true;
            draw(&url_input, url_focused, &page_lines, scroll, &status, loading);
            continue;
        }

        // Enter
        if raw.is_empty() {
            if url_focused {
                let url = url_input.trim().to_string();
                if !url.is_empty() {
                    loading = true;
                    url_focused = false;
                    status = format!("Loading {url}...");
                    draw(&url_input, url_focused, &page_lines, scroll, &status, loading);
                    println!("@supervisor: http-get {url}");
                    let _ = io::stdout().flush();
                }
            }
            continue;
        }

        // Arrow keys (when not focused on URL)
        if raw == "\x1b[A" {
            // Up
            if !url_focused {
                scroll = scroll.saturating_sub(3);
                draw(&url_input, url_focused, &page_lines, scroll, &status, loading);
            }
            continue;
        }
        if raw == "\x1b[B" {
            // Down
            if !url_focused {
                let max_scroll = page_lines.len().saturating_sub(MAX_LINES);
                scroll = (scroll + 3).min(max_scroll);
                draw(&url_input, url_focused, &page_lines, scroll, &status, loading);
            }
            continue;
        }

        // Backspace
        if raw == "\x7f" {
            if url_focused {
                url_input.pop();
                draw(&url_input, url_focused, &page_lines, scroll, &status, loading);
            }
            continue;
        }

        // Printable char
        if raw.len() == 1 {
            let ch = raw.chars().next().unwrap();
            if url_focused && (ch.is_ascii_graphic() || ch == ' ') {
                url_input.push(ch);
                draw(&url_input, url_focused, &page_lines, scroll, &status, loading);
            }
        }
    }
}
