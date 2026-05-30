// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS interactive shell with multi-tab support
//!
//! Reads keyboard input from stdin (forwarded by the supervisor input thread
//! from /dev/tty0 in raw mode).  Each keypress arrives as a separate message:
//!
//!   ""       — Enter key   -> execute current_input as a command
//!   "\x7f"   — Backspace   -> remove last character from current_input
//!   "\x03"   — Ctrl+C      -> clear current_input
//!   "\x01"   — Ctrl+A      -> move cursor to start of line
//!   "\x05"   — Ctrl+E      -> move cursor to end of line
//!   "\x0C"   — Ctrl+L      -> clear screen output and redraw prompt
//!   "\x14"   — Ctrl+T      -> new tab (max 8)
//!   "\x17"   — Ctrl+W      -> close current tab (keep at least 1)
//!   one char — printable   -> append to current_input
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
use ui::draw_shell;

const MAX_TABS: usize = 8;

/// Per-tab state: independent history index, output, and input buffer.
pub struct Tab {
    pub output_lines: Vec<String>,
    pub input_buf: String,
    pub cursor: usize,
    pub cwd: String,
    pub hist_idx: usize,
    pub saved_input: String,
}

impl Tab {
    fn new() -> Self {
        Self {
            output_lines: Vec::new(),
            input_buf: String::new(),
            cursor: 0,
            cwd: "~".to_string(),
            hist_idx: 0,
            saved_input: String::new(),
        }
    }
}

fn main() {
    let mut tabs: Vec<Tab> = vec![Tab::new()];
    let mut active: usize = 0;
    let mut history: Vec<String> = load_history();

    // Runtime screen dimensions — updated when VYOMA_SYSTEM:screen: arrives.
    let mut sw: u32 = ui::DEFAULT_SW;
    let mut sh: u32 = ui::DEFAULT_SH;

    // Initial draw + welcome banner.
    redraw(&tabs, active, sw, sh);
    banner::draw_banner(sw);

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw { Ok(l) => l, Err(_) => break };

        // -- Screen dimension notification ----------------------------------------
        if let Some(dims) = raw.strip_prefix("VYOMA_SYSTEM:screen:") {
            if let Some((ws, hs)) = dims.split_once(',') {
                if let (Ok(w), Ok(h)) = (ws.parse::<u32>(), hs.parse::<u32>()) {
                    sw = w; sh = h;
                }
            }
            continue;
        }

        // -- Supervisor reply -----------------------------------------------------
        if let Some(reply) = raw.strip_prefix("REPLY:") {
            let tab = &mut tabs[active];
            for item in reply.split('|') {
                let item = item.trim();
                if !item.is_empty() {
                    push_line(&mut tab.output_lines, item.to_string());
                }
            }
            redraw(&tabs, active, sw, sh);
            continue;
        }

        // -- Tab switching: Ctrl+1..8 arrive as "\x1b[<49..56>~" -----------------
        if let Some(rest) = raw.strip_prefix("\x1b[") {
            if let Some(num_s) = rest.strip_suffix('~') {
                if let Ok(code) = num_s.parse::<u32>() {
                    if (49..=56).contains(&code) {
                        let idx = (code - 49) as usize;
                        if idx < tabs.len() {
                            active = idx;
                            redraw(&tabs, active, sw, sh);
                        }
                        continue;
                    }
                }
            }
        }

        // -- Raw tty mode: single-char messages -----------------------------------
        match raw.as_str() {
            // Ctrl+T — new tab
            "\x14" => {
                if tabs.len() < MAX_TABS {
                    tabs.push(Tab::new());
                    active = tabs.len() - 1;
                }
                redraw(&tabs, active, sw, sh);
            }
            // Ctrl+W — close current tab (keep at least 1)
            "\x17" => {
                if tabs.len() > 1 {
                    tabs.remove(active);
                    if active >= tabs.len() {
                        active = tabs.len() - 1;
                    }
                }
                redraw(&tabs, active, sw, sh);
            }
            // Ctrl+C — clear input
            "\x03" => {
                let tab = &mut tabs[active];
                tab.input_buf.clear();
                tab.cursor = 0;
                tab.hist_idx = 0;
                tab.saved_input.clear();
                redraw(&tabs, active, sw, sh);
            }
            // Ctrl+A — cursor to start
            "\x01" => {
                tabs[active].cursor = 0;
                redraw(&tabs, active, sw, sh);
            }
            // Ctrl+E — cursor to end
            "\x05" => {
                let tab = &mut tabs[active];
                tab.cursor = tab.input_buf.len();
                redraw(&tabs, active, sw, sh);
            }
            // Ctrl+L — clear output
            "\x0C" => {
                tabs[active].output_lines.clear();
                redraw(&tabs, active, sw, sh);
            }
            // Backspace
            "\x7f" => {
                let tab = &mut tabs[active];
                if tab.cursor > 0 {
                    tab.input_buf.remove(tab.cursor - 1);
                    tab.cursor -= 1;
                }
                redraw(&tabs, active, sw, sh);
            }
            // Enter
            "" => {
                handle_enter(&mut tabs[active], &mut history);
                redraw(&tabs, active, sw, sh);
            }
            // Arrow up — history back
            "\x1b[A" => {
                handle_history_up(&mut tabs[active], &history);
                redraw(&tabs, active, sw, sh);
            }
            // Arrow down — history forward
            "\x1b[B" => {
                handle_history_down(&mut tabs[active], &history);
                redraw(&tabs, active, sw, sh);
            }
            // Tab — completion
            "\x09" => {
                handle_tab_complete(&mut tabs[active]);
                redraw(&tabs, active, sw, sh);
            }
            // Printable ASCII
            s if s.len() == 1
                && s.bytes().next().map(|b| (0x20..=0x7E).contains(&b)).unwrap_or(false) =>
            {
                let tab = &mut tabs[active];
                tab.input_buf.insert_str(tab.cursor, s);
                tab.cursor += 1;
                redraw(&tabs, active, sw, sh);
            }
            // Legacy / line mode fallback
            other => {
                handle_legacy(other, &mut tabs[active], &mut history);
                redraw(&tabs, active, sw, sh);
            }
        }
    }
}

fn redraw(tabs: &[Tab], active: usize, sw: u32, sh: u32) {
    let tab = &tabs[active];
    draw_shell(tabs, active, &tab.output_lines, &tab.input_buf, tab.cursor, sw, sh);
}

fn handle_enter(tab: &mut Tab, history: &mut Vec<String>) {
    let cmd = tab.input_buf.trim().to_string();
    tab.input_buf.clear();
    tab.cursor = 0;
    tab.hist_idx = 0;
    if !cmd.is_empty() {
        if history.last().map(|s| s.as_str()) != Some(cmd.as_str()) {
            if history.len() >= 50 { history.remove(0); }
            history.push(cmd.clone());
            append_history(&cmd);
        }
    }
    if cmd.is_empty() {
        // no-op, just redraw
    } else if is_clear_cmd(&cmd) {
        tab.output_lines.clear();
    } else {
        push_line(&mut tab.output_lines, format!("> {cmd}"));
        handle_command(&cmd, &mut tab.output_lines);
    }
}

fn handle_history_up(tab: &mut Tab, history: &[String]) {
    if !history.is_empty() {
        if tab.hist_idx == 0 { tab.saved_input = tab.input_buf.clone(); }
        tab.hist_idx = (tab.hist_idx + 1).min(history.len());
        tab.input_buf = history[history.len() - tab.hist_idx].clone();
        tab.cursor = tab.input_buf.len();
    }
}

fn handle_history_down(tab: &mut Tab, history: &[String]) {
    if tab.hist_idx > 0 {
        tab.hist_idx -= 1;
        tab.input_buf = if tab.hist_idx == 0 {
            tab.saved_input.clone()
        } else {
            history[history.len() - tab.hist_idx].clone()
        };
        tab.cursor = tab.input_buf.len();
    }
}

fn handle_tab_complete(tab: &mut Tab) {
    const DATA_PREFIX: &str = "/data/";
    if let Some(suffix) = tab.input_buf.strip_prefix(DATA_PREFIX) {
        let matches = path_completions(suffix, DATA_ENTRIES);
        if matches.len() == 1 {
            tab.input_buf = format!("{}{}", DATA_PREFIX, matches[0]);
        } else if matches.is_empty() {
            push_line(&mut tab.output_lines, format!("no /data/ match for '{suffix}'"));
        } else {
            let hint = matches.join("  ");
            push_line(&mut tab.output_lines, format!("candidates: {hint}"));
        }
    } else if let Some(completed) = tab_complete(&tab.input_buf) {
        tab.input_buf = completed.to_string();
    }
}

fn handle_legacy(other: &str, tab: &mut Tab, history: &mut Vec<String>) {
    let cmd = other.trim().to_string();
    if cmd.is_empty() {
        // no-op
    } else if is_clear_cmd(&cmd) {
        tab.input_buf.clear();
        tab.cursor = 0;
        tab.output_lines.clear();
    } else {
        tab.input_buf.clear();
        tab.cursor = 0;
        push_line(&mut tab.output_lines, format!("> {cmd}"));
        handle_command(&cmd, &mut tab.output_lines);
        // Add to history
        if !cmd.is_empty() {
            if history.last().map(|s| s.as_str()) != Some(cmd.as_str()) {
                if history.len() >= 50 { history.remove(0); }
                history.push(cmd.clone());
                append_history(&cmd);
            }
        }
    }
}
