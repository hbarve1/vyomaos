//! VyomaOS GUI demo — live system dashboard
//!
//! Runs in a continuous loop, querying the supervisor every 2 seconds via
//! `@supervisor: ps-raw` and rendering a live grid of app status cards.
//!
//! Each card shows: status dot (green=running, red=stopped), name, uptime,
//! and restart count.  The dashboard redraws only its half of the screen
//! (Y=0..440) so the shell panel below is undisturbed.

use std::io::{BufRead, BufReader, Write};
use std::thread;
use std::time::Duration;

const W:      u32 = 1440;
const DASH_H: u32 = 440;   // dashboard height — shell panel starts at Y=450

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x161B22FF;
const C_PANEL:  u32 = 0x161B22FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_RED:    u32 = 0xF78166FF;
const C_YELLOW: u32 = 0xE3B341FF;
const C_WHITE:  u32 = 0xFFFFFFFF;
const C_DIM:    u32 = 0x8B949EFF;

struct AppInfo {
    name:     String,
    running:  bool,
    uptime:   u64,
    restarts: u32,
}

fn main() {
    let boot_count = read_boot_count();
    let stdin = std::io::stdin();
    let mut stdin_lines = BufReader::new(stdin).lines();
    let mut refresh: u64 = 0;

    loop {
        // Request live status from supervisor
        println!("@supervisor: ps-raw");
        let _ = std::io::stdout().flush();

        // Read reply — skip any non-REPLY lines (shouldn't normally appear)
        let apps = loop {
            match stdin_lines.next() {
                Some(Ok(line)) if line.starts_with("REPLY:") => {
                    break parse_ps(&line);
                }
                Some(Ok(_)) => continue,
                Some(Err(_)) | None => break vec![],
            }
        };

        draw(&apps, boot_count, refresh);
        refresh += 1;

        thread::sleep(Duration::from_secs(2));
    }
}

// ── Parse `REPLY:name:status:uptime:restarts|...` ─────────────────────────────

fn parse_ps(line: &str) -> Vec<AppInfo> {
    let data = match line.strip_prefix("REPLY:") {
        Some(d) if !d.is_empty() => d,
        _ => return vec![],
    };
    let mut apps: Vec<AppInfo> = data
        .split('|')
        .filter_map(|entry| {
            let p: Vec<&str> = entry.splitn(4, ':').collect();
            if p.len() == 4 {
                Some(AppInfo {
                    name:     p[0].trim().to_string(),
                    running:  p[1].trim() == "run",
                    uptime:   p[2].trim().parse().unwrap_or(0),
                    restarts: p[3].trim().parse().unwrap_or(0),
                })
            } else {
                None
            }
        })
        .collect();
    apps.sort_by(|a, b| a.name.cmp(&b.name));
    apps
}

// ── Dashboard renderer ────────────────────────────────────────────────────────

fn draw(apps: &[AppInfo], boot_count: u64, refresh: u64) {
    let running = apps.iter().filter(|a| a.running).count();
    let total   = apps.len();

    // ── Background (dashboard area only) ─────────────────────────────────────
    fill(0, 0, W, DASH_H, C_BG);

    // ── Header bar ────────────────────────────────────────────────────────────
    fill(0, 0, W, 52, C_HEADER);
    fill(0, 52, W, 3, C_ACCENT);
    fill(16, 10, 32, 32, C_GREEN);              // logo block
    text(58, 18, C_WHITE, "VyomaOS");
    text(W - 210, 18, C_DIM, &format!("boot #{}", boot_count));

    // ── Section label ─────────────────────────────────────────────────────────
    text(32, 62, C_DIM,
        &format!("{}/{} apps running  |  refresh #{}", running, total, refresh));

    // ── App grid (4 columns) ──────────────────────────────────────────────────
    const COLS:   u32 = 4;
    const GAP:    u32 = 10;
    const CARD_H: u32 = 80;
    const GRID_Y: u32 = 82;
    let card_w: u32 = (W - GAP * (COLS + 1)) / COLS;

    for (i, app) in apps.iter().enumerate() {
        let col = (i as u32) % COLS;
        let row = (i as u32) / COLS;

        let cx = GAP + col * (card_w + GAP);
        let cy = GRID_Y + row * (CARD_H + GAP);

        if cy + CARD_H > DASH_H { break; } // guard against overflow

        // Card background
        fill(cx, cy, card_w, CARD_H, C_PANEL);

        // Status indicator dot
        let dot = if app.running { C_GREEN } else { C_RED };
        fill(cx + 8, cy + 14, 10, 10, dot);

        // App name
        let name_col = if app.running { C_WHITE } else { C_DIM };
        text(cx + 24, cy + 12, name_col, &app.name);

        // Uptime / status
        let sub = if app.running {
            format!("up {}s", app.uptime)
        } else {
            "stopped".to_string()
        };
        text(cx + 24, cy + 32, C_DIM, &sub);

        // Restart count (shown only if > 0)
        if app.restarts > 0 {
            text(cx + 24, cy + 52, C_YELLOW, &format!("restarts: {}", app.restarts));
        }
    }

    // ── Footer ────────────────────────────────────────────────────────────────
    let fy = DASH_H - 26;
    fill(0, fy - 2, W, 2, 0x30363DFF);
    fill(16, fy, 10, 10, C_GREEN);
    text(34, fy, C_DIM,
        "supervisor: Rust musl PID 1  |  runtime: Wasmtime WASI P2  |  live 2s refresh");

    flush_draw();
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn read_boot_count() -> u64 {
    std::fs::read_to_string("/data/boot_count.txt")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

#[inline] fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}
#[inline] fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},{s}");
}
#[inline] fn flush_draw() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}
