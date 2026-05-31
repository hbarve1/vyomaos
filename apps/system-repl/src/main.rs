// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS System REPL — interactive command-line for supervisor IPC.
//!
//! Provides a `vyoma> ` prompt where users type supervisor commands.
//! Commands are sent as `@supervisor: <cmd>` IPC messages. Replies arrive
//! prefixed with `REPLY:` and are displayed in a scrollable output area.
//!
//! Built-in commands: help, clear, history, apps, run <app>.
//! Tab auto-completes known commands. Up/Down arrows navigate history.

use std::io::BufRead;

// ── Colour constants ─────────────────────────────────────────────────────────

const BG: u32 = 0x1E1E2EFF;
const FG: u32 = 0xCDD6F4FF;
const PROMPT_CLR: u32 = 0xA6E3A1FF; // green prompt
const CURSOR_CLR: u32 = 0xF5E0DCFF; // cursor block
const HEADER_BG: u32 = 0x313244FF;
const HEADER_FG: u32 = 0x89B4FAFF;
const CMD_ECHO: u32 = 0xBAC2DEFF; // echoed command colour
const ERR_CLR: u32 = 0xF38BA8FF;

// ── Layout ───────────────────────────────────────────────────────────────────

const FONT_W: u32 = 8;
const FONT_H: u32 = 16;
const PAD: u32 = 8;
const HEADER_H: u32 = 24;

// ── Known supervisor commands for tab-completion ─────────────────────────────

const KNOWN_CMDS: &[&str] = &[
    "apps", "clear", "help", "history", "run ",
    "ps", "ps-raw", "kill ", "restart ", "log ", "logf ",
    "list", "focus ", "reload ", "status",
];

// ── History file on persistent storage ───────────────────────────────────────

const HISTORY_PATH: &str = "/data/repl-history.txt";
const MAX_HISTORY: usize = 100;
const MAX_OUTPUT: usize = 500;

// ── State ────────────────────────────────────────────────────────────────────

struct Repl {
    output: Vec<String>,
    input: String,
    cursor: usize,
    history: Vec<String>,
    hist_idx: usize,
    saved_input: String,
    tab_matches: Vec<String>,
    tab_cycle: usize,
    sw: u32,
    sh: u32,
}

impl Repl {
    fn new() -> Self {
        let history = load_history();
        Self {
            output: Vec::new(),
            input: String::new(),
            cursor: 0,
            history,
            hist_idx: 0,
            saved_input: String::new(),
            tab_matches: Vec::new(),
            tab_cycle: 0,
            sw: 960,
            sh: 720,
        }
    }

    fn push_output(&mut self, line: String) {
        self.output.push(line);
        if self.output.len() > MAX_OUTPUT {
            self.output.remove(0);
        }
    }
}

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    let mut r = Repl::new();
    draw(&r);

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw {
            Ok(l) => l,
            Err(_) => break,
        };

        // Screen dimension notification.
        if let Some(dims) = raw.strip_prefix("VYOMA_SYSTEM:screen:") {
            if let Some((ws, hs)) = dims.split_once(',') {
                if let (Ok(w), Ok(h)) = (ws.parse::<u32>(), hs.parse::<u32>()) {
                    r.sw = w;
                    r.sh = h;
                }
            }
            continue;
        }

        // Supervisor reply.
        if let Some(reply) = raw.strip_prefix("REPLY:") {
            for part in reply.split('|') {
                let part = part.trim();
                if !part.is_empty() {
                    r.push_output(part.to_string());
                }
            }
            draw(&r);
            continue;
        }

        // Reset tab-completion state on any non-tab key.
        if raw != "\x09" {
            r.tab_matches.clear();
            r.tab_cycle = 0;
        }

        match raw.as_str() {
            // Enter
            "" => handle_enter(&mut r),
            // Backspace
            "\x7f" => {
                if r.cursor > 0 {
                    r.input.remove(r.cursor - 1);
                    r.cursor -= 1;
                }
            }
            // Ctrl+C — clear input
            "\x03" => {
                r.input.clear();
                r.cursor = 0;
                r.hist_idx = 0;
                r.saved_input.clear();
            }
            // Ctrl+A — cursor to start
            "\x01" => r.cursor = 0,
            // Ctrl+E — cursor to end
            "\x05" => r.cursor = r.input.len(),
            // Ctrl+L — clear output
            "\x0C" => r.output.clear(),
            // Arrow up — history back
            "\x1b[A" => history_up(&mut r),
            // Arrow down — history forward
            "\x1b[B" => history_down(&mut r),
            // Tab — auto-complete
            "\x09" => handle_tab(&mut r),
            // Printable ASCII
            s if s.len() == 1
                && s.bytes()
                    .next()
                    .map(|b| (0x20..=0x7E).contains(&b))
                    .unwrap_or(false) =>
            {
                r.input.insert_str(r.cursor, s);
                r.cursor += 1;
            }
            // Legacy / line mode fallback
            other => {
                let cmd = other.trim().to_string();
                if !cmd.is_empty() {
                    r.input.clear();
                    r.cursor = 0;
                    execute_input(&mut r, &cmd);
                }
            }
        }

        draw(&r);
    }
}

// ── Command handling ─────────────────────────────────────────────────────────

fn handle_enter(r: &mut Repl) {
    let cmd = r.input.trim().to_string();
    r.input.clear();
    r.cursor = 0;
    r.hist_idx = 0;
    r.saved_input.clear();

    if cmd.is_empty() {
        return;
    }

    // Record in history (deduplicate consecutive).
    if r.history.last().map(|s| s.as_str()) != Some(cmd.as_str()) {
        if r.history.len() >= MAX_HISTORY {
            r.history.remove(0);
        }
        r.history.push(cmd.clone());
        save_history(&r.history);
    }

    execute_input(r, &cmd);
}

fn execute_input(r: &mut Repl, cmd: &str) {
    r.push_output(format!("vyoma> {cmd}"));

    match cmd {
        "help" => show_help(r),
        "clear" => {
            r.output.clear();
            return;
        }
        "history" => show_history(r),
        "apps" => {
            println!("@supervisor: ps-raw");
        }
        _ if cmd.starts_with("run ") => {
            let app = cmd[4..].trim();
            if app.is_empty() {
                r.push_output("usage: run <app-name>".to_string());
            } else {
                println!("@supervisor: run {app}");
            }
        }
        _ => {
            // Send as-is to supervisor.
            println!("@supervisor: {cmd}");
        }
    }
}

fn show_help(r: &mut Repl) {
    let lines = [
        "--- System REPL Commands ---",
        "help           show this help message",
        "clear          clear the output area",
        "history        show command history",
        "apps           list running apps (ps-raw)",
        "run <app>      start an app",
        "",
        "--- Supervisor Commands (sent via IPC) ---",
        "ps             list running apps",
        "ps-raw         list apps (machine format)",
        "kill <app>     terminate an app",
        "restart <app>  restart an app",
        "log <app>      show app logs",
        "logf <app>     tail app logs",
        "list           list all apps",
        "focus <app>    switch keyboard focus",
        "reload <app>   reload an app",
        "status         supervisor status",
        "",
        "--- Keyboard Shortcuts ---",
        "Tab            auto-complete command",
        "Up/Down        navigate history",
        "Ctrl+C         clear input",
        "Ctrl+L         clear output",
        "Ctrl+A         cursor to start",
        "Ctrl+E         cursor to end",
    ];
    for line in &lines {
        r.push_output(line.to_string());
    }
}

fn show_history(r: &mut Repl) {
    if r.history.is_empty() {
        r.push_output("(no history)".to_string());
        return;
    }
    let lines: Vec<String> = r
        .history
        .iter()
        .enumerate()
        .map(|(i, cmd)| format!("{:>4}  {cmd}", i + 1))
        .collect();
    for line in lines {
        r.push_output(line);
    }
}

// ── History navigation ───────────────────────────────────────────────────────

fn history_up(r: &mut Repl) {
    if r.history.is_empty() {
        return;
    }
    if r.hist_idx == 0 {
        r.saved_input = r.input.clone();
    }
    r.hist_idx = (r.hist_idx + 1).min(r.history.len());
    r.input = r.history[r.history.len() - r.hist_idx].clone();
    r.cursor = r.input.len();
}

fn history_down(r: &mut Repl) {
    if r.hist_idx == 0 {
        return;
    }
    r.hist_idx -= 1;
    r.input = if r.hist_idx == 0 {
        r.saved_input.clone()
    } else {
        r.history[r.history.len() - r.hist_idx].clone()
    };
    r.cursor = r.input.len();
}

// ── Tab completion ───────────────────────────────────────────────────────────

fn handle_tab(r: &mut Repl) {
    if r.tab_matches.is_empty() {
        // Build match list from known commands.
        let prefix = r.input.as_str();
        let matches: Vec<String> = KNOWN_CMDS
            .iter()
            .filter(|c| c.starts_with(prefix) && **c != prefix)
            .map(|c| c.to_string())
            .collect();
        if matches.is_empty() {
            return;
        }
        r.tab_matches = matches;
        r.tab_cycle = 0;
    }

    // Cycle through matches.
    if !r.tab_matches.is_empty() {
        let candidate = &r.tab_matches[r.tab_cycle % r.tab_matches.len()];
        r.input = candidate.clone();
        r.cursor = r.input.len();
        r.tab_cycle += 1;
    }
}

// ── Persistent history ───────────────────────────────────────────────────────

fn load_history() -> Vec<String> {
    std::fs::read_to_string(HISTORY_PATH)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| l.to_string())
        .collect()
}

fn save_history(history: &[String]) {
    let data = history.join("\n");
    let _ = std::fs::write(HISTORY_PATH, data);
}

// ── Drawing ──────────────────────────────────────────────────────────────────

fn draw(r: &Repl) {
    let w = r.sw;
    let h = r.sh;

    // Background.
    println!("VYOMA_DRAW:fill_rect:0,0,{w},{h},{BG}");

    // Header bar.
    println!("VYOMA_DRAW:fill_rect:0,0,{w},{HEADER_H},{HEADER_BG}");
    println!("VYOMA_DRAW:draw_text:{PAD},4,{HEADER_FG},m,System REPL");

    // History count in header.
    let hcount = format!("history: {}", r.history.len());
    let hx = w.saturating_sub(hcount.len() as u32 * FONT_W + PAD);
    println!("VYOMA_DRAW:draw_text:{hx},4,{FG},m,{hcount}");

    // Output area: from below header to above prompt line.
    let prompt_y = h.saturating_sub(FONT_H + PAD * 2);
    let output_top = HEADER_H + PAD;
    let avail_lines = ((prompt_y.saturating_sub(output_top)) / FONT_H) as usize;
    let max_cols = ((w - PAD * 2) / FONT_W) as usize;

    // Render output lines (newest at bottom, scroll if overflow).
    let start = if r.output.len() > avail_lines {
        r.output.len() - avail_lines
    } else {
        0
    };
    for (i, line) in r.output[start..].iter().enumerate() {
        let y = output_top + i as u32 * FONT_H;
        let truncated = if line.len() > max_cols {
            &line[..max_cols]
        } else {
            line.as_str()
        };
        // Colour echoed commands differently.
        let clr = if truncated.starts_with("vyoma> ") {
            CMD_ECHO
        } else if truncated.starts_with("error") || truncated.starts_with("Error") {
            ERR_CLR
        } else {
            FG
        };
        println!("VYOMA_DRAW:draw_text:{PAD},{y},{clr},m,{truncated}");
    }

    // Prompt separator line.
    println!(
        "VYOMA_DRAW:fill_rect:0,{},{w},1,{HEADER_BG}",
        prompt_y.saturating_sub(4)
    );

    // Prompt.
    let py = prompt_y;
    println!("VYOMA_DRAW:draw_text:{PAD},{py},{PROMPT_CLR},m,vyoma> ");
    let input_x = PAD + 7 * FONT_W; // "vyoma> " is 7 chars
    let visible = if r.input.len() > max_cols.saturating_sub(8) {
        &r.input[r.input.len() - max_cols.saturating_sub(8)..]
    } else {
        r.input.as_str()
    };
    println!("VYOMA_DRAW:draw_text:{input_x},{py},{FG},m,{visible}");

    // Cursor block.
    let cursor_x = input_x + r.cursor as u32 * FONT_W;
    println!("VYOMA_DRAW:fill_rect:{cursor_x},{py},{FONT_W},{FONT_H},{CURSOR_CLR}");

    println!("VYOMA_DRAW:flush");
}
