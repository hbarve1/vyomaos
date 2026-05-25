// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS GUI demo — live system dashboard
//!
//! Renders at 60 fps (16 ms frame budget) so the dashboard feels responsive.
//! App status is fetched from supervisor via `@supervisor: ps-raw` every 2 s;
//! between fetches the last known state is redrawn at full frame rate.
//!
//! Each card shows: status dot (green=running, red=stopped), name, uptime,
//! and restart count.  Mouse events accumulated in the stdin pipe are drained
//! on each fetch cycle to keep the cursor-hover highlight up to date.

use std::io::{BufRead, BufReader, Write};
use std::thread;
use std::time::{Duration, Instant};

const W:      u32 = 1440;
const DASH_H: u32 = 440;   // dashboard height; matches [window] h=440 in vyoma.toml

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x161B22FF;
const C_PANEL:  u32 = 0x161B22FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_RED:    u32 = 0xF78166FF;
const C_YELLOW: u32 = 0xE3B341FF;
const C_WHITE:  u32 = 0xFFFFFFFF;
const C_DIM:    u32 = 0x8B949EFF;
const C_HIGHLIGHT: u32 = 0x1C2E4AFF; // blue-tinted card background for hover

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
    let mut cursor_pos: Option<(u32, u32)> = None;
    let mut cached_apps: Vec<AppInfo> = vec![];

    // Trigger an immediate fetch on the first frame.
    let mut last_fetch = Instant::now() - Duration::from_secs(3);

    loop {
        let frame_start = Instant::now();

        // Fetch app status every 2 seconds; drain stdin for mouse events too.
        if last_fetch.elapsed() >= Duration::from_secs(2) {
            println!("@supervisor: ps-raw");
            let _ = std::io::stdout().flush();

            // Blocking read: drain accumulated mouse events then consume REPLY.
            loop {
                match stdin_lines.next() {
                    Some(Ok(line)) if line.starts_with("REPLY:") => {
                        cached_apps = parse_ps(&line);
                        break;
                    }
                    Some(Ok(line)) if line.starts_with("VYOMA_INPUT:mouse:") => {
                        cursor_pos = parse_mouse_event(&line).or(cursor_pos);
                    }
                    Some(Ok(_)) => {}
                    Some(Err(_)) | None => break,
                }
            }
            last_fetch = Instant::now();
        }

        draw(&cached_apps, boot_count, refresh, cursor_pos);
        refresh += 1;

        // Sleep for the remainder of the 16 ms frame budget (≈60 fps).
        let elapsed = frame_start.elapsed();
        let budget  = Duration::from_millis(16);
        if elapsed < budget {
            thread::sleep(budget - elapsed);
        }
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

fn draw(apps: &[AppInfo], boot_count: u64, refresh: u64, cursor_pos: Option<(u32, u32)>) {
    let running = apps.iter().filter(|a| a.running).count();
    let total   = apps.len();

    // ── Background (dashboard area only) ─────────────────────────────────────
    fill(0, 0, W, DASH_H, C_BG);

    // ── Header bar ────────────────────────────────────────────────────────────
    fill(0, 0, W, 64, C_HEADER);               // 52→64 to fit 16×32 large font
    fill(0, 64, W, 3, C_ACCENT);              // accent line moves down
    fill(16, 14, 32, 32, C_GREEN);             // logo block
    text_large(58, 16, C_WHITE, "VyomaOS");    // 16×32 glyph, y=16 centres in 64px bar
    text(W - 210, 24, C_DIM, &format!("boot #{}", boot_count));

    // ── Section label ─────────────────────────────────────────────────────────
    text(32, 74, C_DIM,
        &format!("{}/{} apps running  |  refresh #{}", running, total, refresh));

    // ── App grid (4 columns) ──────────────────────────────────────────────────
    const COLS:   u32 = 4;
    const GAP:    u32 = 10;
    const CARD_H: u32 = 80;
    const GRID_Y: u32 = 94;
    let card_w: u32 = (W - GAP * (COLS + 1)) / COLS;

    for (i, app) in apps.iter().enumerate() {
        let col = (i as u32) % COLS;
        let row = (i as u32) / COLS;

        let cx = GAP + col * (card_w + GAP);
        let cy = GRID_Y + row * (CARD_H + GAP);

        if cy + CARD_H > DASH_H { break; } // guard against overflow

        // Card background — highlight if cursor is over this card
        let hovered = cursor_pos
            .map(|(mx, my)| find_card_under_cursor(mx, my, apps.len()) == Some(i))
            .unwrap_or(false);
        let card_bg = if hovered { C_HIGHLIGHT } else { C_PANEL };
        fill(cx, cy, card_w, CARD_H, card_bg);

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

/// Parse `VYOMA_INPUT:mouse:move:<x>,<y>` or `VYOMA_INPUT:mouse:click:<x>,<y>:<btn>`
/// → local window coords `(x, y)`.
fn parse_mouse_event(line: &str) -> Option<(u32, u32)> {
    let rest = line.strip_prefix("VYOMA_INPUT:mouse:")?;
    let coords = rest.strip_prefix("move:")
        .or_else(|| rest.strip_prefix("click:"))?;
    let mut parts = coords.splitn(2, ',');
    let x: u32 = parts.next()?.parse().ok()?;
    // y field may have ":button" suffix (click format) — strip it
    let y_raw = parts.next()?;
    let y_str = y_raw.split(':').next()?;
    let y: u32 = y_str.parse().ok()?;
    Some((x, y))
}

/// Return the index of the app card at local cursor position (mx, my), or None.
fn find_card_under_cursor(mx: u32, my: u32, count: usize) -> Option<usize> {
    const COLS: u32   = 4;
    const GAP: u32    = 10;
    const CARD_H: u32 = 80;
    const GRID_Y: u32 = 94;
    let card_w: u32   = (W - GAP * (COLS + 1)) / COLS;
    for i in 0..count {
        let col = (i as u32) % COLS;
        let row = (i as u32) / COLS;
        let cx  = GAP + col * (card_w + GAP);
        let cy  = GRID_Y + row * (CARD_H + GAP);
        if cy + CARD_H > DASH_H { break; }
        if mx >= cx && mx < cx + card_w && my >= cy && my < cy + CARD_H {
            return Some(i);
        }
    }
    None
}

#[inline] fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}
#[inline] fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}
#[inline] fn text_large(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},l,{s}");
}
#[inline] fn flush_draw() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mouse_valid() {
        assert_eq!(parse_mouse_event("VYOMA_INPUT:mouse:move:100,200"), Some((100, 200)));
        assert_eq!(parse_mouse_event("VYOMA_INPUT:mouse:click:0,0:left"), Some((0, 0)));
        assert_eq!(parse_mouse_event("VYOMA_INPUT:mouse:click:50,75:right"), Some((50, 75)));
    }

    #[test]
    fn parse_mouse_invalid() {
        assert_eq!(parse_mouse_event("REPLY:something"), None);
        assert_eq!(parse_mouse_event("VYOMA_INPUT:mouse:move:abc,200"), None);
    }

    #[test]
    fn no_cards_no_highlight() {
        assert_eq!(find_card_under_cursor(100, 100, 0), None);
    }

    #[test]
    fn cursor_on_first_card() {
        // card 0: col=0, row=0 → cx=GAP=10, cy=GRID_Y=94
        // card_w = (1440 - 10*5) / 4 = 347
        // Inside: x in [10, 357), y in [94, 174)
        assert_eq!(find_card_under_cursor(15, 100, 1), Some(0));
        assert_eq!(find_card_under_cursor(356, 173, 1), Some(0));
    }

    #[test]
    fn cursor_in_gap_no_highlight() {
        // card 0 ends at x=357; card 1 starts at x=367; gap is [357, 367)
        assert_eq!(find_card_under_cursor(360, 100, 5), None);
    }

    #[test]
    fn cursor_above_grid() {
        assert_eq!(find_card_under_cursor(15, 90, 5), None);
    }

    #[test]
    fn cursor_on_second_card() {
        // card 1: col=1 → cx = 10 + 1*(347+10) = 367, cy=94
        assert_eq!(find_card_under_cursor(370, 100, 5), Some(1));
    }

    #[test]
    fn cursor_on_fifth_card_second_row() {
        // card 4: col=0, row=1 → cx=10, cy=94+(80+10)=184
        assert_eq!(find_card_under_cursor(15, 190, 5), Some(4));
    }
}
