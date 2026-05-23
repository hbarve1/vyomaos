use std::io::{self, BufRead, Write};

const W: u32 = 640;
const H: u32 = 520;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x1F4068FF;
const C_TODAY: u32  = 0x3FB950FF;
const C_HINT: u32   = 0x6E7681FF;
const C_FIELD: u32  = 0x21262DFF;
const C_SAT: u32    = 0xFF7B72FF;

const HEADER_Y: u32 = 50;
const GRID_X: u32   = 20;
const GRID_Y: u32   = 90;
const CELL_W: u32   = 84;
const CELL_H: u32   = 60;
const DAY_LABELS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

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

// Returns (year, month, day) for "today" — hardcoded to 2026-05-20 since
// WASM apps don't have access to system time without a clock capability.
fn today() -> (i32, u32, u32) {
    (2026, 5, 20)
}

fn is_leap(y: i32) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

fn days_in_month(y: i32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => if is_leap(y) { 29 } else { 28 },
        _ => 30,
    }
}

// Zeller-like: weekday of 1st of month (0=Sun … 6=Sat)
fn weekday_of_first(y: i32, m: u32) -> u32 {
    // Use Tomohiko Sakamoto's algorithm
    let t: [u32; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let mut yr = y;
    let mn = m;
    if mn < 3 { yr -= 1; }
    let d = 1u32;
    let val = (yr as u32)
        .wrapping_add((yr as u32) / 4)
        .wrapping_sub((yr as u32) / 100)
        .wrapping_add((yr as u32) / 400)
        .wrapping_add(t[(mn - 1) as usize])
        .wrapping_add(d);
    val % 7
}

const MONTH_NAMES: [&str; 12] = [
    "January","February","March","April","May","June",
    "July","August","September","October","November","December",
];

fn draw(year: i32, month: u32, sel_day: Option<u32>) {
    fill(0, 0, W, H, C_BG);

    // Header
    let hdr = format!("{}  {}", MONTH_NAMES[(month - 1) as usize], year);
    text(20, 14, C_ACCENT, "Calendar");
    fill(0, 34, W, 1, C_BORDER);
    // Center month/year
    let hdr_x = (W - hdr.len() as u32 * 8) / 2;
    text(hdr_x, HEADER_Y, C_TITLE, &hdr);

    // Day-of-week column headers
    for (col, lbl) in DAY_LABELS.iter().enumerate() {
        let cx = GRID_X + col as u32 * CELL_W + CELL_W / 2 - 12;
        let lc = if col == 0 || col == 6 { C_SAT } else { C_DIM };
        text(cx, GRID_Y - 18, lc, lbl);
    }
    fill(0, GRID_Y - 4, W, 1, C_BORDER);

    let first_wd = weekday_of_first(year, month);
    let dim = days_in_month(year, month);
    let (ty, tm, td) = today();

    let mut day = 1u32;
    let mut row = 0u32;
    loop {
        if day > dim { break; }
        for col in 0..7u32 {
            let cell_day_slot = row * 7 + col;
            if cell_day_slot < first_wd { continue; }
            if day > dim { break; }
            let cx = GRID_X + col * CELL_W;
            let cy = GRID_Y + row * CELL_H;

            let is_today = year == ty && month == tm && day == td;
            let is_sel   = sel_day == Some(day);

            if is_sel {
                fill(cx + 1, cy + 1, CELL_W - 2, CELL_H - 2, C_SEL);
            } else if is_today {
                fill(cx + 1, cy + 1, CELL_W - 2, CELL_H - 2, 0x0D2818FF);
            }

            border(cx, cy, CELL_W, CELL_H, C_BORDER);

            let num_str = format!("{day}");
            let tc = if is_today { C_TODAY } else if col == 0 || col == 6 { C_SAT } else if is_sel { C_TITLE } else { C_DIM };
            text(cx + 4, cy + 4, tc, &num_str);

            day += 1;
        }
        row += 1;
    }

    // Footer
    fill(0, H - 30, W, 1, C_BORDER);
    text(20, H - 18, C_HINT, "←→: prev/next month   ↑↓: prev/next month   Ctrl+C: quit");
    flush();
}

fn prev_month(y: i32, m: u32) -> (i32, u32) {
    if m == 1 { (y - 1, 12) } else { (y, m - 1) }
}

fn next_month(y: i32, m: u32) -> (i32, u32) {
    if m == 12 { (y + 1, 1) } else { (y, m + 1) }
}

fn main() {
    let stdin = io::stdin();
    let (ty, tm, _) = today();
    let mut year  = ty;
    let mut month = tm;
    let mut sel: Option<u32> = None;

    draw(year, month, sel);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            "\x1b[D" | "\x1b[A" => {
                let (ny, nm) = prev_month(year, month);
                year = ny; month = nm; sel = None;
                draw(year, month, sel);
            }
            "\x1b[C" | "\x1b[B" => {
                let (ny, nm) = next_month(year, month);
                year = ny; month = nm; sel = None;
                draw(year, month, sel);
            }
            _ => {}
        }
    }
}
