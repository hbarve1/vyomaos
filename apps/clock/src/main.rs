// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 320;
const H: u32 = 360;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;

// Start epoch: 2026-05-20 00:00:00 (Wednesday)
const START_HOUR:   u32 = 0;
const START_MIN:    u32 = 0;
const START_SEC:    u32 = 0;
const START_DAY:    u32 = 20;
const START_MONTH:  u32 = 5;
const START_YEAR:   u32 = 2026;
const START_WDAY:   u32 = 3; // 0=Sun, 3=Wed

const MONTH_NAMES: [&str; 12] = [
    "January","February","March","April","May","June",
    "July","August","September","October","November","December",
];
const DAY_NAMES: [&str; 7] = ["Sunday","Monday","Tuesday","Wednesday","Thursday","Friday","Saturday"];

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

fn is_leap(y: u32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(y) { 29 } else { 28 },
        _ => 30,
    }
}

struct DateTime {
    year: u32, month: u32, day: u32,
    hour: u32, min: u32, sec: u32,
    wday: u32, // 0=Sun
}

impl DateTime {
    fn start() -> Self {
        DateTime {
            year: START_YEAR, month: START_MONTH, day: START_DAY,
            hour: START_HOUR, min: START_MIN, sec: START_SEC,
            wday: START_WDAY,
        }
    }

    fn from_elapsed(elapsed_secs: u64) -> Self {
        let mut dt = DateTime::start();
        let mut remaining = elapsed_secs;

        // Add full days
        let full_days = remaining / 86400;
        remaining %= 86400;

        dt.hour = (remaining / 3600) as u32;
        remaining %= 3600;
        dt.min  = (remaining / 60)   as u32;
        dt.sec  = (remaining % 60)   as u32;

        // Advance date by full_days
        let mut days_left = full_days;
        while days_left > 0 {
            let dim = days_in_month(dt.year, dt.month);
            let remaining_in_month = dim - dt.day;
            if days_left <= remaining_in_month as u64 {
                dt.day += days_left as u32;
                days_left = 0;
            } else {
                days_left -= (remaining_in_month + 1) as u64;
                dt.month += 1;
                if dt.month > 12 { dt.month = 1; dt.year += 1; }
                dt.day = 1;
            }
        }
        // Recalculate weekday (simple: START_WDAY + total_days mod 7)
        dt.wday = ((START_WDAY as u64 + elapsed_secs / 86400) % 7) as u32;
        dt
    }
}

fn draw(dt: &DateTime) {
    fill(0, 0, W, H, C_BG);
    fill(0, 34, W, 1, C_BORDER);
    text(20, 14, C_ACCENT, "Clock");

    // Large time display — centered
    let time_str = format!("{:02}:{:02}:{:02}", dt.hour, dt.min, dt.sec);
    // Each large char is ~16px wide (size l = 16x32), time_str is 8 chars
    let time_w = time_str.len() as u32 * 16;
    let time_x = (W - time_w) / 2;
    println!("VYOMA_DRAW:draw_text:{time_x},100,{C_TITLE:#010x},l,{time_str}");

    // Date line — centered
    let date_str = format!("{} {} {} {}",
        DAY_NAMES[dt.wday as usize],
        dt.day,
        MONTH_NAMES[(dt.month - 1) as usize],
        dt.year
    );
    let date_w = date_str.len() as u32 * 8;
    let date_x = if date_w < W { (W - date_w) / 2 } else { 4 };
    text(date_x, 160, C_DIM, &date_str);

    // Divider
    fill(40, 195, W - 80, 1, C_BORDER);

    // AM/PM
    let ampm = if dt.hour < 12 { "AM" } else { "PM" };
    let h12  = match dt.hour % 12 { 0 => 12, h => h };
    let ampm_str = format!("{h12}:{:02} {ampm}", dt.min);
    let ampm_w = ampm_str.len() as u32 * 8;
    let ampm_x = (W - ampm_w) / 2;
    text(ampm_x, 210, C_ACCENT, &ampm_str);

    fill(0, H - 30, W, 1, C_BORDER);
    text(20, H - 18, C_HINT, "any key: tick 1s   Ctrl+C: quit");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let boot = Instant::now();
    let mut manual_ticks: u64 = 0;

    let dt = DateTime::from_elapsed(0);
    draw(&dt);

    // Request periodic ticks via supervisor ping
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:pong") || raw.starts_with("REPLY:ping") {
            // Re-ping for continuous ticks
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            _ => {}
        }

        // Use real elapsed time from boot + manual ticks as fallback
        let elapsed = boot.elapsed().as_secs() + manual_ticks;
        if raw.len() == 1 && raw.as_bytes().first() != Some(&b'\x03') {
            manual_ticks += 1;
        }
        let dt = DateTime::from_elapsed(elapsed);
        draw(&dt);
    }
}
