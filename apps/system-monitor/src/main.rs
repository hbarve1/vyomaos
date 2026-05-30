// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS Process Monitor
//!
//! Real-time interactive process monitor with keyboard navigation.
//! Polls `@supervisor: ps-raw` every 2 seconds.
//! Keys: Up/Down select, k kill, r restart, f focus, l logs, q quit.

use std::io::{BufRead, Write};
use std::thread;
use std::time::Duration;

// ── Layout ──────────────────────────────────────────────────────────────────

const W: u32 = 700;
const H: u32 = 500;
const PX: u32 = 8;
const PY: u32 = 8;
const PW: u32 = W - 16;
const PH: u32 = H - 16;
const TITLE_H: u32 = 28;
const HDR_H: u32 = 20;
const STATUS_H: u32 = 18;
const LOG_PANE_H: u32 = 100;
const ROW_H: u32 = 18;
const INNER_X: u32 = PX + 8;
const INNER_Y: u32 = PY + TITLE_H + HDR_H + 4;
const INNER_W: u32 = PW - 16;

// Column X offsets (relative to INNER_X)
const COL_NAME: u32 = 0;
const COL_STATUS: u32 = 180;
const COL_UPTIME: u32 = 310;
const COL_RESTARTS: u32 = 430;

// ── Colours ─────────────────────────────────────────────────────────────────

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
const C_SEL: u32 = 0x1F6FEB44;
const C_ALT_ROW: u32 = 0x161B22FF;
const C_ROW: u32 = 0x0D1117FF;
const C_LOG_BG: u32 = 0x0A0E14FF;

// ── Data model ──────────────────────────────────────────────────────────────

#[derive(Default, Clone)]
struct AppRow {
    name: String,
    running: bool,
    uptime: u64,
    restarts: u64,
}

struct State {
    apps: Vec<AppRow>,
    selected: usize,
    tick: u64,
    log_lines: Vec<String>,
    log_target: String,
    show_log: bool,
}

impl State {
    fn new() -> Self {
        Self {
            apps: Vec::new(),
            selected: 0,
            tick: 0,
            log_lines: Vec::new(),
            log_target: String::new(),
            show_log: false,
        }
    }

    fn selected_name(&self) -> Option<&str> {
        self.apps.get(self.selected).map(|a| a.name.as_str())
    }

    fn clamp_selection(&mut self) {
        if !self.apps.is_empty() && self.selected >= self.apps.len() {
            self.selected = self.apps.len() - 1;
        }
    }

    fn max_rows(&self) -> usize {
        let table_h = if self.show_log {
            PH - TITLE_H - HDR_H - STATUS_H - LOG_PANE_H - 12
        } else {
            PH - TITLE_H - HDR_H - STATUS_H - 8
        };
        (table_h / ROW_H) as usize
    }
}

// ── Main ────────────────────────────────────────────────────────────────────

fn main() {
    let mut state = State::new();

    draw(&state);
    query_ps();

    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();

    loop {
        match lines.next() {
            None | Some(Err(_)) => break,
            Some(Ok(line)) => {
                if !handle_line(&line, &mut state) {
                    break;
                }
            }
        }
    }
}

fn handle_line(line: &str, state: &mut State) -> bool {
    match line {
        "q" | "\x03" => return false,
        _ if line.starts_with("REPLY:ps-raw:") => {
            let data = &line["REPLY:ps-raw:".len()..];
            state.apps = parse_ps_raw(data);
            state.clamp_selection();
            state.tick += 1;
            draw(state);
            thread::sleep(Duration::from_secs(2));
            query_ps();
        }
        _ if line.starts_with("REPLY:pong") => {
            query_ps_raw();
        }
        _ if line.starts_with("REPLY:log:") => {
            let log_data = &line["REPLY:log:".len()..];
            state.log_lines = log_data
                .split("\\n")
                .map(|s| s.to_string())
                .collect();
            // Keep last 5 lines
            if state.log_lines.len() > 5 {
                let start = state.log_lines.len() - 5;
                state.log_lines = state.log_lines[start..].to_vec();
            }
            state.show_log = true;
            draw(state);
        }
        _ if line.starts_with("REPLY:") => {
            // Ignore other replies, redraw
            draw(state);
        }
        "VYOMA_INPUT:key:up" => {
            if state.selected > 0 {
                state.selected -= 1;
                draw(state);
            }
        }
        "VYOMA_INPUT:key:down" => {
            if state.selected + 1 < state.apps.len() {
                state.selected += 1;
                draw(state);
            }
        }
        "k" => {
            if let Some(name) = state.selected_name() {
                println!("@supervisor: kill {name}");
                let _ = std::io::stdout().flush();
            }
        }
        "r" => {
            if let Some(name) = state.selected_name() {
                println!("@supervisor: restart {name}");
                let _ = std::io::stdout().flush();
            }
        }
        "f" => {
            if let Some(name) = state.selected_name() {
                println!("@supervisor: focus {name}");
                let _ = std::io::stdout().flush();
            }
        }
        "l" => {
            if let Some(name) = state.selected_name().map(str::to_string) {
                state.log_target = name.clone();
                println!("@supervisor: log {name}");
                let _ = std::io::stdout().flush();
            }
        }
        "h" => {
            state.show_log = false;
            state.log_lines.clear();
            draw(state);
        }
        _ => {}
    }
    true
}

// ── Supervisor queries ──────────────────────────────────────────────────────

fn query_ps() {
    println!("@supervisor: ps-raw");
    let _ = std::io::stdout().flush();
}

fn query_ps_raw() {
    println!("@supervisor: ps-raw");
    let _ = std::io::stdout().flush();
}

// ── Parser ──────────────────────────────────────────────────────────────────

fn parse_ps_raw(data: &str) -> Vec<AppRow> {
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

// ── Drawing ─────────────────────────────────────────────────────────────────

fn draw(state: &State) {
    fill(0, 0, W, H, C_BG);
    fill(PX, PY, PW, PH, C_PANEL);

    draw_title(state);
    draw_headers();
    draw_rows(state);

    if state.show_log {
        draw_log_pane(state);
    }

    draw_status_bar(state);
    flush_draw();
}

fn draw_title(state: &State) {
    fill(PX, PY, PW, TITLE_H, C_TITLE);
    fill(PX, PY + TITLE_H, PW, 2, C_ACCENT);
    text(PX + 10, PY + 6, C_ACCENT, "Process Monitor");

    let running = state.apps.iter().filter(|a| a.running).count();
    let total = state.apps.len();
    text(PX + 180, PY + 6, C_DIM, &format!("{running}/{total} running"));

    let max_uptime = state.apps.iter().map(|a| a.uptime).max().unwrap_or(0);
    text(
        PX + PW - 200,
        PY + 6,
        C_DIM,
        &format!("uptime: {}  tick #{}", fmt_uptime(max_uptime), state.tick),
    );
    border(PX, PY, PW, PH, C_ACCENT);
}

fn draw_headers() {
    let hy = PY + TITLE_H + 2;
    fill(INNER_X, hy, INNER_W, HDR_H, C_HDR);
    fill(INNER_X, hy + HDR_H - 1, INNER_W, 1, C_ACCENT);
    col_hdr("NAME", COL_NAME, hy);
    col_hdr("STATUS", COL_STATUS, hy);
    col_hdr("UPTIME", COL_UPTIME, hy);
    col_hdr("RESTARTS", COL_RESTARTS, hy);
}

fn draw_rows(state: &State) {
    let max = state.max_rows();
    let inner_h = max as u32 * ROW_H;

    // Clear table area
    fill(INNER_X, INNER_Y, INNER_W, inner_h, C_BG);

    if state.apps.is_empty() {
        text(
            INNER_X + INNER_W / 2 - 60,
            INNER_Y + inner_h / 2 - 8,
            C_DIM,
            "Waiting for data...",
        );
        return;
    }

    for (i, app) in state.apps.iter().take(max).enumerate() {
        let ry = INNER_Y + i as u32 * ROW_H;
        let is_selected = i == state.selected;

        // Row background
        let bg = if is_selected {
            C_SEL
        } else if i % 2 == 0 {
            C_ROW
        } else {
            C_ALT_ROW
        };
        fill(INNER_X, ry, INNER_W, ROW_H, bg);

        // Selection indicator
        if is_selected {
            fill(INNER_X, ry, 3, ROW_H, C_ACCENT);
        }

        // Name
        let name_c = if app.running { C_WHITE } else { C_DIM };
        text(INNER_X + COL_NAME + 6, ry + 3, name_c, &app.name);

        // Status
        let (st, sc) = if app.running {
            ("running", C_GREEN)
        } else {
            ("stopped", C_RED)
        };
        text(INNER_X + COL_STATUS + 4, ry + 3, sc, st);

        // Uptime
        text(
            INNER_X + COL_UPTIME + 4,
            ry + 3,
            C_DIM,
            &fmt_uptime(app.uptime),
        );

        // Restarts
        let rc = if app.restarts > 0 { C_YELLOW } else { C_DIM };
        text(
            INNER_X + COL_RESTARTS + 4,
            ry + 3,
            rc,
            &app.restarts.to_string(),
        );
    }

    if state.apps.len() > max {
        let oy = INNER_Y + max as u32 * ROW_H + 2;
        text(
            INNER_X + 4,
            oy,
            C_DIM,
            &format!("... {} more", state.apps.len() - max),
        );
    }
}

fn draw_log_pane(state: &State) {
    let ly = PY + PH - STATUS_H - LOG_PANE_H;
    fill(PX, ly, PW, LOG_PANE_H, C_LOG_BG);
    fill(PX, ly, PW, 1, C_ACCENT);

    let header = format!("Logs: {}  (h to hide)", state.log_target);
    text(INNER_X, ly + 4, C_ACCENT, &header);

    for (i, line) in state.log_lines.iter().enumerate() {
        let ty = ly + 20 + i as u32 * 16;
        if ty + 16 > ly + LOG_PANE_H {
            break;
        }
        // Truncate long lines
        let display = if line.len() > 75 {
            &line[..75]
        } else {
            line
        };
        text(INNER_X, ty, C_DIM, display);
    }
}

fn draw_status_bar(state: &State) {
    let sy = PY + PH - STATUS_H;
    fill(PX, sy, PW, STATUS_H, C_TITLE);
    fill(PX, sy, PW, 1, 0x30363DFF);

    let keys = if state.show_log {
        "Up/Down:select  k:kill  r:restart  f:focus  l:logs  h:hide  q:quit"
    } else {
        "Up/Down:select  k:kill  r:restart  f:focus  l:logs  q:quit"
    };
    text(INNER_X, sy + 3, C_DIM, keys);
}

fn col_hdr(label: &str, col_x: u32, hy: u32) {
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

// ── VYOMA_DRAW helpers ──────────────────────────────────────────────────────

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
fn flush_draw() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}
