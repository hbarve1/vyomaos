//! VyomaOS interactive shell
//!
//! Reads keyboard input from stdin (forwarded by the supervisor input thread
//! from /dev/tty0).  Renders a command prompt panel on the lower portion of
//! the framebuffer via VYOMA_DRAW.  Dispatches commands via @supervisor: IPC.
//!
//! Supervisor reply protocol:
//!   Replies arrive on stdin prefixed with "REPLY:" so the shell can
//!   distinguish them from keyboard input lines.

use std::io::{BufRead, Write};

// ── Panel geometry (lower portion of 1440×900 screen) ────────────────────────

const PX: u32 = 24;       // panel left edge
const PY: u32 = 450;      // panel top edge
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

    draw_panel(&lines, "");

    let stdin = std::io::stdin();
    for raw in stdin.lock().lines() {
        let raw = match raw { Ok(l) => l, Err(_) => break };

        // ── Supervisor reply ──────────────────────────────────────────────────
        if let Some(reply) = raw.strip_prefix("REPLY:") {
            // pipe-separated list (for "list") or JSON (for "status") or message
            for item in reply.split('|') {
                let item = item.trim();
                if !item.is_empty() {
                    push_line(&mut lines, item.to_string());
                }
            }
            draw_panel(&lines, "");
            continue;
        }

        // ── Keyboard input: a completed command line ──────────────────────────
        let cmd = raw.trim().to_string();
        if cmd.is_empty() {
            draw_panel(&lines, "");
            continue;
        }

        push_line(&mut lines, format!("> {cmd}"));

        match cmd.as_str() {
            "help" => {
                push_line(&mut lines, "commands:".into());
                push_line(&mut lines, "  help              — this text".into());
                push_line(&mut lines, "  ps                — list all apps + status".into());
                push_line(&mut lines, "  status            — running app count".into());
                push_line(&mut lines, "  list              — list app names".into());
                push_line(&mut lines, "  log <app>         — last 20 lines (memory)".into());
                push_line(&mut lines, "  logf <app>        — last 30 lines from disk log".into());
                push_line(&mut lines, "  logs              — list apps with log files".into());
                push_line(&mut lines, "  kill <app>        — terminate an app".into());
                push_line(&mut lines, "  restart <app>     — kill + relaunch an app".into());
                push_line(&mut lines, "  run <app>         — launch an app by name".into());
                push_line(&mut lines, "  reload            — re-read boot.toml".into());
                push_line(&mut lines, "  pkg list          — list available packages".into());
                push_line(&mut lines, "  pkg install <n>   — install a package".into());
                push_line(&mut lines, "  pkg remove <n>    — remove a package".into());
                push_line(&mut lines, "  pkg installed     — list installed packages".into());
                push_line(&mut lines, "  clear             — clear shell output".into());
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
                push_line(&mut lines, "reloading boot.toml...".into());
            }
            "logs" => {
                println!("@supervisor: logs");
            }
            other if other.starts_with("logf ") => {
                let app = other[5..].trim();
                if app.is_empty() {
                    push_line(&mut lines, "usage: logf <appname>".into());
                } else {
                    println!("@supervisor: logf {app}");
                }
            }
            other if other.starts_with("log ") => {
                let app = other[4..].trim();
                if app.is_empty() {
                    push_line(&mut lines, "usage: log <appname>".into());
                } else {
                    println!("@supervisor: log {app}");
                }
            }
            other if other.starts_with("kill ") => {
                let app = other[5..].trim();
                if app.is_empty() {
                    push_line(&mut lines, "usage: kill <appname>".into());
                } else {
                    println!("@supervisor: kill {app}");
                    push_line(&mut lines, format!("killing {app}..."));
                }
            }
            other if other.starts_with("restart ") => {
                let app = other[8..].trim();
                if app.is_empty() {
                    push_line(&mut lines, "usage: restart <appname>".into());
                } else {
                    println!("@supervisor: restart {app}");
                    push_line(&mut lines, format!("restarting {app}..."));
                }
            }
            // pkg <subcmd> [arg] — package manager
            other if other.starts_with("pkg ") || other == "pkg" => {
                let sub = other[4..].trim(); // strip "pkg "
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
                            push_line(&mut lines, "usage: pkg install <name>".into());
                        } else {
                            println!("@supervisor: pkg-install {pkg}");
                            push_line(&mut lines, format!("installing {pkg}..."));
                        }
                    }
                    s if s.starts_with("remove ") => {
                        let pkg = s[7..].trim();
                        if pkg.is_empty() {
                            push_line(&mut lines, "usage: pkg remove <name>".into());
                        } else {
                            println!("@supervisor: pkg-remove {pkg}");
                            push_line(&mut lines, format!("removing {pkg}..."));
                        }
                    }
                    _ => {
                        push_line(&mut lines, "pkg: list | install <n> | remove <n> | installed".into());
                    }
                }
            }
            other if other.starts_with("run ") => {
                let app = other[4..].trim();
                if app.is_empty() {
                    push_line(&mut lines, "usage: run <appname>".into());
                } else {
                    println!("@supervisor: run /apps/{app}/vyoma.toml");
                    push_line(&mut lines, format!("launching {app}..."));
                }
            }
            other => {
                push_line(&mut lines, format!("unknown: {other}"));
            }
        }

        draw_panel(&lines, "");
        flush();
    }
}

// ── Keep lines buffer bounded ─────────────────────────────────────────────────

fn push_line(lines: &mut Vec<String>, s: String) {
    lines.push(s);
    while lines.len() > MAX_LINES {
        lines.remove(0);
    }
}

// ── Draw the full shell panel ─────────────────────────────────────────────────

fn draw_panel(lines: &[String], input: &str) {
    // Panel background
    fill(PX, PY, PW, PH, C_PANEL);

    // Title bar
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_ACCENT); // accent line
    text(PX + 8, PY + 4, C_ACCENT, "shell");
    text(PX + PW - 136, PY + 4, C_DIM, "VyomaOS v0.1");

    // Output lines
    for (i, line) in lines.iter().enumerate() {
        let ly = INNER_Y + i as u32 * LINE_H;
        if ly + LINE_H > PROMPT_Y { break; }
        let colour = if line.starts_with("> ") { C_DIM } else { C_WHITE };
        text(INNER_X, ly, colour, line);
    }

    // Prompt + cursor
    fill(PX, PROMPT_Y - 2, PW, 2, 0x30363DFF); // separator
    text(INNER_X, PROMPT_Y, C_PROMPT, "> ");
    if !input.is_empty() {
        text(INNER_X + 16, PROMPT_Y, C_WHITE, input);
    }
    // Blinking cursor block (toggled on each redraw — always shown here)
    let cursor_x = INNER_X + 16 + input.len() as u32 * 8;
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
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},{s}");
}

#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
    // Pipe stdout is block-buffered — must flush explicitly so VYOMA_DRAW
    // commands reach the supervisor without waiting for the buffer to fill.
    let _ = std::io::stdout().flush();
}
