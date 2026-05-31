// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

// Window dimensions
const W: u32 = 860;
const H: u32 = 700;

// Colors
const C_BG: u32      = 0x0D1117FF;
const C_HDR: u32     = 0x161B22FF;
const C_ACCENT: u32  = 0x58A6FFFF;
const C_DIM: u32     = 0x8B949EFF;
const C_TITLE: u32   = 0xFFFFFFFF;
const C_BORDER: u32  = 0x30363DFF;
const C_SEL: u32     = 0x1F4068FF;
const C_GRANTED: u32 = 0x3FB950FF;
const C_DENIED: u32  = 0xFF7B72FF;
const C_GRAY: u32    = 0x484F58FF;
const C_HINT: u32    = 0x6E7681FF;

const ROW_H: u32 = 22;
const LIST_Y: u32 = 72;
const HDR_Y: u32 = 48;
const VIS_ROWS: usize = ((H - LIST_Y - 36) / ROW_H) as usize;

const ALL_CAPS: &[&str] = &[
    "stdio", "filesystem", "network", "display", "shell", "mouse", "audio",
];

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn text_s(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},s,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

#[derive(Clone)]
struct AppEntry {
    name: String,
    status: String,
    caps: Vec<String>,
}

impl AppEntry {
    fn cap_state(&self, cap: &str) -> CapState {
        if self.caps.iter().any(|c| c == cap) {
            CapState::Granted
        } else {
            CapState::NotRequested
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum CapState {
    Granted,
    NotRequested,
}

#[derive(Clone, Copy, PartialEq)]
enum View {
    List,
    Detail,
}

struct State {
    apps: Vec<AppEntry>,
    cursor: usize,
    scroll: usize,
    view: View,
    detail_cap_idx: usize,
    last_poll: Instant,
    pending_caps: bool,
    caps_target: Option<String>,
}

impl State {
    fn new() -> Self {
        Self {
            apps: Vec::new(),
            cursor: 0,
            scroll: 0,
            view: View::List,
            detail_cap_idx: 0,
            last_poll: Instant::now() - Duration::from_secs(5),
            pending_caps: false,
            caps_target: None,
        }
    }

    fn selected_app(&self) -> Option<&AppEntry> {
        self.apps.get(self.cursor)
    }
}

fn parse_apps(reply: &str) -> Vec<AppEntry> {
    reply
        .split('|')
        .filter(|s| !s.is_empty())
        .filter_map(|part| {
            // Format: name:status:uptime:restarts
            let fields: Vec<&str> = part.splitn(4, ':').collect();
            if fields.len() < 2 {
                return None;
            }
            Some(AppEntry {
                name: fields[0].to_string(),
                status: fields[1].to_string(),
                caps: Vec::new(),
            })
        })
        .collect()
}

fn parse_caps(reply: &str) -> Vec<String> {
    // Expected: "cap1,cap2,cap3" or "stdio,display,shell"
    reply
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn draw_header() {
    fill(0, 0, W, 36, C_HDR);
    text(20, 10, C_ACCENT, "App Permissions");
    fill(0, 36, W, 1, C_BORDER);
}

fn draw_list(state: &State) {
    fill(0, 0, W, H, C_BG);
    draw_header();

    // Column headers
    fill(0, HDR_Y, W, ROW_H, C_HDR);
    text(12, HDR_Y + 4, C_DIM, "APP NAME");
    text(200, HDR_Y + 4, C_DIM, "STATUS");

    // Capability column headers
    let cap_start_x: u32 = 300;
    let cap_col_w: u32 = 76;
    for (i, cap) in ALL_CAPS.iter().enumerate() {
        let cx = cap_start_x + i as u32 * cap_col_w;
        text_s(cx, HDR_Y + 4, C_DIM, &cap.to_uppercase());
    }
    fill(0, HDR_Y + ROW_H, W, 1, C_BORDER);

    let visible = state.apps.iter().skip(state.scroll).take(VIS_ROWS);
    for (i, app) in visible.enumerate() {
        let ry = LIST_Y + i as u32 * ROW_H;
        let abs_idx = state.scroll + i;
        let is_sel = abs_idx == state.cursor;

        if is_sel {
            fill(0, ry, W, ROW_H, C_SEL);
        }

        let tc = if is_sel { C_TITLE } else { C_DIM };
        let name_disp = if app.name.len() > 20 {
            &app.name[..20]
        } else {
            &app.name
        };
        text(12, ry + 4, tc, name_disp);

        let sc = if app.status == "run" { C_GRANTED } else { C_DENIED };
        text(200, ry + 4, sc, &app.status);

        // Capability dots
        for (ci, cap) in ALL_CAPS.iter().enumerate() {
            let cx = cap_start_x + ci as u32 * cap_col_w;
            let (symbol, color) = match app.cap_state(cap) {
                CapState::Granted => ("ON", C_GRANTED),
                CapState::NotRequested => ("--", C_GRAY),
            };
            text_s(cx + 8, ry + 4, color, symbol);
        }
    }

    let total = state.apps.len();
    text(
        12,
        H - 24,
        C_HINT,
        &format!(
            "Up/Down: navigate  Enter: details  q: quit  ({total} apps)"
        ),
    );
    flush();
}

fn draw_detail(state: &State) {
    let app = match state.selected_app() {
        Some(a) => a,
        None => return,
    };

    fill(0, 0, W, H, C_BG);
    draw_header();

    // App name + status
    let sc = if app.status == "run" { C_GRANTED } else { C_DENIED };
    text(20, 50, C_TITLE, &format!("App: {}", app.name));
    text(20, 72, C_DIM, "Status:");
    text(120, 72, sc, &app.status);

    // Section title
    fill(20, 100, W - 40, 1, C_BORDER);
    text(20, 110, C_ACCENT, "Capabilities");

    // Capability rows
    let cap_y_start: u32 = 140;
    let cap_row_h: u32 = 32;

    for (i, cap) in ALL_CAPS.iter().enumerate() {
        let ry = cap_y_start + i as u32 * cap_row_h;
        let is_sel = i == state.detail_cap_idx;

        if is_sel {
            fill(20, ry, W - 40, cap_row_h - 2, C_SEL);
        }

        let cap_st = app.cap_state(cap);
        let (status_str, color) = match cap_st {
            CapState::Granted => ("GRANTED", C_GRANTED),
            CapState::NotRequested => ("NOT REQUESTED", C_GRAY),
        };

        let tc = if is_sel { C_TITLE } else { C_DIM };
        text(36, ry + 8, tc, cap);
        text(260, ry + 8, color, status_str);

        // Description
        let desc = cap_description(cap);
        text_s(460, ry + 8, C_HINT, desc);
    }

    // Actions hint
    let hint_y = cap_y_start + ALL_CAPS.len() as u32 * cap_row_h + 20;
    fill(20, hint_y, W - 40, 1, C_BORDER);
    text(20, hint_y + 12, C_HINT, "Up/Down: select capability");
    text(20, hint_y + 30, C_HINT, "r: revoke selected   g: grant selected");
    text(20, hint_y + 48, C_HINT, "q/Esc: back to list");

    // Action feedback area
    text(20, H - 24, C_DIM, "Permissions are enforced by the supervisor manifest.");

    flush();
}

fn cap_description(cap: &str) -> &'static str {
    match cap {
        "stdio" => "Console I/O (stdin/stdout)",
        "filesystem" => "Access to /data mount",
        "network" => "TCP/socket networking",
        "display" => "Framebuffer + VYOMA_DRAW",
        "shell" => "Supervisor IPC commands",
        "mouse" => "Mouse/pointer events",
        "audio" => "Audio playback/capture",
        _ => "Unknown capability",
    }
}

fn request_app_list() {
    println!("@supervisor: ps-raw");
    let _ = io::stdout().flush();
}

fn request_caps(app_name: &str) {
    println!("@supervisor: list-caps {app_name}");
    let _ = io::stdout().flush();
}

fn revoke_cap(app_name: &str, cap: &str) {
    println!("@supervisor: revoke-cap {app_name} {cap}");
    let _ = io::stdout().flush();
}

fn grant_cap(cap: &str) {
    println!("@supervisor: request-cap {cap}");
    let _ = io::stdout().flush();
}

fn poll_all_caps(state: &mut State) {
    if state.apps.is_empty() {
        return;
    }
    // Request caps for the first app that has no caps loaded yet,
    // or cycle through all
    if let Some(app) = state.apps.iter().find(|a| a.caps.is_empty()) {
        let name = app.name.clone();
        state.pending_caps = true;
        state.caps_target = Some(name.clone());
        request_caps(&name);
    } else {
        state.pending_caps = false;
    }
}

fn handle_key_list(state: &mut State, key: &str) -> bool {
    match key {
        "\x1b[A" => {
            if state.cursor > 0 {
                state.cursor -= 1;
                if state.cursor < state.scroll {
                    state.scroll = state.cursor;
                }
                draw_list(state);
            }
        }
        "\x1b[B" => {
            if state.cursor + 1 < state.apps.len() {
                state.cursor += 1;
                if state.cursor >= state.scroll + VIS_ROWS {
                    state.scroll = state.cursor + 1 - VIS_ROWS;
                }
                draw_list(state);
            }
        }
        "" => {
            if state.selected_app().is_some() {
                state.view = View::Detail;
                state.detail_cap_idx = 0;
                draw_detail(state);
            }
        }
        "\x03" | "q" => return true,
        _ => {}
    }
    false
}

fn handle_key_detail(state: &mut State, key: &str) {
    match key {
        "\x1b[A" => {
            if state.detail_cap_idx > 0 {
                state.detail_cap_idx -= 1;
                draw_detail(state);
            }
        }
        "\x1b[B" => {
            if state.detail_cap_idx + 1 < ALL_CAPS.len() {
                state.detail_cap_idx += 1;
                draw_detail(state);
            }
        }
        "r" => {
            if let Some(app) = state.selected_app() {
                let cap = ALL_CAPS[state.detail_cap_idx];
                let name = app.name.clone();
                revoke_cap(&name, cap);
                // Refresh after action
                std::thread::sleep(Duration::from_millis(300));
                request_caps(&name);
                state.caps_target = Some(name);
                state.pending_caps = true;
            }
        }
        "g" => {
            let cap = ALL_CAPS[state.detail_cap_idx];
            grant_cap(cap);
            // Refresh after action
            if let Some(app) = state.selected_app() {
                let name = app.name.clone();
                std::thread::sleep(Duration::from_millis(300));
                request_caps(&name);
                state.caps_target = Some(name);
                state.pending_caps = true;
            }
        }
        "\x1b" | "q" | "\x03" => {
            state.view = View::List;
            draw_list(state);
        }
        _ => {}
    }
}

fn main() {
    let stdin = io::stdin();
    let mut state = State::new();

    // Initial request
    request_app_list();

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle supervisor replies
        if let Some(rest) = raw.strip_prefix("REPLY:") {
            if let Some(caps_data) = rest.strip_prefix("list-caps ") {
                // Format: "appname cap1,cap2,cap3"
                let parts: Vec<&str> = caps_data.splitn(2, ' ').collect();
                if parts.len() == 2 {
                    let name = parts[0];
                    let caps = parse_caps(parts[1]);
                    if let Some(app) = state.apps.iter_mut().find(|a| a.name == name) {
                        app.caps = caps;
                    }
                }
                // Continue polling caps for remaining apps
                if state.pending_caps {
                    poll_all_caps(&mut state);
                }
                // Redraw current view
                match state.view {
                    View::List => draw_list(&state),
                    View::Detail => draw_detail(&state),
                }
                continue;
            }

            // ps-raw reply
            let new_apps = parse_apps(rest);
            if !new_apps.is_empty() {
                // Preserve existing caps data where app names match
                let merged: Vec<AppEntry> = new_apps
                    .into_iter()
                    .map(|mut new_app| {
                        if let Some(old) =
                            state.apps.iter().find(|a| a.name == new_app.name)
                        {
                            new_app.caps = old.caps.clone();
                        }
                        new_app
                    })
                    .collect();

                // Clamp cursor
                if state.cursor >= merged.len() && !merged.is_empty() {
                    state.cursor = merged.len() - 1;
                }
                state.apps = merged;
            }
            state.last_poll = Instant::now();

            // Start polling capabilities for all apps
            poll_all_caps(&mut state);

            match state.view {
                View::List => draw_list(&state),
                View::Detail => draw_detail(&state),
            }

            // Sleep before next auto-refresh
            std::thread::sleep(Duration::from_secs(3));
            request_app_list();
            continue;
        }

        // Keyboard input
        match state.view {
            View::List => {
                if handle_key_list(&mut state, &raw) {
                    fill(0, 0, W, H, C_BG);
                    flush();
                    std::process::exit(0);
                }
            }
            View::Detail => {
                handle_key_detail(&mut state, &raw);
            }
        }

        // Periodic auto-refresh
        if state.last_poll.elapsed() > Duration::from_secs(5) {
            request_app_list();
            state.last_poll = Instant::now();
        }
    }
}
