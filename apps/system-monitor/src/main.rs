//! VyomaOS system monitor (P26)
//!
//! Polls `@supervisor: ps-raw` every second.
//! Renders a full-screen table: app name | status | uptime | restarts | watchdog.
//! Press q or Ctrl+C to quit (returns keyboard focus to shell).

use std::io::{BufRead, Write};
use std::thread;
use std::time::Duration;

// ── Layout ────────────────────────────────────────────────────────────────────

const W: u32 = 1440;
const H: u32 = 900;
const PX: u32 = 32;
const PY: u32 = 32;
const PW: u32 = W - 64;
const PH: u32 = H - 64;
const TITLE_H: u32 = 28;
const HDR_H: u32 = 22;     // column header row
const STATUS_H: u32 = 20;
const ROW_H: u32 = 22;
const INNER_X: u32 = PX + 14;
const INNER_Y: u32 = PY + TITLE_H + HDR_H + 4;
const INNER_W: u32 = PW - 28;
const INNER_H: u32 = PH - TITLE_H - HDR_H - STATUS_H - 8;
const MAX_ROWS: usize = (INNER_H / ROW_H) as usize;

// Column X offsets (relative to INNER_X)
const COL_NAME: u32 = 0;
const COL_STATUS: u32 = 280;
const COL_UPTIME: u32 = 400;
const COL_RESTARTS: u32 = 580;
const COL_WATCHDOG: u32 = 720;

// ── Colours ───────────────────────────────────────────────────────────────────

const C_BG: u32 = 0x0D1117FF;
const C_PANEL: u32 = 0x161B22FF;
const C_TITLE: u32 = 0x21262DFF;
const C_HDR: u32 = 0x1C2128FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_WHITE: u32 = 0xFFFFFFFF;
const C_DIM: u32 = 0x8B949EFF;
const C_GREEN: u32 = 0x3FB950FF;
const C_RED: u32 = 0xF85149FF;
const C_YELLOW: u32 = 0xE3B341FF;
const C_ALT_ROW: u32 = 0x161B22FF;  // alternating row tint
const C_ROW: u32 = 0x0D1117FF;

// ── App entry parsed from ps-raw ──────────────────────────────────────────────

#[derive(Default, Clone)]
struct AppRow {
    name: String,
    running: bool,
    uptime: u64,
    restarts: u64,
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let mut apps: Vec<AppRow> = Vec::new();
    let mut tick: u64 = 0;

    // Initial draw with empty state
    draw_monitor(&apps, tick);

    // Send first poll immediately
    query_ps();

    let stdin = std::io::stdin();
    let mut lines_iter = stdin.lock().lines();

    loop {
        // Non-blocking: try to read stdin. Since supervisor sends REPLY shortly
        // after our query, the next().unwrap_or yields the reply or keyboard input.
        // We block here briefly; sleep happens after processing.
        match lines_iter.next() {
            None => break,
            Some(Err(_)) => break,
            Some(Ok(line)) => {
                match line.as_str() {
                    "q" | "\x03" => break,
                    _ if line.starts_with("REPLY:") => {
                        apps = parse_ps_raw(line.trim_start_matches("REPLY:"));
                        tick += 1;
                        draw_monitor(&apps, tick);
                        // Sleep 1 second before next poll
                        thread::sleep(Duration::from_secs(1));
                        query_ps();
                    }
                    _ => {}
                }
            }
        }
    }
}

// ── Supervisor query ──────────────────────────────────────────────────────────

fn query_ps() {
    println!("@supervisor: ps-raw");
    let _ = std::io::stdout().flush();
}

// ── Parser ────────────────────────────────────────────────────────────────────

fn parse_ps_raw(data: &str) -> Vec<AppRow> {
    // Format: name:status:uptime_secs:restart_count per entry, separated by |
    let mut rows: Vec<AppRow> = data
        .split('|')
        .filter(|s| !s.is_empty())
        .map(|entry| {
            let parts: Vec<&str> = entry.splitn(4, ':').collect();
            AppRow {
                name: parts.first().unwrap_or(&"?").to_string(),
                running: parts.get(1).unwrap_or(&"") == &"run",
                uptime: parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0),
                restarts: parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0),
            }
        })
        .collect();
    rows.sort_by(|a, b| a.name.cmp(&b.name));
    rows
}

// ── Drawing ───────────────────────────────────────────────────────────────────

fn draw_monitor(apps: &[AppRow], tick: u64) {
    fill(0, 0, W, H, C_BG);
    fill(PX, PY, PW, PH, C_PANEL);

    // Title bar
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_ACCENT);
    text(PX + 10, PY + 6, C_ACCENT, "System Monitor");
    let running = apps.iter().filter(|a| a.running).count();
    let total = apps.len();
    text(
        PX + 160,
        PY + 6,
        C_DIM,
        &format!("{running}/{total} running"),
    );
    text(
        PX + PW - 140,
        PY + 6,
        C_DIM,
        &format!("tick #{tick}   q quit"),
    );
    border(PX, PY, PW, PH, C_ACCENT);

    // Column headers
    let hy = PY + TITLE_H + 2;
    fill(INNER_X, hy, INNER_W, HDR_H, C_HDR);
    fill(INNER_X, hy + HDR_H - 1, INNER_W, 1, C_ACCENT);
    col_header("APP NAME", COL_NAME, hy);
    col_header("STATUS", COL_STATUS, hy);
    col_header("UPTIME", COL_UPTIME, hy);
    col_header("RESTARTS", COL_RESTARTS, hy);
    col_header("NOTE", COL_WATCHDOG, hy);

    // App rows
    clear_region(INNER_X, INNER_Y, INNER_W, INNER_H);

    if apps.is_empty() {
        text(INNER_X + INNER_W / 2 - 60, INNER_Y + INNER_H / 2 - 8, C_DIM, "Waiting for data…");
        flush_draw();
        return;
    }

    for (i, app) in apps.iter().take(MAX_ROWS).enumerate() {
        let ry = INNER_Y + i as u32 * ROW_H;
        let bg = if i % 2 == 0 { C_ROW } else { C_ALT_ROW };
        fill(INNER_X, ry, INNER_W, ROW_H, bg);

        // Name
        let name_color = if app.running { C_WHITE } else { C_DIM };
        text(INNER_X + COL_NAME + 4, ry + 3, name_color, &app.name);

        // Status pill
        let (status_text, status_color) = if app.running {
            ("● running", C_GREEN)
        } else {
            ("○ stopped", C_RED)
        };
        text(INNER_X + COL_STATUS + 4, ry + 3, status_color, status_text);

        // Uptime
        let uptime_str = fmt_uptime(app.uptime);
        text(INNER_X + COL_UPTIME + 4, ry + 3, C_DIM, &uptime_str);

        // Restarts — highlight in yellow if > 0
        let restart_color = if app.restarts > 0 { C_YELLOW } else { C_DIM };
        text(
            INNER_X + COL_RESTARTS + 4,
            ry + 3,
            restart_color,
            &app.restarts.to_string(),
        );

        // Note column: flag high-restart apps
        if app.restarts >= 5 {
            text(INNER_X + COL_WATCHDOG + 4, ry + 3, C_YELLOW, "⚠ unstable");
        } else if !app.running && app.restarts == 0 {
            text(INNER_X + COL_WATCHDOG + 4, ry + 3, C_DIM, "exited clean");
        }
    }

    // Overflow indicator
    if apps.len() > MAX_ROWS {
        let oy = INNER_Y + MAX_ROWS as u32 * ROW_H + 4;
        text(INNER_X + 4, oy, C_DIM, &format!("… {} more apps", apps.len() - MAX_ROWS));
    }

    // Status bar
    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_TITLE);
    fill(PX, sy, PW, 1, 0x30363DFF);
    text(INNER_X, sy + 3, C_DIM, "Auto-refreshing every 1s   q or Ctrl+C to quit");

    flush_draw();
}

fn col_header(label: &str, col_x: u32, hy: u32) {
    text(INNER_X + col_x + 4, hy + 4, C_ACCENT, label);
}

fn fmt_uptime(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        format!("{}h {}m", secs / 3600, (secs % 3600) / 60)
    }
}

// ── VYOMA_DRAW helpers ────────────────────────────────────────────────────────

#[inline] fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}
#[inline] fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}
#[inline] fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba}");
}
#[inline] fn clear_region(x: u32, y: u32, w: u32, h: u32) {
    println!("VYOMA_DRAW:clear_region:{x},{y},{w},{h}");
}
#[inline] fn flush_draw() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}
