// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS interactive shell
//!
//! Reads keyboard input from stdin (forwarded by the supervisor input thread
//! from /dev/tty0 in raw mode).  Each keypress arrives as a separate message:
//!
//!   ""       — Enter key   → execute current_input as a command
//!   "\x7f"   — Backspace   → remove last character from current_input
//!   "\x03"   — Ctrl+C      → clear current_input
//!   "\x01"   — Ctrl+A      → move cursor to start of line
//!   "\x05"   — Ctrl+E      → move cursor to end of line
//!   "\x0C"   — Ctrl+L      → clear screen output and redraw prompt
//!   one char — printable   → append to current_input
//!   multi-ch — legacy line mode (headless / non-raw fallback)
//!
//! Supervisor reply protocol:
//!   Replies arrive prefixed with "REPLY:" so the shell can
//!   distinguish them from keyboard input.
//!
//! Renders a command prompt panel on the lower portion of the framebuffer
//! via VYOMA_DRAW.  Redraws on every keypress to show the live input buffer.

use std::io::BufRead;

mod banner;
mod commands;
mod history;
mod ui;

use commands::{handle_command, is_clear_cmd, tab_complete, path_completions, push_line, DATA_ENTRIES};
use history::{load_history, append_history};
use ui::draw_panel;

fn main() {
    let mut lines: Vec<String> = Vec::new();
    let mut current_input = String::new();
    let mut cursor_pos: usize = 0;
    let mut history: Vec<String> = load_history();
    let mut hist_idx: usize = 0;
    let mut saved_input = String::new();
    // Runtime screen width — updated when VYOMA_SYSTEM:screen: arrives.
    let mut sw: u32 = ui::DEFAULT_SW;
    let mut sh: u32 = ui::DEFAULT_SH;
    // Draw panel first, then overlay the welcome banner.
    // The banner is naturally cleared on the next draw_panel call (first keypress).
    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
    banner::draw_banner(sw);

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw { Ok(l) => l, Err(_) => break };

        // ── Screen dimension notification ─────────────────────────────────────
        if let Some(dims) = raw.strip_prefix("VYOMA_SYSTEM:screen:") {
            if let Some((ws, hs)) = dims.split_once(',') {
                if let (Ok(w), Ok(h)) = (ws.parse::<u32>(), hs.parse::<u32>()) {
                    sw = w; sh = h;
                }
            }
            continue;
        }

        // ── Supervisor reply ──────────────────────────────────────────────────
        if let Some(reply) = raw.strip_prefix("REPLY:") {
            for item in reply.split('|') {
                let item = item.trim();
                if !item.is_empty() {
                    push_line(&mut lines, item.to_string());
                }
            }
            draw_panel(&lines, &current_input, cursor_pos, sw, sh);
            continue;
        }

        // ── Raw tty mode: single-char messages ───────────────────────────────
        match raw.as_str() {
            "\x03" => {
                current_input.clear();
                cursor_pos = 0;
                hist_idx = 0;
                saved_input.clear();
                draw_panel(&lines, &current_input, cursor_pos, sw, sh);
            }
            "\x01" => {
                cursor_pos = 0;
                draw_panel(&lines, &current_input, cursor_pos, sw, sh);
            }
            "\x05" => {
                cursor_pos = current_input.len();
                draw_panel(&lines, &current_input, cursor_pos, sw, sh);
            }
            "\x0C" => {
                lines.clear();
                draw_panel(&lines, &current_input, cursor_pos, sw, sh);
            }
            "\x7f" => {
                current_input.pop();
                cursor_pos = current_input.len();
                draw_panel(&lines, &current_input, cursor_pos, sw, sh);
            }
            "" => {
                let cmd = current_input.trim().to_string();
                current_input.clear();
                cursor_pos = 0;
                hist_idx = 0;
                if !cmd.is_empty() {
                    if history.last().map(|s| s.as_str()) != Some(cmd.as_str()) {
                        if history.len() >= 50 { history.remove(0); }
                        history.push(cmd.clone());
                        append_history(&cmd);
                    }
                }
                if cmd.is_empty() {
                    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
                } else if is_clear_cmd(&cmd) {
                    lines.clear();
                    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
                } else {
                    push_line(&mut lines, format!("> {cmd}"));
                    handle_command(&cmd, &mut lines);
                    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
                }
            }
            "\x1b[A" => {
                if !history.is_empty() {
                    if hist_idx == 0 { saved_input = current_input.clone(); }
                    hist_idx = (hist_idx + 1).min(history.len());
                    current_input = history[history.len() - hist_idx].clone();
                    cursor_pos = current_input.len();
                    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
                }
            }
            "\x1b[B" => {
                if hist_idx > 0 {
                    hist_idx -= 1;
                    current_input = if hist_idx == 0 {
                        saved_input.clone()
                    } else {
                        history[history.len() - hist_idx].clone()
                    };
                    cursor_pos = current_input.len();
                    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
                }
            }
            "\x09" => {
                // Tab — /data/ path completion takes priority, then command completion
                const DATA_PREFIX: &str = "/data/";
                if let Some(suffix) = current_input.strip_prefix(DATA_PREFIX) {
                    let matches = path_completions(suffix, DATA_ENTRIES);
                    if matches.len() == 1 {
                        current_input = format!("{}{}", DATA_PREFIX, matches[0]);
                    } else if matches.is_empty() {
                        push_line(&mut lines, format!("no /data/ match for '{suffix}'"));
                    } else {
                        let hint = matches.join("  ");
                        push_line(&mut lines, format!("candidates: {hint}"));
                    }
                } else if let Some(completed) = tab_complete(&current_input) {
                    current_input = completed.to_string();
                }
                draw_panel(&lines, &current_input, cursor_pos, sw, sh);
            }
            s if s.len() == 1
                && s.bytes().next().map(|b| (0x20..=0x7E).contains(&b)).unwrap_or(false) =>
            {
                current_input.push_str(s);
                cursor_pos = current_input.len();
                draw_panel(&lines, &current_input, cursor_pos, sw, sh);
            }
            // ── Legacy / line mode fallback ───────────────────────────────────
            other => {
                let cmd = other.trim().to_string();
                if cmd.is_empty() {
                    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
                } else if is_clear_cmd(&cmd) {
                    current_input.clear();
                    cursor_pos = 0;
                    lines.clear();
                    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
                } else {
                    current_input.clear();
                    cursor_pos = 0;
                    push_line(&mut lines, format!("> {cmd}"));
                    handle_command(&cmd, &mut lines);
                    draw_panel(&lines, &current_input, cursor_pos, sw, sh);
                }
            }
        }
    }
}
