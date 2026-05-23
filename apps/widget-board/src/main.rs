use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 400;
const H: u32 = 500;
const C_BG: u32     = 0x161B22F0;
const C_CARD: u32   = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_SEL_BG: u32 = 0x1F4068FF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;

const CARD_H: u32   = 108;
const CARD_GAP: u32 = 6;
const CARD_X: u32   = 8;
const CARD_W: u32   = W - 16;

const QUICK_APPS: &[(&str, &str, u32)] = &[
    ("Shell",     "shell",     0x3FB950FF),
    ("Browser",   "browser",   0x58A6FFFF),
    ("Settings",  "settings",  0xFFA657FF),
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

fn card_y(idx: u32) -> u32 {
    8 + idx * (CARD_H + CARD_GAP)
}

fn elapsed_to_hms(secs: u64) -> (u64, u64, u64) {
    ((secs / 3600 + 9) % 24, (secs / 60) % 60, secs % 60)
}

fn draw_clock(cy: u32, elapsed: u64, selected: bool) {
    let bc = if selected { C_SEL } else { C_BORDER };
    fill(CARD_X, cy, CARD_W, CARD_H, C_CARD);
    border(CARD_X, cy, CARD_W, CARD_H, bc);
    if selected { fill(CARD_X, cy, 3, CARD_H, C_SEL); }

    text(CARD_X + 12, cy + 8, C_HINT, "Clock");
    let (h, m, s) = elapsed_to_hms(elapsed);
    let time_str = format!("{:02}:{:02}:{:02}", h, m, s);
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let day_idx = (4 + elapsed / 86400) % 7;
    let date_str = format!("{} 2026-05-21", days[day_idx as usize]);
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,{time_str}", CARD_X + 12, cy + 28, C_TEXT);
    text(CARD_X + 12, cy + 72, C_DIM, &date_str);
}

fn draw_stats(cy: u32, app_count: usize, total_restarts: u32, selected: bool) {
    let bc = if selected { C_SEL } else { C_BORDER };
    fill(CARD_X, cy, CARD_W, CARD_H, C_CARD);
    border(CARD_X, cy, CARD_W, CARD_H, bc);
    if selected { fill(CARD_X, cy, 3, CARD_H, C_SEL); }

    text(CARD_X + 12, cy + 8, C_HINT, "System");
    let ac_str = format!("{} apps running", app_count);
    text(CARD_X + 12, cy + 32, C_GREEN, &ac_str);
    let rs_str = format!("{} total restarts", total_restarts);
    text(CARD_X + 12, cy + 56, C_DIM, &rs_str);
    text(CARD_X + 12, cy + 80, C_HINT, "VyomaOS is healthy");
}

fn draw_quick_launch(cy: u32, ql_cursor: usize, selected: bool) {
    let bc = if selected { C_SEL } else { C_BORDER };
    fill(CARD_X, cy, CARD_W, CARD_H, C_CARD);
    border(CARD_X, cy, CARD_W, CARD_H, bc);
    if selected { fill(CARD_X, cy, 3, CARD_H, C_SEL); }

    text(CARD_X + 12, cy + 8, C_HINT, "Quick Launch");

    let btn_w = (CARD_W - 24) / 3;
    for (i, &(name, _, color)) in QUICK_APPS.iter().enumerate() {
        let bx = CARD_X + 12 + i as u32 * (btn_w + 6);
        let by = cy + 30;
        let is_ql_sel = selected && i == ql_cursor;
        let bg = if is_ql_sel { C_SEL_BG } else { 0x30363DFF };
        fill(bx, by, btn_w, 52, bg);
        border(bx, by, btn_w, 52, if is_ql_sel { C_SEL } else { C_BORDER });
        fill(bx + btn_w / 2 - 8, by + 6, 16, 16, color);
        let nw = name.len() as u32 * 8;
        text(bx + (btn_w - nw.min(btn_w)) / 2, by + 30, if is_ql_sel { C_TEXT } else { C_DIM }, name);
    }
}

fn draw_notes(cy: u32, selected: bool) {
    let bc = if selected { C_SEL } else { C_BORDER };
    fill(CARD_X, cy, CARD_W, CARD_H, C_CARD);
    border(CARD_X, cy, CARD_W, CARD_H, bc);
    if selected { fill(CARD_X, cy, 3, CARD_H, C_SEL); }

    text(CARD_X + 12, cy + 8, C_HINT, "Notes");
    text(CARD_X + 12, cy + 30, C_TEXT, "VyomaOS macOS-like desktop");
    text(CARD_X + 12, cy + 50, C_DIM,  "90+ WASM apps running");
    text(CARD_X + 12, cy + 70, C_DIM,  "Kernel: Linux 5.10 + Wasmtime 43");
    text(CARD_X + 12, cy + 88, C_HINT, "Open source — contribute!");
}

fn draw(elapsed: u64, widget_cursor: usize, ql_cursor: usize, apps: &[String]) {
    fill(0, 0, W, H, C_BG);

    let app_count = apps.len();
    let total_restarts: u32 = 0; // ps-raw doesn't expose total in this view

    draw_clock(card_y(0), elapsed, widget_cursor == 0);
    draw_stats(card_y(1), app_count, total_restarts, widget_cursor == 1);
    draw_quick_launch(card_y(2), ql_cursor, widget_cursor == 2);
    draw_notes(card_y(3), widget_cursor == 3);

    // Footer hint
    let hint = if widget_cursor == 2 {
        "←→: select app  Enter: launch  ↑↓: widget  Esc: close"
    } else {
        "↑↓: widget  Esc: close"
    };
    text(CARD_X, H - 14, C_HINT, hint);

    flush();
}

fn parse_apps(reply: &str) -> Vec<String> {
    let payload = reply
        .trim_start_matches("REPLY:ps-raw:")
        .trim_start_matches("REPLY:ps-raw ");
    payload.split('|')
        .filter_map(|e| e.split(':').next().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn main() {
    let stdin = io::stdin();
    let start = Instant::now();
    let mut widget_cursor = 0usize;
    let mut ql_cursor = 0usize;
    let mut apps: Vec<String> = Vec::new();

    println!("@supervisor: ping");
    println!("@supervisor: ps-raw");
    let _ = io::stdout().flush();

    draw(0, widget_cursor, ql_cursor, &apps);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            let elapsed = start.elapsed().as_secs();
            draw(elapsed, widget_cursor, ql_cursor, &apps);
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:ps-raw") {
            apps = parse_apps(&raw);
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if widget_cursor > 0 { widget_cursor -= 1; }
                let elapsed = start.elapsed().as_secs();
                draw(elapsed, widget_cursor, ql_cursor, &apps);
            }
            "\x1b[B" => {
                if widget_cursor < 3 { widget_cursor += 1; }
                let elapsed = start.elapsed().as_secs();
                draw(elapsed, widget_cursor, ql_cursor, &apps);
            }
            "\x1b[D" => {
                if widget_cursor == 2 && ql_cursor > 0 { ql_cursor -= 1; }
                let elapsed = start.elapsed().as_secs();
                draw(elapsed, widget_cursor, ql_cursor, &apps);
            }
            "\x1b[C" => {
                if widget_cursor == 2 && ql_cursor + 1 < QUICK_APPS.len() { ql_cursor += 1; }
                let elapsed = start.elapsed().as_secs();
                draw(elapsed, widget_cursor, ql_cursor, &apps);
            }
            "" => {
                if widget_cursor == 2 {
                    let (_, cmd, _) = QUICK_APPS[ql_cursor];
                    println!("@supervisor: run {cmd}");
                    let _ = io::stdout().flush();
                }
                let elapsed = start.elapsed().as_secs();
                draw(elapsed, widget_cursor, ql_cursor, &apps);
            }
            _ => {}
        }
    }
}
