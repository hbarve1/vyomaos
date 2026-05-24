use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 1440;
const H: u32 = 28;
const C_BAR: u32    = 0x161B22FF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_BORDER: u32 = 0x30363DFF;

// Start epoch: 2026-05-21 09:00:00 (Thursday)
const START_H: u32 = 9;
const START_M: u32 = 0;
const START_S: u32 = 0;
const START_WDAY: u32 = 4; // 0=Sun, 4=Thu
const START_DAY:  u32 = 21;
const START_MON:  u32 = 5;
const START_YEAR: u32 = 2026;

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

const MONTH_SHORT: [&str; 12] = ["Jan","Feb","Mar","Apr","May","Jun","Jul","Aug","Sep","Oct","Nov","Dec"];
const DAY_SHORT:   [&str; 7]  = ["Sun","Mon","Tue","Wed","Thu","Fri","Sat"];

fn elapsed_to_time(secs: u64) -> (u32, u32, u32, u32, u32, u32, u32) {
    let total_days = secs / 86400;
    let rem = secs % 86400;
    let h = START_H + (rem / 3600) as u32;
    let m = START_M + ((rem % 3600) / 60) as u32;
    let s = START_S + (rem % 60) as u32;
    // Normalise
    let (h, m, s) = {
        let ts = h * 3600 + m * 60 + s;
        ((ts / 3600) % 24, (ts / 60) % 60, ts % 60)
    };
    let wday = (START_WDAY as u64 + total_days) % 7;
    // Simple day advance
    let mut day = START_DAY;
    let mut mon = START_MON;
    let mut yr  = START_YEAR;
    let mut d   = total_days;
    while d > 0 {
        let dim = match mon {
            1|3|5|7|8|10|12 => 31,
            4|6|9|11        => 30,
            2 => if yr % 4 == 0 { 29 } else { 28 },
            _ => 30,
        };
        if day + (d as u32) <= dim {
            day += d as u32;
            break;
        }
        d -= (dim - day + 1) as u64;
        day = 1; mon += 1;
        if mon > 12 { mon = 1; yr += 1; }
    }
    (h, m, s, wday as u32, day, mon, yr)
}

fn draw(elapsed: u64, focused_app: &str) {
    fill(0, 0, W, H, C_BAR);
    fill(0, H - 1, W, 1, C_BORDER);

    // Left: VyomaOS mark + focused app name
    text(10, 8, C_ACCENT, "V");
    text(22, 8, C_TEXT, "VyomaOS");
    if !focused_app.is_empty() && focused_app != "menu-bar" {
        let sep_x = 22 + 7 * 8 + 8;
        text(sep_x, 8, C_DIM, "|");
        text(sep_x + 12, 8, C_TEXT, focused_app);
    }

    // Right: status icons + clock + date
    let (h, m, _s, wday, day, mon, _yr) = elapsed_to_time(elapsed);
    let ampm   = if h < 12 { "AM" } else { "PM" };
    let h12    = match h % 12 { 0 => 12, x => x };
    let clock  = format!("{h12}:{m:02} {ampm}");
    let date   = format!("{} {} {}", DAY_SHORT[wday as usize], day, MONTH_SHORT[(mon-1) as usize]);

    // Right-to-left layout
    let clock_w = clock.len() as u32 * 8;
    let date_w  = date.len() as u32 * 8;
    let clock_x = W - clock_w - 12;
    let date_x  = clock_x - date_w - 16;

    text(clock_x, 8, C_TEXT, &clock);
    text(date_x,  8, C_DIM,  &date);

    // Status placeholders
    let icons_x = date_x - 80;
    text(icons_x,      8, C_DIM, "wifi");
    text(icons_x + 40, 8, C_DIM, "vol");

    flush();
}

fn main() {
    let stdin  = io::stdin();
    let boot   = Instant::now();
    let mut focused = String::new();

    draw(0, &focused);

    // Raise to front and start tick loop
    println!("@supervisor: raise menu-bar");
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:pong") || raw.starts_with("REPLY:ping") {
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
        }

        // Poll focused app from ps-raw
        if raw.starts_with("REPLY:ps-raw:") || raw.starts_with("REPLY:ps-raw ") {
            // parse focused = first app that is "running" (simplification: just use first entry)
            let payload = raw.trim_start_matches("REPLY:ps-raw:").trim_start_matches("REPLY:ps-raw ");
            if let Some(first) = payload.split('|').next() {
                if let Some(name) = first.split(':').next() {
                    let name = name.trim();
                    if !name.is_empty() { focused = name.to_string(); }
                }
            }
        }

        // Periodically request ps-raw to know focused app
        if raw.starts_with("REPLY:pong") {
            println!("@supervisor: ps-raw");
            let _ = io::stdout().flush();
        }

        let elapsed = boot.elapsed().as_secs();
        draw(elapsed, &focused);
    }
}
