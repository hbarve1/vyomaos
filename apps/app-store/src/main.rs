// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1440;
const H: u32 = 880;
const C_BG: u32      = 0x0D1117FF;
const C_ACCENT: u32  = 0x58A6FFFF;
const C_DIM: u32     = 0x8B949EFF;
const C_TITLE: u32   = 0xFFFFFFFF;
const C_BORDER: u32  = 0x30363DFF;
const C_SEL: u32     = 0x1F4068FF;
const C_OK: u32      = 0x3FB950FF;
const C_ERR: u32     = 0xFF7B72FF;
const C_HINT: u32    = 0x6E7681FF;
const C_FIELD: u32   = 0x21262DFF;

const COLS: u32  = 4;
const CARD_W: u32 = 320;
const CARD_H: u32 = 80;
const GAP_X: u32 = 20;
const GAP_Y: u32 = 12;
const GRID_X: u32 = (W - COLS * CARD_W - (COLS - 1) * GAP_X) / 2;
const GRID_Y: u32 = 110;

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

#[derive(Clone)]
struct Package {
    name:      String,
    installed: bool,
}

fn parse_pkg_list(reply: &str) -> Vec<Package> {
    reply.split('|')
        .filter(|s| !s.is_empty() && (s.starts_with('[') ))
        .map(|entry| {
            let installed = entry.starts_with("[+]");
            let rest = if installed { &entry[4..] } else { &entry[4..] };
            let name = rest.split_whitespace().next().unwrap_or("").to_string();
            Package { name, installed }
        })
        .filter(|p| !p.name.is_empty())
        .collect()
}

struct State {
    all:      Vec<Package>,
    filter:   String,
    cursor:   usize,
    status:   String,
}

impl State {
    fn filtered(&self) -> Vec<&Package> {
        let f = self.filter.to_lowercase();
        self.all.iter()
            .filter(|p| f.is_empty() || p.name.to_lowercase().contains(&f))
            .collect()
    }
}

fn draw(s: &State) {
    fill(0, 0, W, H, C_BG);
    text(20, 16, C_ACCENT, "App Store");
    fill(0, 38, W, 1, C_BORDER);

    // Search bar
    fill(40, 52, 800, 32, C_FIELD);
    border(40, 52, 800, 32, C_BORDER);
    let search_text = if s.filter.is_empty() { "Search packages…" } else { &s.filter };
    let sc = if s.filter.is_empty() { C_HINT } else { C_TITLE };
    text(54, 60, sc, search_text);

    // Status message
    if !s.status.is_empty() {
        let color = if s.status.starts_with("Error") { C_ERR } else { C_OK };
        text(860, 60, color, &s.status);
    }

    // Grid legend
    text(40, 92, C_DIM, "[+] installed   [ ] available");
    text(W - 340, 92, C_HINT, "↑↓←→/Tab: nav   Enter: install/remove   Ctrl+C: quit");

    let visible = s.filtered();
    let rows = (visible.len() as u32 + COLS - 1) / COLS;
    let max_visible_rows = (H - GRID_Y - 36) / (CARD_H + GAP_Y);

    for (i, pkg) in visible.iter().enumerate() {
        let col = i as u32 % COLS;
        let row = i as u32 / COLS;
        if row >= max_visible_rows { break; }
        let cx = GRID_X + col * (CARD_W + GAP_X);
        let cy = GRID_Y + row * (CARD_H + GAP_Y);

        let is_sel = i == s.cursor;
        let bg = if is_sel { C_SEL } else { 0x161B22FF };
        let bc = if is_sel { C_ACCENT } else { C_BORDER };
        fill(cx, cy, CARD_W, CARD_H, bg);
        border(cx, cy, CARD_W, CARD_H, bc);

        let mark = if pkg.installed { "[+]" } else { "[ ]" };
        let mc = if pkg.installed { C_OK } else { C_DIM };
        text(cx + 10, cy + 14, mc, mark);
        text(cx + 52, cy + 14, C_TITLE, &pkg.name);

        let action = if pkg.installed { "→ Remove" } else { "→ Install" };
        let ac = if pkg.installed { C_ERR } else { C_ACCENT };
        if is_sel {
            text(cx + 10, cy + 48, ac, action);
        }
    }

    let _ = rows;
    text(20, H - 18, C_HINT, &format!("{} packages", visible.len()));
    flush();
}

fn request_lists() {
    println!("@supervisor: pkg-list");
    let _ = io::stdout().flush();
}

fn main() {
    let stdin = io::stdin();
    let mut s = State { all: Vec::new(), filter: String::new(), cursor: 0, status: String::new() };

    request_lists();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if let Some(rest) = raw.strip_prefix("REPLY:") {
            if rest.contains('[') {
                // pkg-list reply
                s.all = parse_pkg_list(rest);
                if s.cursor >= s.all.len() && !s.all.is_empty() {
                    s.cursor = s.all.len() - 1;
                }
            } else if !rest.starts_with("no packages") && !rest.is_empty() {
                // install/remove status reply
                s.status = rest.chars().take(60).collect();
                // re-query
                request_lists();
            } else {
                s.status = rest.chars().take(60).collect();
            }
            draw(&s);
            continue;
        }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\t" | "\x1b[C" => {
                let n = s.filtered().len();
                if n > 0 { s.cursor = (s.cursor + 1) % n; }
                s.status.clear();
                draw(&s);
            }
            "\x1b[A" => {
                if s.cursor >= COLS as usize { s.cursor -= COLS as usize; }
                s.status.clear();
                draw(&s);
            }
            "\x1b[B" => {
                let n = s.filtered().len();
                if s.cursor + (COLS as usize) < n { s.cursor += COLS as usize; }
                s.status.clear();
                draw(&s);
            }
            "\x1b[D" => {
                if s.cursor > 0 { s.cursor -= 1; }
                s.status.clear();
                draw(&s);
            }
            "\x7f" => {
                s.filter.pop();
                s.cursor = 0;
                s.status.clear();
                draw(&s);
            }
            "" => {
                // Enter: install or remove
                let visible = s.filtered();
                if let Some(pkg) = visible.get(s.cursor) {
                    let name = pkg.name.clone();
                    let installed = pkg.installed;
                    s.status = if installed {
                        format!("Removing {name}…")
                    } else {
                        format!("Installing {name}…")
                    };
                    draw(&s);
                    if installed {
                        println!("@supervisor: pkg-remove {name}");
                    } else {
                        println!("@supervisor: pkg-install {name}");
                    }
                    let _ = io::stdout().flush();
                }
            }
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if c.is_ascii_graphic() || c == ' ' {
                    s.filter.push(c);
                    s.cursor = 0;
                    s.status.clear();
                    draw(&s);
                }
            }
            _ => {}
        }
    }
}
