// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Lock screen app — full-screen overlay with PIN entry, lockout, and unlock IPC.

use std::io::{self, BufRead, Write};
use std::time::Instant;

const W: u32 = 1440;
const H: u32 = 900;
const C_BG: u32 = 0x0D1117FF;
const C_TEXT: u32 = 0xE6EDF3FF;
const C_DIM: u32 = 0x8B949EFF;
const C_HINT: u32 = 0x6E7681FF;
const C_SEL: u32 = 0x58A6FFFF;
const C_ERR: u32 = 0xFF5F57FF;
const C_INPUT_BG: u32 = 0x161B22FF;
const C_INPUT_BORDER: u32 = 0x30363DFF;

const DEFAULT_PIN: &str = "0000";
const MAX_ATTEMPTS: u32 = 3;
const LOCKOUT_SECS: u64 = 30;
const PIN_LENGTH: usize = 4;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}

fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}

fn text_large(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},l,{s}");
}

fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}

fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn elapsed_to_hms(secs: u64) -> (u64, u64, u64) {
    (secs / 3600 % 24, (secs / 60) % 60, secs % 60)
}

/// Read PIN from /data/settings.toml if it exists, otherwise return default.
fn read_pin() -> String {
    match std::fs::read_to_string("/data/settings.toml") {
        Ok(content) => {
            for line in content.lines() {
                let trimmed = line.trim();
                if let Some(val) = trimmed.strip_prefix("pin") {
                    let val = val.trim().strip_prefix('=').unwrap_or("").trim();
                    let val = val.trim_matches('"').trim();
                    if !val.is_empty() {
                        return val.to_string();
                    }
                }
            }
            DEFAULT_PIN.to_string()
        }
        Err(_) => DEFAULT_PIN.to_string(),
    }
}

/// Draw the padlock icon centered at (cx, cy).
fn draw_lock(cx: u32, cy: u32) {
    // Shackle (arch)
    border(cx - 20, cy - 36, 40, 30, C_SEL);
    fill(cx - 16, cy - 32, 32, 26, C_BG);
    // Body
    fill(cx - 28, cy - 8, 56, 44, C_SEL);
    border(cx - 28, cy - 8, 56, 44, C_TEXT);
    // Keyhole
    fill(cx - 6, cy + 4, 12, 12, C_BG);
    fill(cx - 3, cy + 14, 6, 10, C_BG);
}

/// Draw the PIN dots (filled for entered digits, hollow for remaining).
fn draw_pin_dots(pin_len: usize) {
    let dot_w: u32 = 24;
    let gap: u32 = 16;
    let total_w = PIN_LENGTH as u32 * dot_w + (PIN_LENGTH as u32 - 1) * gap;
    let start_x = (W - total_w) / 2;
    let y = H / 2 + 130;

    // Input field background
    let field_pad: u32 = 16;
    fill(
        start_x - field_pad,
        y - field_pad,
        total_w + field_pad * 2,
        dot_w + field_pad * 2,
        C_INPUT_BG,
    );
    border(
        start_x - field_pad,
        y - field_pad,
        total_w + field_pad * 2,
        dot_w + field_pad * 2,
        C_INPUT_BORDER,
    );

    for i in 0..PIN_LENGTH {
        let dx = start_x + i as u32 * (dot_w + gap);
        if i < pin_len {
            // Filled dot (entered digit)
            fill(dx + 4, y + 4, dot_w - 8, dot_w - 8, C_SEL);
        } else {
            // Hollow dot (not yet entered)
            border(dx + 4, y + 4, dot_w - 8, dot_w - 8, C_DIM);
        }
    }
}

struct LockState {
    pin_buf: String,
    correct_pin: String,
    attempts: u32,
    status_msg: String,
    status_color: u32,
    lockout_until: Option<Instant>,
}

impl LockState {
    fn new() -> Self {
        Self {
            pin_buf: String::new(),
            correct_pin: read_pin(),
            attempts: 0,
            status_msg: String::new(),
            status_color: C_DIM,
            lockout_until: None,
        }
    }

    fn is_locked_out(&self) -> bool {
        self.lockout_until
            .map(|t| Instant::now() < t)
            .unwrap_or(false)
    }

    fn lockout_remaining(&self) -> u64 {
        self.lockout_until
            .map(|t| {
                let now = Instant::now();
                if now < t {
                    (t - now).as_secs() + 1
                } else {
                    0
                }
            })
            .unwrap_or(0)
    }
}

/// Draw the full lock screen.
fn draw(elapsed: u64, state: &LockState) {
    fill(0, 0, W, H, C_BG);

    // Clock display
    let (h, m, s) = elapsed_to_hms(elapsed + 9 * 3600);
    let time_str = format!("{h:02}:{m:02}");
    let tw = time_str.len() as u32 * 24;
    let tx = (W - tw) / 2;
    text_large(tx, H / 2 - 160, C_TEXT, &time_str);

    // Seconds
    let sec_str = format!(":{s:02}");
    text(tx + tw + 4, H / 2 - 140, C_DIM, &sec_str);

    // Date line
    let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let total_days = elapsed / 86400;
    let wday = (3 + total_days) % 7;
    let date_str = format!(
        "{}, {} {:02}",
        days[wday as usize],
        months[4],
        21 + total_days % 10
    );
    let dw = date_str.len() as u32 * 8;
    text((W - dw) / 2, H / 2 - 90, C_DIM, &date_str);

    // Padlock icon
    draw_lock(W / 2, H / 2);

    // VyomaOS label
    let label = "VyomaOS";
    let lw = label.len() as u32 * 8;
    text((W - lw) / 2, H / 2 + 70, C_SEL, label);

    // PIN entry section
    let title = "Enter PIN to unlock";
    let title_w = title.len() as u32 * 8;
    text((W - title_w) / 2, H / 2 + 100, C_TEXT, title);

    // PIN dots
    if state.is_locked_out() {
        let remaining = state.lockout_remaining();
        let lockout_msg = format!("Locked out - wait {}s", remaining);
        let lw2 = lockout_msg.len() as u32 * 8;
        text((W - lw2) / 2, H / 2 + 142, C_ERR, &lockout_msg);
    } else {
        draw_pin_dots(state.pin_buf.len());
    }

    // Status message (wrong PIN, etc.)
    if !state.status_msg.is_empty() {
        let sw = state.status_msg.len() as u32 * 8;
        text((W - sw) / 2, H / 2 + 190, state.status_color, &state.status_msg);
    }

    // Hint
    let hint = "Enter 4-digit PIN, then press Enter";
    let hw = hint.len() as u32 * 8;
    text((W - hw) / 2, H - 60, C_HINT, hint);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let start = Instant::now();

    // Raise to front and request focus
    println!("@supervisor: raise screen-lock");
    let _ = io::stdout().flush();

    let mut state = LockState::new();
    draw(0, &state);

    // Kick off periodic refresh via ping
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle ping/pong for clock updates
        if raw == "REPLY:pong" || raw.starts_with("REPLY:pong ") {
            let elapsed = start.elapsed().as_secs();

            // Clear lockout if expired
            if state.is_locked_out() && state.lockout_remaining() == 0 {
                state.lockout_until = None;
                state.attempts = 0;
                state.status_msg.clear();
            }

            draw(elapsed, &state);
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }

        if raw.starts_with("REPLY:") {
            continue;
        }

        // Skip input during lockout
        if state.is_locked_out() {
            continue;
        }

        // Handle keyboard input
        if raw == "\x7f" || raw == "\x08" {
            // Backspace — remove last digit
            state.pin_buf.pop();
            state.status_msg.clear();
        } else if raw.is_empty() {
            // Enter key — verify PIN
            if state.pin_buf.len() == PIN_LENGTH {
                if state.pin_buf == state.correct_pin {
                    // Correct PIN — unlock
                    println!("@supervisor: unlock");
                    let _ = io::stdout().flush();
                    // Clear screen and exit
                    fill(0, 0, W, H, C_BG);
                    flush();
                    std::process::exit(0);
                } else {
                    // Wrong PIN
                    state.attempts += 1;
                    state.pin_buf.clear();
                    if state.attempts >= MAX_ATTEMPTS {
                        state.status_msg = format!("Too many attempts. Locked for {LOCKOUT_SECS}s");
                        state.status_color = C_ERR;
                        state.lockout_until = Some(Instant::now()
                            + std::time::Duration::from_secs(LOCKOUT_SECS));
                    } else {
                        let remaining = MAX_ATTEMPTS - state.attempts;
                        state.status_msg =
                            format!("Wrong PIN. {} attempt(s) remaining", remaining);
                        state.status_color = C_ERR;
                    }
                }
            } else {
                state.status_msg = format!("Enter {} digits", PIN_LENGTH);
                state.status_color = C_HINT;
            }
        } else if raw.len() == 1 {
            let ch = raw.chars().next().unwrap();
            if ch.is_ascii_digit() && state.pin_buf.len() < PIN_LENGTH {
                state.pin_buf.push(ch);
                state.status_msg.clear();
            }
        }

        let elapsed = start.elapsed().as_secs();
        draw(elapsed, &state);
    }
}
