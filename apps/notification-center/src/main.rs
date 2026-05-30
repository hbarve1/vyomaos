// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Notification Center -- history viewer for system and app notifications.
//!
//! Reads `/data/notifications.log` (one JSON line per notification) and displays
//! a scrollable, selectable list newest-first. Listens on stdin for live
//! `VYOMA_SYSTEM:notification:<json>` events from the supervisor.

use std::io::{self, BufRead, Write};

const LOG_PATH: &str = "/data/notifications.log";

const W: u32 = 400;
const H: u32 = 600;

// Layout constants
const HEADER_H: u32 = 36;
const CARD_X: u32 = 8;
const CARD_W: u32 = W - CARD_X * 2;
const CARD_H: u32 = 72;
const CARD_GAP: u32 = 4;
const CARD_PITCH: u32 = CARD_H + CARD_GAP;
const LIST_TOP: u32 = HEADER_H + 4;
const FOOTER_H: u32 = 24;
const CLEAR_BTN_H: u32 = 32;

// Colors
const C_BG: u32 = 0x161B22F0;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32 = 0xE6EDF3FF;
const C_APP: u32 = 0x58A6FFFF;
const C_BODY: u32 = 0xC9D1D9FF;
const C_TIME: u32 = 0x8B949EFF;
const C_HINT: u32 = 0x6E7681FF;
const C_CARD: u32 = 0x1C2128FF;
const C_SEL: u32 = 0x1F6A4EFF;
const C_CLEAR_BG: u32 = 0x3D1F1FFF;
const C_CLEAR_SEL: u32 = 0x6E2B2BFF;
const C_CLEAR_TEXT: u32 = 0xFF6B6BFF;
const C_EMPTY: u32 = 0x484F58FF;
const C_FOOTER: u32 = 0x21262DFF;

/// A single notification entry.
struct Notification {
    title: String,
    body: String,
    app: String,
    time: String,
}

// -- Drawing helpers ----------------------------------------------------------

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

// -- Minimal JSON parsing (no serde dependency) -------------------------------

fn extract_json_field(json: &str, key: &str) -> String {
    let needle = format!("\"{}\"", key);
    let Some(pos) = json.find(&needle) else { return String::new() };
    let after = &json[pos + needle.len()..];
    let after = after.trim_start();
    let after = after.strip_prefix(':').unwrap_or(after).trim_start();
    if after.starts_with('"') {
        let inner = &after[1..];
        let end = inner.find('"').unwrap_or(inner.len());
        inner[..end].to_string()
    } else {
        let end = after.find([',', '}', ']']).unwrap_or(after.len());
        after[..end].trim().to_string()
    }
}

fn parse_notification(json: &str) -> Option<Notification> {
    let trimmed = json.trim();
    if !trimmed.starts_with('{') {
        return None;
    }
    let title = extract_json_field(trimmed, "title");
    if title.is_empty() {
        return None;
    }
    Some(Notification {
        title,
        body: extract_json_field(trimmed, "body"),
        app: extract_json_field(trimmed, "app"),
        time: extract_json_field(trimmed, "time"),
    })
}

fn notification_to_json(n: &Notification) -> String {
    let esc = |s: &str| -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    };
    format!(
        "{{\"title\":\"{}\",\"body\":\"{}\",\"app\":\"{}\",\"time\":\"{}\"}}",
        esc(&n.title),
        esc(&n.body),
        esc(&n.app),
        esc(&n.time),
    )
}

// -- Persistence --------------------------------------------------------------

fn load_notifications() -> Vec<Notification> {
    let content = match std::fs::read_to_string(LOG_PATH) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    content.lines().filter_map(parse_notification).collect()
}

fn save_notifications(notifs: &[Notification]) {
    let mut out = String::new();
    for n in notifs {
        out.push_str(&notification_to_json(n));
        out.push('\n');
    }
    let _ = std::fs::write(LOG_PATH, &out);
}

// -- UI rendering -------------------------------------------------------------

/// Number of notification card slots visible between header and footer.
fn visible_slots() -> usize {
    let footer_area = FOOTER_H + CLEAR_BTN_H + CARD_GAP;
    let avail = H.saturating_sub(LIST_TOP + footer_area);
    (avail / CARD_PITCH).max(1) as usize
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max_chars.saturating_sub(3)).collect();
        t.push_str("...");
        t
    }
}

fn draw(notifs: &[Notification], selected: usize, scroll: usize) {
    // Background
    fill(0, 0, W, H, C_BG);
    border(0, 0, W, H, C_BORDER);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    text(16, 10, C_TEXT, "Notification Center");
    fill(0, HEADER_H, W, 1, C_BORDER);

    if notifs.is_empty() {
        text(W / 2 - 60, H / 2 - 8, C_EMPTY, "No notifications");
        draw_footer(notifs.len());
        flush();
        return;
    }

    let slots = visible_slots();
    let total = notifs.len();

    // Draw notification cards (newest-first)
    for slot in 0..slots {
        let idx = scroll + slot;
        if idx >= total {
            break;
        }
        // newest = last element in the vec
        let arr_idx = total - 1 - idx;
        let n = &notifs[arr_idx];
        let cy = LIST_TOP + (slot as u32) * CARD_PITCH;
        let bg = if idx == selected { C_SEL } else { C_CARD };

        fill(CARD_X, cy, CARD_W, CARD_H, bg);
        border(CARD_X, cy, CARD_W, CARD_H, C_BORDER);

        // App name (accent)
        let app_label = truncate_str(&n.app, 20);
        text(CARD_X + 10, cy + 8, C_APP, &app_label);

        // Timestamp (right side)
        if !n.time.is_empty() {
            let time_trunc = truncate_str(&n.time, 16);
            let time_x = CARD_X + CARD_W - 10 - (time_trunc.len() as u32) * 8;
            text(time_x, cy + 8, C_TIME, &time_trunc);
        }

        // Title
        let title_display = truncate_str(&n.title, 44);
        text(CARD_X + 10, cy + 28, C_TEXT, &title_display);

        // Body preview
        let body_display = truncate_str(&n.body, 46);
        text(CARD_X + 10, cy + 48, C_BODY, &body_display);
    }

    // "Clear All" button
    let clear_y = H - FOOTER_H - CLEAR_BTN_H - CARD_GAP;
    let is_clear_selected = selected == total; // virtual item after all cards
    let clear_bg = if is_clear_selected { C_CLEAR_SEL } else { C_CLEAR_BG };
    fill(CARD_X, clear_y, CARD_W, CLEAR_BTN_H, clear_bg);
    border(CARD_X, clear_y, CARD_W, CLEAR_BTN_H, C_BORDER);
    text(CARD_X + CARD_W / 2 - 36, clear_y + 8, C_CLEAR_TEXT, "Clear All");

    // Scroll indicator in header area
    if total > slots {
        let end = (scroll + slots).min(total);
        let ind = format!("{}-{}/{}", scroll + 1, end, total);
        let ind_x = W - 16 - (ind.len() as u32) * 8;
        text(ind_x, 10, C_HINT, &ind);
    }

    draw_footer(total);
    flush();
}

fn draw_footer(count: usize) {
    let fy = H - FOOTER_H;
    fill(0, fy, W, FOOTER_H, C_FOOTER);
    fill(0, fy, W, 1, C_BORDER);
    if count > 0 {
        let hint = "Up/Dn d:del c:clr Enter:focus Esc:close";
        text(8, fy + 6, C_HINT, hint);
    } else {
        text(8, fy + 6, C_HINT, "Esc: close");
    }
}

// -- Main loop ----------------------------------------------------------------

fn clear_and_exit() {
    fill(0, 0, W, H, 0x0D1117FF);
    flush();
    std::process::exit(0);
}

fn main() {
    let stdin = io::stdin();
    let mut notifs = load_notifications();
    let mut selected: usize = 0;
    let mut scroll: usize = 0;

    // Raise ourselves to front
    println!("@supervisor: raise notification-center");
    let _ = io::stdout().flush();

    draw(&notifs, selected, scroll);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Ignore supervisor replies
        if raw.starts_with("REPLY:") {
            continue;
        }

        // Live notification from supervisor
        if let Some(json) = raw.strip_prefix("VYOMA_SYSTEM:notification:") {
            if let Some(n) = parse_notification(json) {
                notifs.push(n);
                save_notifications(&notifs);
                selected = 0;
                scroll = 0;
                draw(&notifs, selected, scroll);
            }
            continue;
        }

        // Legacy NOTIFY: format (from older supervisor versions)
        if raw.starts_with("NOTIFY:") {
            let payload = raw.trim_start_matches("NOTIFY:");
            let mut parts = payload.splitn(2, '|');
            let title = parts.next().unwrap_or("").trim().to_string();
            let body = parts.next().unwrap_or("").trim().to_string();
            if !title.is_empty() {
                notifs.push(Notification {
                    title,
                    body,
                    app: String::new(),
                    time: String::new(),
                });
                save_notifications(&notifs);
                selected = 0;
                scroll = 0;
                draw(&notifs, selected, scroll);
            }
            continue;
        }

        // Escape
        if raw == "\x1b" || raw == "\x1b[" {
            clear_and_exit();
        }

        // Arrow Up
        if raw == "\x1b[A" {
            if selected > 0 {
                selected -= 1;
                if selected < scroll {
                    scroll = selected;
                }
            }
            draw(&notifs, selected, scroll);
            continue;
        }

        // Arrow Down
        if raw == "\x1b[B" {
            let max_sel = notifs.len(); // Clear All = virtual index == len
            if selected < max_sel {
                selected += 1;
                let slots = visible_slots();
                // Only auto-scroll when within the notification list
                if selected < notifs.len() && selected >= scroll + slots {
                    scroll = selected - slots + 1;
                }
            }
            draw(&notifs, selected, scroll);
            continue;
        }

        // Ctrl+C
        if raw == "\x03" {
            clear_and_exit();
        }

        // Enter -- focus source app or activate Clear All
        if raw.is_empty() {
            if notifs.is_empty() {
                continue;
            }
            if selected == notifs.len() {
                // Clear All
                notifs.clear();
                save_notifications(&notifs);
                selected = 0;
                scroll = 0;
            } else if selected < notifs.len() {
                let arr_idx = notifs.len() - 1 - selected;
                let app_name = notifs[arr_idx].app.clone();
                if !app_name.is_empty() {
                    println!("@supervisor: focus {app_name}");
                    let _ = io::stdout().flush();
                }
            }
            draw(&notifs, selected, scroll);
            continue;
        }

        // 'd' -- dismiss selected notification
        if raw == "d" {
            if !notifs.is_empty() && selected < notifs.len() {
                let arr_idx = notifs.len() - 1 - selected;
                notifs.remove(arr_idx);
                save_notifications(&notifs);
                if selected > 0 && selected >= notifs.len() {
                    selected = notifs.len().saturating_sub(1);
                }
                let slots = visible_slots();
                if scroll > 0 && scroll + slots > notifs.len() {
                    scroll = notifs.len().saturating_sub(slots);
                }
            }
            draw(&notifs, selected, scroll);
            continue;
        }

        // 'c' / 'C' -- clear all
        if raw == "c" || raw == "C" {
            notifs.clear();
            save_notifications(&notifs);
            selected = 0;
            scroll = 0;
            draw(&notifs, selected, scroll);
            continue;
        }
    }
}
