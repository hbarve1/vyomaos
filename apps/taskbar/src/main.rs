//! VyomaOS taskbar
//!
//! Full-width dock at the bottom of the screen (y=860, h=40).
//! Shows: brand label on left | running app buttons in center | elapsed clock on right.
//! Polls @supervisor: ps-raw every 2s to refresh the app list.
//! Left-clicking an app button sends @supervisor: focus + raise for that app.

use std::io::{BufRead, Write};
use std::thread;
use std::time::{Duration, Instant};

// ── Local window geometry (0,0 = top-left of this 1440×40 window) ─────────────

const W:          u32 = 1440;
const H:          u32 = 40;
const BTN_START:  u32 = 180;   // first app button left edge
const BTN_W:      u32 = 100;   // button width
const BTN_PITCH:  u32 = 104;   // button stride (width + 4px gap)
const BTN_MAX_X:  u32 = 1320;  // right boundary; leave room for clock
const CLOCK_X:    u32 = 1336;  // clock text left edge (8 chars × 8px = 64px → fits to 1400)

// ── Colours ───────────────────────────────────────────────────────────────────

const C_BG:     u32 = 0x161B22FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_BTN:    u32 = 0x21262DFF;
const C_HOVER:  u32 = 0x30363DFF;
const C_WHITE:  u32 = 0xFFFFFFFF;
const C_DIM:    u32 = 0x8B949EFF;

fn main() {
    let start = Instant::now();
    let mut apps: Vec<String> = Vec::new();

    // Initial draw then request the first app list
    draw(&apps, start.elapsed().as_secs());
    println!("@supervisor: ps-raw");
    let _ = std::io::stdout().flush();

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw { Ok(l) => l, Err(_) => break };

        if let Some(reply) = raw.strip_prefix("REPLY:") {
            apps = parse_ps_raw(reply);
            draw(&apps, start.elapsed().as_secs());
            // Sleep then issue the next poll
            thread::sleep(Duration::from_secs(2));
            println!("@supervisor: ps-raw");
            let _ = std::io::stdout().flush();

        } else if let Some(data) = raw.strip_prefix("VYOMA_INPUT:mouse:") {
            // data: lx,ly,btn
            let mut parts = data.split(',');
            let lx: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let _ly: u32 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let btn: u8  = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

            if btn == 1 && lx >= BTN_START {
                let idx = ((lx - BTN_START) / BTN_PITCH) as usize;
                if idx < apps.len() {
                    let name = &apps[idx];
                    println!("@supervisor: raise {name}");
                    println!("@supervisor: focus {name}");
                    let _ = std::io::stdout().flush();
                }
            }
        }
    }
}

/// Parse ps-raw reply (name:status:uptime:restarts|...) into sorted app names.
/// Excludes "taskbar" itself and any empty entries.
fn parse_ps_raw(reply: &str) -> Vec<String> {
    let mut names: Vec<String> = reply
        .split('|')
        .filter_map(|entry| {
            let name = entry.split(':').next()?.trim();
            if name.is_empty() || name == "taskbar" {
                return None;
            }
            Some(name.to_string())
        })
        .collect();
    names.sort_unstable();
    names
}

/// Redraw the entire taskbar.
fn draw(apps: &[String], elapsed_secs: u64) {
    // Background fill
    fill(0, 0, W, H, C_BG);
    // Top accent line
    fill(0, 0, W, 2, C_ACCENT);

    // Brand label
    text(8, 12, C_ACCENT, "VyomaOS");

    // Vertical separator before app buttons
    fill(BTN_START - 8, 6, 1, H - 12, C_DIM);

    // App buttons — each 100×28 px, truncate name to 9 chars
    for (i, name) in apps.iter().enumerate() {
        let bx = BTN_START + i as u32 * BTN_PITCH;
        if bx + BTN_W > BTN_MAX_X {
            break;
        }
        fill(bx, 6, BTN_W, H - 12, C_BTN);
        // Rounded feel: 1px lighter border on top
        fill(bx, 6, BTN_W, 1, C_HOVER);
        let label: String = name.chars().take(9).collect();
        text(bx + 6, 12, C_WHITE, &label);
    }

    // Vertical separator before clock
    fill(BTN_MAX_X + 4, 6, 1, H - 12, C_DIM);

    // Elapsed clock  HH:MM:SS
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;
    let clock = format!("{h:02}:{m:02}:{s:02}");
    text(CLOCK_X, 12, C_DIM, &clock);

    flush();
}

// ── VYOMA_DRAW protocol helpers ───────────────────────────────────────────────

#[inline]
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}

#[inline]
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}

#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}
