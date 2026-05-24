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

use std::io::{BufRead, Write};
use std::fs::OpenOptions;

// ── Tab-completion command list ───────────────────────────────────────────────

const COMPLETIONS: &[&str] = &[
    "ls", "ps", "kill", "log", "logf", "restart", "clear", "help", "exit",
    "status", "list", "reload", "logs", "run", "focus", "raise", "lower",
    "update", "notify", "resize", "wallpaper", "shutdown", "reboot",
    "session-save", "session-restore", "monitors", "dns", "tls-info",
    "download", "clip-set", "clip-get", "screenshot", "pkg",
];

/// Returns the unique completion for `input` if exactly one command starts with
/// it, or `None` when there are zero or multiple matches.
fn tab_complete(input: &str) -> Option<&'static str> {
    let matches: Vec<&str> = COMPLETIONS
        .iter()
        .copied()
        .filter(|c| c.starts_with(input))
        .collect();
    if matches.len() == 1 { Some(matches[0]) } else { None }
}

// ── Soft-wrap helper ──────────────────────────────────────────────────────────

/// Split `text` into chunks of at most `cols` chars.
/// Pure function with no side effects — safe to unit-test.
fn wrap_line(text: &str, cols: usize) -> Vec<String> {
    if cols == 0 || text.is_empty() {
        return vec![text.to_string()];
    }
    text.chars()
        .collect::<Vec<_>>()
        .chunks(cols)
        .map(|c| c.iter().collect())
        .collect()
}

// ── Panel geometry (local window coords; window declared at y=440 in vyoma.toml) ──────

const PX: u32 = 24;       // panel left edge (horizontal margin within window)
const PY: u32 = 10;       // panel top edge in LOCAL window coords (window.y=440)
const PW: u32 = 1392;     // panel width  (1440 - 24*2)
const PH: u32 = 420;      // panel height

const TITLE_H: u32 = 24;                    // title bar height
const INNER_X: u32 = PX + 8;               // content left margin
const INNER_Y: u32 = PY + TITLE_H + 6;     // content top
const LINE_H: u32  = 16;                    // glyph height
const PROMPT_Y: u32 = PY + PH - 22;        // prompt line y position
const MAX_LINES: usize = ((PROMPT_Y - INNER_Y) / LINE_H) as usize;

// ── Colours ───────────────────────────────────────────────────────────────────

const C_PANEL:   u32 = 0x161B22FF; // panel background
const C_TITLE:   u32 = 0x21262DFF; // title bar
const C_ACCENT:  u32 = 0x58A6FFFF; // blue accent
const C_WHITE:   u32 = 0xFFFFFFFF;
const C_DIM:     u32 = 0x8B949EFF; // dimmed text
const C_GREEN:   u32 = 0x3FB950FF;
const C_PROMPT:  u32 = 0x58A6FFFF; // prompt colour

fn main() {
    let mut lines: Vec<String> = Vec::new();
    let mut current_input = String::new();
    let mut cursor_pos: usize = 0;                 // byte position within current_input
    let mut history: Vec<String> = load_history(); // populated from /data/shell_history
    let mut hist_idx: usize = 0;                   // 0 = not browsing history
    let mut saved_input = String::new();           // input saved when ↑ is first pressed

    draw_panel(&lines, &current_input, cursor_pos);

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw { Ok(l) => l, Err(_) => break };

        // ── Supervisor reply ──────────────────────────────────────────────────
        if let Some(reply) = raw.strip_prefix("REPLY:") {
            for item in reply.split('|') {
                let item = item.trim();
                if !item.is_empty() {
                    push_line(&mut lines, item.to_string());
                }
            }
            draw_panel(&lines, &current_input, cursor_pos);
            continue;
        }

        // ── Raw tty mode: single-char messages ───────────────────────────────
        match raw.as_str() {
            "\x03" => {
                // Ctrl+C — clear the input line and reset history navigation
                current_input.clear();
                cursor_pos = 0;
                hist_idx = 0;
                saved_input.clear();
                draw_panel(&lines, &current_input, cursor_pos);
            }
            "\x01" => {
                // Ctrl+A — move cursor to start of line
                cursor_pos = 0;
                draw_panel(&lines, &current_input, cursor_pos);
            }
            "\x05" => {
                // Ctrl+E — move cursor to end of line
                cursor_pos = current_input.len();
                draw_panel(&lines, &current_input, cursor_pos);
            }
            "\x0C" => {
                // Ctrl+L — clear screen and redraw prompt with current input
                lines.clear();
                draw_panel(&lines, &current_input, cursor_pos);
            }
            "\x7f" => {
                // Backspace — remove last character
                current_input.pop();
                cursor_pos = current_input.len();
                draw_panel(&lines, &current_input, cursor_pos);
            }
            "" => {
                // Enter — execute whatever is in the input buffer
                let cmd = current_input.trim().to_string();
                current_input.clear();
                cursor_pos = 0;
                hist_idx = 0; // reset history navigation
                // Push non-empty, non-consecutive-duplicate commands to history
                if !cmd.is_empty() {
                    if history.last().map(|s| s.as_str()) != Some(cmd.as_str()) {
                        if history.len() >= 50 { history.remove(0); }
                        history.push(cmd.clone());
                        append_history(&cmd);
                    }
                }
                if cmd.is_empty() {
                    draw_panel(&lines, &current_input, cursor_pos);
                } else {
                    push_line(&mut lines, format!("> {cmd}"));
                    handle_command(&cmd, &mut lines);
                    draw_panel(&lines, &current_input, cursor_pos);
                }
            }
            "\x1b[A" => {
                // Up arrow — go back in history
                if !history.is_empty() {
                    if hist_idx == 0 { saved_input = current_input.clone(); }
                    hist_idx = (hist_idx + 1).min(history.len());
                    current_input = history[history.len() - hist_idx].clone();
                    cursor_pos = current_input.len();
                    draw_panel(&lines, &current_input, cursor_pos);
                }
            }
            "\x1b[B" => {
                // Down arrow — go forward in history
                if hist_idx > 0 {
                    hist_idx -= 1;
                    current_input = if hist_idx == 0 {
                        saved_input.clone()
                    } else {
                        history[history.len() - hist_idx].clone()
                    };
                    cursor_pos = current_input.len();
                    draw_panel(&lines, &current_input, cursor_pos);
                }
            }
            "\x09" => {
                // Tab — attempt unique-prefix completion (cursor must be at end)
                if let Some(completed) = tab_complete(&current_input) {
                    current_input = completed.to_string();
                    draw_panel(&lines, &current_input);
                }
                // If no unique match, do nothing
            }
            "\t" => {
                // Tab — complete a /data/ path if the input starts with that prefix
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
                }
                draw_panel(&lines, &current_input);
            }
            s if s.len() == 1
                && s.bytes().next().map(|b| (0x20..=0x7E).contains(&b)).unwrap_or(false) =>
            {
                // Single printable ASCII char — append to input buffer
                current_input.push_str(s);
                cursor_pos = current_input.len();
                draw_panel(&lines, &current_input, cursor_pos);
            }
            // ── Legacy / line mode: complete command string (non-raw fallback) ─
            other => {
                let cmd = other.trim().to_string();
                if cmd.is_empty() {
                    draw_panel(&lines, &current_input, cursor_pos);
                } else {
                    current_input.clear();
                    cursor_pos = 0;
                    push_line(&mut lines, format!("> {cmd}"));
                    handle_command(&cmd, &mut lines);
                    draw_panel(&lines, &current_input, cursor_pos);
                }
            }
        }
    }
}

// ── Persistent history helpers ────────────────────────────────────────────────

const HISTORY_PATH: &str = "/data/shell_history";
const HISTORY_CAP: usize = 100;

/// Load up to HISTORY_CAP lines from /data/shell_history.
/// Returns an empty Vec on any I/O error (silent failure).
fn load_history() -> Vec<String> {
    let content = match std::fs::read_to_string(HISTORY_PATH) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let all: Vec<String> = content
        .lines()
        .map(|l| l.to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if all.len() > HISTORY_CAP {
        all[all.len() - HISTORY_CAP..].to_vec()
    } else {
        all
    }
}

/// Append a single command to /data/shell_history.
/// Silently ignores any I/O error.
fn append_history(cmd: &str) {
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(HISTORY_PATH)
    {
        let _ = writeln!(f, "{cmd}");
    }
}

// ── Command dispatcher ────────────────────────────────────────────────────────

fn handle_command(cmd: &str, lines: &mut Vec<String>) {
    match cmd {
        "help" => {
            push_line(lines, "commands:".into());
            push_line(lines, "  help              — this text".into());
            push_line(lines, "  Ctrl+L            — clear screen".into());
            push_line(lines, "  Ctrl+A            — move cursor to line start".into());
            push_line(lines, "  Ctrl+E            — move cursor to line end".into());
            push_line(lines, "  ps                — list all apps + status".into());
            push_line(lines, "  status            — running app count".into());
            push_line(lines, "  list              — list app names".into());
            push_line(lines, "  log <app>         — last 20 lines (memory)".into());
            push_line(lines, "  logf <app>        — last 30 lines from disk log".into());
            push_line(lines, "  logs              — list apps with log files".into());
            push_line(lines, "  kill <app>        — terminate an app".into());
            push_line(lines, "  restart <app>     — kill + relaunch an app".into());
            push_line(lines, "  run <app>         — launch an app by name".into());
            push_line(lines, "  reload            — re-read boot.toml".into());
            push_line(lines, "  pkg list          — list available packages".into());
            push_line(lines, "  pkg install <n>   — install a package".into());
            push_line(lines, "  pkg remove <n>    — remove a package".into());
            push_line(lines, "  pkg installed     — list installed packages".into());
            push_line(lines, "  focus <app>        — give keyboard focus to an app".into());
            push_line(lines, "  raise <app>        — bring window to front (Z-order)".into());
            push_line(lines, "  lower <app>        — send window to back (Z-order)".into());
            push_line(lines, "  update <app> <url> — OTA hot-swap wasm binary from url".into());
            push_line(lines, "  wallpaper <rgba>   — set desktop background color".into());
            push_line(lines, "  resize <app> <w> <h> — resize app window".into());
            push_line(lines, "  notify <title> <msg> — show toast notification".into());
            push_line(lines, "  shutdown           — power off the system".into());
            push_line(lines, "  reboot             — restart the system".into());
            push_line(lines, "  session-save       — save window layout to /data/session.toml".into());
            push_line(lines, "  session-restore    — restore window layout from /data/session.toml".into());
            push_line(lines, "  monitors           — count connected DRM displays".into());
            push_line(lines, "  dns <hostname>     — resolve hostname to IP via supervisor".into());
            push_line(lines, "  tls-info           — check TLS cert/key availability".into());
            push_line(lines, "  download <url> <dest> — download file from HTTP URL to /data path".into());
            push_line(lines, "  clip-set <text>    — copy text to supervisor clipboard".into());
            push_line(lines, "  clip-get           — paste text from supervisor clipboard".into());
            push_line(lines, "  screenshot [path]  — save framebuffer PPM to /data/screenshot.ppm".into());
            push_line(lines, "  clear             — clear shell output".into());
        }
        "clear" => {
            lines.clear();
        }
        "ps" => {
            println!("@supervisor: ps");
        }
        "status" => {
            println!("@supervisor: status");
        }
        "list" => {
            println!("@supervisor: list");
        }
        "reload" => {
            println!("@supervisor: reload");
            push_line(lines, "reloading boot.toml...".into());
        }
        "logs" => {
            println!("@supervisor: logs");
        }
        other if other.starts_with("logf ") => {
            let app = other[5..].trim();
            if app.is_empty() {
                push_line(lines, "usage: logf <appname>".into());
            } else {
                println!("@supervisor: logf {app}");
            }
        }
        other if other.starts_with("log ") => {
            let app = other[4..].trim();
            if app.is_empty() {
                push_line(lines, "usage: log <appname>".into());
            } else {
                println!("@supervisor: log {app}");
            }
        }
        other if other.starts_with("kill ") => {
            let app = other[5..].trim();
            if app.is_empty() {
                push_line(lines, "usage: kill <appname>".into());
            } else {
                println!("@supervisor: kill {app}");
                push_line(lines, format!("killing {app}..."));
            }
        }
        other if other.starts_with("restart ") => {
            let app = other[8..].trim();
            if app.is_empty() {
                push_line(lines, "usage: restart <appname>".into());
            } else {
                println!("@supervisor: restart {app}");
                push_line(lines, format!("restarting {app}..."));
            }
        }
        // pkg <subcmd> [arg] — package manager
        other if other.starts_with("pkg") => {
            let sub = other.get(4..).map(|s| s.trim()).unwrap_or("");
            match sub {
                "list" => {
                    println!("@supervisor: pkg-list");
                }
                "installed" => {
                    println!("@supervisor: pkg-installed");
                }
                s if s.starts_with("install ") => {
                    let pkg = s[8..].trim();
                    if pkg.is_empty() {
                        push_line(lines, "usage: pkg install <name>".into());
                    } else {
                        println!("@supervisor: pkg-install {pkg}");
                        push_line(lines, format!("installing {pkg}..."));
                    }
                }
                s if s.starts_with("remove ") => {
                    let pkg = s[7..].trim();
                    if pkg.is_empty() {
                        push_line(lines, "usage: pkg remove <name>".into());
                    } else {
                        println!("@supervisor: pkg-remove {pkg}");
                        push_line(lines, format!("removing {pkg}..."));
                    }
                }
                _ => {
                    push_line(lines, "pkg: list | install <n> | remove <n> | installed".into());
                }
            }
        }
        other if other.starts_with("resize ") => {
            let args = other[7..].trim();
            if args.split_whitespace().count() < 3 {
                push_line(lines, "usage: resize <app> <w> <h>".into());
            } else {
                println!("@supervisor: resize {args}");
                push_line(lines, format!("resizing {args}..."));
            }
        }
        other if other.starts_with("wallpaper ") => {
            let color = other[10..].trim();
            if color.is_empty() {
                push_line(lines, "usage: wallpaper <rgba_hex>  e.g. wallpaper 0x1E1E2EFF".into());
            } else {
                println!("@supervisor: wallpaper {color}");
                push_line(lines, format!("setting wallpaper to {color}..."));
            }
        }
        other if other.starts_with("raise ") => {
            let app = other[6..].trim();
            if app.is_empty() {
                push_line(lines, "usage: raise <appname>".into());
            } else {
                println!("@supervisor: raise {app}");
                push_line(lines, format!("raising {app}..."));
            }
        }
        other if other.starts_with("lower ") => {
            let app = other[6..].trim();
            if app.is_empty() {
                push_line(lines, "usage: lower <appname>".into());
            } else {
                println!("@supervisor: lower {app}");
                push_line(lines, format!("lowering {app}..."));
            }
        }
        other if other.starts_with("update ") => {
            let args = other[7..].trim();
            if args.is_empty() {
                push_line(lines, "usage: update <app> <url>".into());
            } else {
                println!("@supervisor: update {args}");
                push_line(lines, format!("updating {args}…"));
            }
        }
        other if other.starts_with("notify ") => {
            let args = other[7..].trim();
            if args.is_empty() {
                push_line(lines, "usage: notify <title> <msg>".into());
            } else {
                println!("@supervisor: notify {args}");
                push_line(lines, format!("notification sent: {args}"));
            }
        }
        other if other.starts_with("focus ") => {
            let app = other[6..].trim();
            if app.is_empty() {
                push_line(lines, "usage: focus <appname>".into());
            } else {
                println!("@supervisor: focus {app}");
            }
        }
        other if other.starts_with("run ") => {
            let app = other[4..].trim();
            if app.is_empty() {
                push_line(lines, "usage: run <appname>".into());
            } else {
                println!("@supervisor: run /apps/{app}/vyoma.toml");
                println!("@supervisor: focus {app}");
                push_line(lines, format!("launching {app}..."));
            }
        }
        "shutdown" => {
            push_line(lines, "system shutting down...".into());
            println!("@supervisor: shutdown");
        }
        "reboot" => {
            push_line(lines, "system rebooting...".into());
            println!("@supervisor: reboot");
        }
        "session-save" => {
            println!("@supervisor: session-save");
            push_line(lines, "saving session...".into());
        }
        "session-restore" => {
            println!("@supervisor: session-restore");
            push_line(lines, "restoring session...".into());
        }
        "monitors" => {
            println!("@supervisor: monitors");
        }
        "tls-info" => {
            println!("@supervisor: tls-info");
        }
        other if other.starts_with("dns ") => {
            let host = other[4..].trim();
            if host.is_empty() {
                push_line(lines, "usage: dns <hostname>".into());
            } else {
                println!("@supervisor: dns-resolve {host}");
                push_line(lines, format!("resolving {host}..."));
            }
        }
        other if other.starts_with("download ") => {
            let args = other[9..].trim();
            match args.split_once(' ') {
                Some((url, dest)) if !url.is_empty() && !dest.is_empty() => {
                    println!("@supervisor: download {url} {dest}");
                    push_line(lines, format!("downloading {url} → {dest}"));
                }
                _ => push_line(lines, "usage: download <url> <dest>".into()),
            }
        }
        other if other.starts_with("clip-set ") => {
            let text = other[9..].trim();
            if text.is_empty() {
                push_line(lines, "usage: clip-set <text>".into());
            } else {
                println!("@supervisor: clipboard-set {text}");
                push_line(lines, format!("clipboard set ({} chars)", text.len()));
            }
        }
        "clip-get" => {
            println!("@supervisor: clipboard-get");
        }
        other if other.starts_with("screenshot") => {
            let path = other[10..].trim();
            let dest = if path.is_empty() { "/data/screenshot.ppm" } else { path };
            println!("@supervisor: screenshot {dest}");
            push_line(lines, format!("saving screenshot to {dest}..."));
        }
        other => {
            push_line(lines, format!("unknown: {other}"));
        }
    }
}

// ── /data/ path completion ────────────────────────────────────────────────────

const DATA_ENTRIES: &[&str] = &["shell_history", "boot_count.txt", "boot_log.txt"];

/// Return every entry whose name starts with `prefix`.  Pure: no I/O, no global state.
fn path_completions<'a>(prefix: &str, entries: &'a [&'a str]) -> Vec<&'a str> {
    entries.iter().copied().filter(|e| e.starts_with(prefix)).collect()
}

// ── Keep lines buffer bounded ─────────────────────────────────────────────────

fn push_line(lines: &mut Vec<String>, s: String) {
    lines.push(s);
    while lines.len() > MAX_LINES {
        lines.remove(0);
    }
}

// ── Draw the full shell panel ─────────────────────────────────────────────────

fn draw_panel(lines: &[String], input: &str, cursor_pos: usize) {
    // Panel background + border
    fill(PX, PY, PW, PH, C_PANEL);

    // Title bar
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_ACCENT); // accent line
    text(PX + 8, PY + 4, C_ACCENT, "shell");
    text(PX + PW - 136, PY + 4, C_DIM, "VyomaOS v0.1");
    border(PX, PY, PW, PH, C_ACCENT);

    // Clear output area
    clear_region(INNER_X, INNER_Y, PW - 16, PROMPT_Y - INNER_Y);

    // Output lines — each logical line is soft-wrapped at 90 chars
    let mut ly = INNER_Y;
    'outer: for line in lines.iter() {
        let colour = if line.starts_with("> ") { C_DIM } else { C_WHITE };
        for wrapped in wrap_line(line, 90) {
            if ly + LINE_H > PROMPT_Y { break 'outer; }
            text(INNER_X, ly, colour, &wrapped);
            ly += LINE_H;
        }
    }

    // Prompt + cursor
    fill(PX, PROMPT_Y - 2, PW, 2, 0x30363DFF); // separator
    text(INNER_X, PROMPT_Y, C_PROMPT, "> ");
    if !input.is_empty() {
        text(INNER_X + 16, PROMPT_Y, C_WHITE, input);
    }
    // Cursor block positioned at cursor_pos (chars * 8px per glyph)
    let cursor_x = INNER_X + 16 + cursor_pos as u32 * 8;
    fill(cursor_x, PROMPT_Y, 8, 14, C_GREEN);

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
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba}");
}

#[inline]
fn clear_region(x: u32, y: u32, w: u32, h: u32) {
    println!("VYOMA_DRAW:clear_region:{x},{y},{w},{h}");
}

#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
    // Pipe stdout is block-buffered — must flush explicitly so VYOMA_DRAW
    // commands reach the supervisor without waiting for the buffer to fill.
    let _ = std::io::stdout().flush();
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::{wrap_line, path_completions};

    #[test]
    fn wrap_line_short_fits_one_chunk() {
        assert_eq!(wrap_line("hello", 90), vec!["hello"]);
    }

    #[test]
    fn wrap_line_exact_boundary() {
        let s = "a".repeat(90);
        assert_eq!(wrap_line(&s, 90), vec![s]);
    }

    #[test]
    fn wrap_line_splits_long_line() {
        let s = "a".repeat(91);
        let result = wrap_line(&s, 90);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].len(), 90);
        assert_eq!(result[1].len(), 1);
    }

    #[test]
    fn wrap_line_empty_returns_one() {
        assert_eq!(wrap_line("", 90), vec![""]);
    }

    #[test]
    fn wrap_line_zero_cols_no_panic() {
        let result = wrap_line("hello", 0);
        assert_eq!(result, vec!["hello"]);
    }

    #[test]
    fn path_completions_exact_match() {
        let entries = &["shell_history", "boot_count.txt", "boot_log.txt"];
        assert_eq!(path_completions("shell", entries), vec!["shell_history"]);
    }

    #[test]
    fn path_completions_prefix_match_multiple() {
        let entries = &["boot_count.txt", "boot_log.txt", "shell_history"];
        assert_eq!(path_completions("boot", entries).len(), 2);
    }

    #[test]
    fn path_completions_no_match() {
        let entries = &["shell_history", "boot_count.txt"];
        assert!(path_completions("xyz", entries).is_empty());
    }

    #[test]
    fn path_completions_empty_prefix_returns_all() {
        let entries = &["a", "b", "c"];
        assert_eq!(path_completions("", entries).len(), 3);
    }
}
