use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 700;
const HEADER_H: u32 = 40;
const ROW_H: u32    = 32;
const STATUS_H: u32 = 28;
const BAR_W: u32    = 160;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_DIM: u32     = 0x8B949EFF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_YELLOW: u32  = 0xFFA657FF;
const C_RED: u32     = 0xFF7B72FF;
const C_ALT: u32     = 0x161B22FF;

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

struct AppRow {
    name:     String,
    status:   String,
    uptime:   u64,
    restarts: u32,
    cpu_pct:  u32,  // 0-100 cosmetic estimate
    mem_kb:   u32,  // cosmetic estimate in KB
}

fn parse_apps(reply: &str) -> Vec<AppRow> {
    let payload = reply
        .trim_start_matches("REPLY:ps-raw:")
        .trim_start_matches("REPLY:ps-raw ");
    payload.split('|')
        .filter_map(|entry| {
            let mut fields = entry.split(':');
            let name     = fields.next()?.trim().to_string();
            let status   = fields.next().unwrap_or("running").trim().to_string();
            let uptime: u64 = fields.next().unwrap_or("0").trim().parse().unwrap_or(0);
            let restarts: u32 = fields.next().unwrap_or("0").trim().parse().unwrap_or(0);
            if name.is_empty() { return None; }

            // Pseudo-deterministic cosmetic estimates
            let h = name.bytes().fold(5381u32, |acc, b| acc.wrapping_mul(33).wrapping_add(b as u32));
            let cpu_pct = (h % 30 + (uptime % 5) as u32).min(99);
            let mem_kb  = 1024 + (h % 8192);

            Some(AppRow { name, status, uptime, restarts, cpu_pct, mem_kb })
        })
        .collect()
}

#[derive(PartialEq)]
enum SortBy { Name, Cpu, Mem }

fn sort_apps(apps: &mut Vec<AppRow>, by: &SortBy) {
    match by {
        SortBy::Name => apps.sort_by(|a, b| a.name.cmp(&b.name)),
        SortBy::Cpu  => apps.sort_by(|a, b| b.cpu_pct.cmp(&a.cpu_pct)),
        SortBy::Mem  => apps.sort_by(|a, b| b.mem_kb.cmp(&a.mem_kb)),
    }
}

fn bar_color(pct: u32) -> u32 {
    if pct < 50 { C_GREEN } else if pct < 80 { C_YELLOW } else { C_RED }
}

fn draw_bar(x: u32, y: u32, pct: u32, max_w: u32) {
    border(x, y + 4, max_w, 16, C_BORDER);
    let filled = (pct * max_w / 100).max(1);
    fill(x + 1, y + 5, filled - 1, 14, bar_color(pct));
}

const COL_NAME: u32    = 12;
const COL_STATUS: u32  = 220;
const COL_UPTIME: u32  = 340;
const COL_RESTART: u32 = 460;
const COL_CPU: u32     = 560;
const COL_MEM: u32     = 740;

fn draw(apps: &[AppRow], cursor: usize, sort: &SortBy, scroll: usize) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);

    let sh = |s: &SortBy| if std::mem::discriminant(sort) == std::mem::discriminant(s) { C_SEL } else { C_DIM };
    text(COL_NAME,    10, sh(&SortBy::Name), "Process Name");
    text(COL_STATUS,  10, C_DIM,             "Status");
    text(COL_UPTIME,  10, C_DIM,             "Uptime");
    text(COL_RESTART, 10, C_DIM,             "Restarts");
    text(COL_CPU,     10, sh(&SortBy::Cpu),  "CPU %");
    text(COL_MEM,     10, sh(&SortBy::Mem),  "Memory");

    let visible = ((H - HEADER_H - STATUS_H) / ROW_H) as usize;
    let start = scroll;
    let end = (start + visible).min(apps.len());

    for (i, app) in apps[start..end].iter().enumerate() {
        let ry = HEADER_H + i as u32 * ROW_H;
        let is_sel = (start + i) == cursor;

        let bg = if is_sel { C_SEL_BG } else if i % 2 == 0 { C_BG } else { C_ALT };
        fill(0, ry, W, ROW_H, bg);
        if is_sel { fill(0, ry, 3, ROW_H, C_SEL); }

        let tc = if is_sel { C_TEXT } else { C_DIM };
        let name: String = app.name.chars().take(18).collect();
        text(COL_NAME,    ry + 8, C_TEXT, &name);
        text(COL_STATUS,  ry + 8, C_GREEN, &app.status);

        let uptime = format!("{}s", app.uptime);
        text(COL_UPTIME,  ry + 8, tc, &uptime);

        let restarts = format!("{}", app.restarts);
        text(COL_RESTART, ry + 8, if app.restarts > 0 { C_YELLOW } else { tc }, &restarts);

        let cpu = format!("{:2}%", app.cpu_pct);
        text(COL_CPU,     ry + 8, tc, &cpu);
        draw_bar(COL_CPU + 36, ry, app.cpu_pct, BAR_W);

        let mem_mb = app.mem_kb / 1024;
        let mem_str = if mem_mb > 0 { format!("{}M", mem_mb) } else { format!("{}K", app.mem_kb) };
        text(COL_MEM,     ry + 8, tc, &mem_str);
        let mem_pct = (app.mem_kb * 100 / (256 * 1024)).min(100);
        draw_bar(COL_MEM + 36, ry, mem_pct, BAR_W);
    }

    // Status bar
    fill(0, H - STATUS_H, W, STATUS_H, C_HEADER);
    fill(0, H - STATUS_H, W, 1, C_BORDER);

    let count = format!("{} processes running", apps.len());
    text(12, H - STATUS_H + 8, C_HINT, &count);
    text(W - 360, H - STATUS_H + 8, C_HINT, "n: sort name  c: sort CPU  m: sort mem  q: quit");

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut apps: Vec<AppRow> = Vec::new();
    let mut cursor = 0usize;
    let mut scroll = 0usize;
    let mut sort = SortBy::Name;
    let visible = ((H - HEADER_H - STATUS_H) / ROW_H) as usize;

    println!("@supervisor: ps-raw");
    println!("@supervisor: ping");
    println!("@supervisor: raise activity-monitor");
    let _ = io::stdout().flush();

    draw(&apps, cursor, &sort, scroll);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:ps-raw") {
            apps = parse_apps(&raw);
            sort_apps(&mut apps, &sort);
            if cursor >= apps.len() { cursor = apps.len().saturating_sub(1); }
            draw(&apps, cursor, &sort, scroll);
            continue;
        }
        if raw == "REPLY:pong" {
            println!("@supervisor: ps-raw");
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        let n = apps.len();
        match raw.as_str() {
            "q" | "\x03" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if cursor > 0 { cursor -= 1; }
                if cursor < scroll { scroll = cursor; }
                draw(&apps, cursor, &sort, scroll);
            }
            "\x1b[B" => {
                if cursor + 1 < n { cursor += 1; }
                if cursor >= scroll + visible { scroll = cursor + 1 - visible; }
                draw(&apps, cursor, &sort, scroll);
            }
            "n" | "N" => { sort = SortBy::Name; sort_apps(&mut apps, &sort); draw(&apps, cursor, &sort, scroll); }
            "c" | "C" => { sort = SortBy::Cpu;  sort_apps(&mut apps, &sort); draw(&apps, cursor, &sort, scroll); }
            "m" | "M" => { sort = SortBy::Mem;  sort_apps(&mut apps, &sort); draw(&apps, cursor, &sort, scroll); }
            _ => {}
        }
    }
}
