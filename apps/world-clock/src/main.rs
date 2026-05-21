use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 960;
const H: u32 = 500;
const COLS: u32 = 3;
const ROWS: u32 = 2;
const CARD_W: u32 = W / COLS;
const CARD_H: u32 = (H - 36) / ROWS;
const HEADER_H: u32 = 36;

const C_BG: u32      = 0x0D1117FF;
const C_CARD: u32    = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_SEL: u32     = 0x58A6FFFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_DIM: u32     = 0x8B949EFF;
const C_HINT: u32    = 0x6E7681FF;
const C_HEADER: u32  = 0x21262DFF;

// (name, label, utc_offset_minutes, dst_offset_minutes)
const ZONES: &[(&str, &str, i64, i64)] = &[
    ("UTC",           "UTC+0",     0,     0),
    ("US/Eastern",    "UTC-5 EST", -300,  0),
    ("US/Pacific",    "UTC-8 PST", -480,  0),
    ("Europe/London", "UTC+1 BST", 60,    0),
    ("Asia/Tokyo",    "UTC+9 JST", 540,   0),
    ("India/Kolkata", "UTC+5:30 IST", 330, 0),
];

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

fn utc_secs_to_hms(utc_secs: i64) -> (i64, i64, i64) {
    let s = utc_secs.rem_euclid(86400);
    (s / 3600, (s / 60) % 60, s % 60)
}

fn utc_to_date(utc_secs: i64) -> (i64, i64, i64) {
    // Base: 2026-05-21 09:00:00 UTC = day offset 0
    // days since base
    let day_offset = (utc_secs / 86400) as i64;
    let day_of_week = (3 + day_offset).rem_euclid(7); // Thu=4 base
    let day_of_month = 21 + day_offset % 30;
    (5, day_of_month, day_of_week) // month=May
}

fn draw(elapsed_secs: i64, cursor: usize) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    text(16, 10, C_TEXT, "World Clock");
    text(W - 180, 10, C_HINT, "↑↓←→: select  Esc: close");

    // Base UTC time: epoch 2026-05-21 09:00:00 + elapsed
    let base_utc: i64 = 9 * 3600 + elapsed_secs;

    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun",
                  "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    for (i, &(name, label, offset_min, _)) in ZONES.iter().enumerate() {
        let col = i as u32 % COLS;
        let row = i as u32 / COLS;
        let cx = col * CARD_W;
        let cy = HEADER_H + row * CARD_H;
        let is_sel = i == cursor;

        fill(cx + 1, cy + 1, CARD_W - 2, CARD_H - 2, C_CARD);
        let bc = if is_sel { C_SEL } else { C_BORDER };
        border(cx, cy, CARD_W, CARD_H, bc);
        if is_sel { fill(cx, cy, CARD_W, 3, C_SEL); }

        let zone_secs = base_utc + offset_min * 60;
        let (h, m, s) = utc_secs_to_hms(zone_secs);
        let (mon, dom, dow) = utc_to_date(zone_secs / 86400);
        let _ = mon;

        // Zone name
        text(cx + 12, cy + 12, if is_sel { C_SEL } else { C_DIM }, name);
        text(cx + 12, cy + 30, C_HINT, label);

        // Large time
        let time_str = format!("{:02}:{:02}:{:02}", h, m, s);
        println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,{time_str}", cx + 12, cy + 52, C_TEXT);

        // Date
        let date_str = format!("{} {} 2026", days[dow as usize], dom);
        text(cx + 12, cy + 76, C_DIM, &date_str);

        // AM/PM indicator
        let ampm = if h < 12 { "AM" } else { "PM" };
        text(cx + CARD_W - 36, cy + 52, C_HINT, ampm);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let start = Instant::now();
    let mut cursor = 0usize;

    println!("@supervisor: ping");
    println!("@supervisor: raise world-clock");
    let _ = io::stdout().flush();

    draw(0, cursor);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            let elapsed = start.elapsed().as_secs() as i64;
            draw(elapsed, cursor);
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        let n = ZONES.len();
        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[D" => { if cursor > 0 { cursor -= 1; } }
            "\x1b[C" => { if cursor + 1 < n { cursor += 1; } }
            "\x1b[A" => { if cursor >= COLS as usize { cursor -= COLS as usize; } }
            "\x1b[B" => { if cursor + COLS as usize < n { cursor += COLS as usize; } }
            _ => {}
        }
        let elapsed = start.elapsed().as_secs() as i64;
        draw(elapsed, cursor);
    }
}
