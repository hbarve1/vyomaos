use std::io::{self, BufRead, Write};

const W: u32 = 840;
const H: u32 = 500;
const C_BG: u32     = 0x0D1117FF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_FIELD: u32  = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_OK: u32     = 0x3FB950FF;
const C_ERR: u32    = 0xFF7B72FF;
const C_HINT: u32   = 0x6E7681FF;

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

struct Entry {
    host: String,
    result: String,
    ok: bool,
}

fn draw(input: &str, results: &[Entry], pending: bool) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "DNS Resolver");

    // Input field
    text(20, 46, C_DIM, "Hostname:");
    fill(20, 64, W - 40, 34, C_FIELD);
    border(20, 64, W - 40, 34, C_BORDER);
    if !input.is_empty() {
        text(30, 73, C_TITLE, input);
    } else {
        text(30, 73, C_HINT, "type hostname and press Enter...");
    }

    // Status indicator
    if pending {
        text(20, 108, C_DIM, "resolving...");
    }

    // Results
    let result_y = 126u32;
    fill(20, result_y, W - 40, 1, C_BORDER);
    for (i, entry) in results.iter().rev().take(14).enumerate() {
        let ry = result_y + 10 + i as u32 * 24;
        let c = if entry.ok { C_OK } else { C_ERR };
        let line = format!("{} -> {}", entry.host, entry.result);
        text(20, ry, c, &line);
    }

    // Footer
    text(20, H - 22, C_HINT, "Enter: resolve  Backspace: delete  Ctrl+C: quit");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut input = String::new();
    let mut results: Vec<Entry> = Vec::new();
    let mut pending = false;

    draw(&input, &results, pending);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // REPLY from supervisor
        if let Some(rest) = raw.strip_prefix("REPLY:dns ") {
            pending = false;
            let mut parts = rest.splitn(2, ' ');
            let host = parts.next().unwrap_or("").to_string();
            let ip   = parts.next().unwrap_or("NXDOMAIN").to_string();
            let ok   = ip != "NXDOMAIN" && !ip.starts_with("error");
            results.push(Entry { host, result: ip, ok });
            draw(&input, &results, pending);
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        // Ctrl+C
        if raw == "\x03" {
            fill(0, 0, W, H, 0x0D1117FF);
            flush();
            std::process::exit(0);
        }

        // Enter — resolve
        if raw.is_empty() {
            let host = input.trim().to_string();
            if !host.is_empty() {
                println!("@supervisor: dns-resolve {host}");
                let _ = io::stdout().flush();
                pending = true;
                input.clear();
                draw(&input, &results, pending);
            }
            continue;
        }

        // Backspace
        if raw == "\x7f" {
            input.pop();
            draw(&input, &results, pending);
            continue;
        }

        // Printable char
        if raw.len() == 1 {
            let ch = raw.chars().next().unwrap();
            if ch.is_ascii_graphic() || ch == '-' || ch == '.' {
                input.push(ch);
                draw(&input, &results, pending);
            }
        }
    }
}
