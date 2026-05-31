// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P47: App Store — catalog browser with category tabs, install/uninstall via
//! supervisor IPC, and persistent tracking via /data/installed.txt.

use std::io::{self, BufRead, Write};

// ── Layout constants ────────────────────────────────────────────────────────
const W: u32 = 1440;
const H: u32 = 900;
const HEADER_H: u32 = 56;
const TAB_Y: u32 = 64;
const TAB_H: u32 = 32;
const LIST_Y: u32 = 108;
const CARD_H: u32 = 60;
const CARD_GAP: u32 = 4;
const CARD_X: u32 = 40;
const CARD_W: u32 = W - 80;

// ── Colors ──────────────────────────────────────────────────────────────────
const C_BG: u32        = 0x0D1117FF;
const C_HEADER: u32    = 0x161B22FF;
const C_TITLE: u32     = 0xFFFFFFFF;
const C_ACCENT: u32    = 0x58A6FFFF;
const C_CARD: u32      = 0x161B22FF;
const C_SEL: u32       = 0x1F6A4EFF;
const C_HINT: u32      = 0x8B949EFF;
const C_INSTALLED: u32 = 0x3FB950FF;
const C_TAB_BG: u32    = 0x21262DFF;
const C_TAB_SEL: u32   = 0x30363DFF;
const C_BTN_INST: u32  = 0x238636FF;
const C_STATUS: u32    = 0xF0C000FF;
const C_ERR: u32       = 0xFF7B72FF;
const C_BORDER: u32    = 0x30363DFF;

// ── Catalog ─────────────────────────────────────────────────────────────────
struct AppEntry {
    name: &'static str,
    desc: &'static str,
    version: &'static str,
    size: &'static str,
    category: Category,
}

#[derive(Clone, Copy, PartialEq)]
enum Category { All, Productivity, Games, Utilities, System }

const CATEGORIES: &[Category] = &[
    Category::All, Category::Productivity, Category::Games,
    Category::Utilities, Category::System,
];

fn cat_label(c: Category) -> &'static str {
    match c {
        Category::All          => "All",
        Category::Productivity => "Productivity",
        Category::Games        => "Games",
        Category::Utilities    => "Utilities",
        Category::System       => "System",
    }
}

const CATALOG: &[AppEntry] = &[
    AppEntry { name: "calculator",   desc: "Basic arithmetic calculator",     version: "0.1.0", size: "4 KB",  category: Category::Productivity },
    AppEntry { name: "notes",        desc: "Simple note-taking app",          version: "0.1.0", size: "4 KB",  category: Category::Productivity },
    AppEntry { name: "factorial",    desc: "Recursive factorial computation", version: "0.1.0", size: "2 KB",  category: Category::Utilities },
    AppEntry { name: "gui-demo",     desc: "Framebuffer dashboard demo",      version: "0.1.0", size: "6 KB",  category: Category::Utilities },
    AppEntry { name: "hello-world",  desc: "Hello World demo app",            version: "0.1.0", size: "1 KB",  category: Category::Utilities },
    AppEntry { name: "ping",         desc: "IPC demo - send side",            version: "0.1.0", size: "2 KB",  category: Category::Games },
    AppEntry { name: "pong",         desc: "IPC demo - recv side",            version: "0.1.0", size: "2 KB",  category: Category::Games },
    AppEntry { name: "http-server",  desc: "HTTP status server on :8080",     version: "0.1.0", size: "5 KB",  category: Category::System },
    AppEntry { name: "storage-demo", desc: "Persistent storage demo",         version: "0.1.0", size: "3 KB",  category: Category::System },
    AppEntry { name: "shell",        desc: "Interactive VyomaOS shell",       version: "0.1.0", size: "8 KB",  category: Category::System },
];

// ── Draw helpers ────────────────────────────────────────────────────────────
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn text_s(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},s,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

// ── Installed tracking via /data/installed.txt ──────────────────────────────
fn read_installed() -> Vec<String> {
    std::fs::read_to_string("/data/installed.txt")
        .unwrap_or_default()
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn is_installed(name: &str, list: &[String]) -> bool {
    list.iter().any(|n| n == name)
}

fn add_to_installed(name: &str) {
    use std::fs::OpenOptions;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("/data/installed.txt") {
        let _ = writeln!(f, "{name}");
    }
}

fn remove_from_installed(name: &str) {
    let kept: Vec<String> = read_installed().into_iter().filter(|n| n != name).collect();
    let out = if kept.is_empty() { String::new() } else { kept.join("\n") + "\n" };
    let _ = std::fs::write("/data/installed.txt", out);
}

// ── Filtered catalog view ───────────────────────────────────────────────────
fn visible(cat: Category) -> Vec<usize> {
    CATALOG.iter().enumerate()
        .filter(|(_, a)| cat == Category::All || a.category == cat)
        .map(|(i, _)| i)
        .collect()
}

// ── UI rendering ────────────────────────────────────────────────────────────
fn draw_ui(cat_idx: usize, sel: usize, installed: &[String], status: &str) {
    let cat = CATEGORIES[cat_idx];
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    text(40, 18, C_TITLE, "App Store");
    text(W - 340, 18, C_HINT, "VyomaOS Package Manager");
    fill(0, HEADER_H, W, 1, C_BORDER);

    // Category tabs
    draw_tabs(cat_idx);

    // App list
    let idxs = visible(cat);
    let max_rows = ((H - LIST_Y - 56) / (CARD_H + CARD_GAP)) as usize;
    let scroll = if sel >= max_rows { sel - max_rows + 1 } else { 0 };

    for (vi, &ci) in idxs.iter().enumerate().skip(scroll).take(max_rows) {
        let app = &CATALOG[ci];
        let cy = LIST_Y + ((vi - scroll) as u32) * (CARD_H + CARD_GAP);
        let is_sel = vi == sel;
        let inst = is_installed(app.name, installed);
        draw_card(cy, app, is_sel, inst);
    }

    // Status bar
    if !status.is_empty() {
        let sc = if status.starts_with("Error") { C_ERR } else { C_STATUS };
        fill(CARD_X, H - 76, CARD_W, 24, C_HEADER);
        text(CARD_X + 12, H - 72, sc, status);
    }

    // Footer
    let help = "Up/Down: select  Left/Right: category  Enter: install/launch  u: uninstall  r: refresh  Esc: exit";
    text_s(CARD_X, H - 40, C_HINT, help);
    text_s(CARD_X, H - 24, C_HINT, &format!("{} apps in catalog", idxs.len()));

    flush();
}

fn draw_tabs(cat_idx: usize) {
    let mut tx = CARD_X;
    for (i, &c) in CATEGORIES.iter().enumerate() {
        let label = cat_label(c);
        let tw = (label.len() as u32) * 8 + 24;
        let bg = if i == cat_idx { C_TAB_SEL } else { C_TAB_BG };
        let fg = if i == cat_idx { C_ACCENT } else { C_HINT };
        fill(tx, TAB_Y, tw, TAB_H, bg);
        if i == cat_idx {
            border(tx, TAB_Y, tw, TAB_H, C_ACCENT);
        }
        text(tx + 12, TAB_Y + 8, fg, label);
        tx += tw + 8;
    }
}

fn draw_card(cy: u32, app: &AppEntry, is_sel: bool, inst: bool) {
    let card_bg = if is_sel { C_SEL } else { C_CARD };
    fill(CARD_X, cy, CARD_W, CARD_H, card_bg);
    if is_sel {
        border(CARD_X, cy, CARD_W, CARD_H, C_ACCENT);
    }

    // Icon placeholder
    let icon_c = if inst { C_INSTALLED } else { C_ACCENT };
    fill(CARD_X + 10, cy + 10, 40, 40, icon_c);
    let initial = &app.name[..1].to_uppercase();
    text(CARD_X + 22, cy + 22, C_TITLE, &initial);

    // Name + version
    text(CARD_X + 64, cy + 10, C_TITLE, &format!("{}  v{}", app.name, app.version));

    // Description + size
    text_s(CARD_X + 64, cy + 34, C_HINT, &format!("{}  ({})", app.desc, app.size));

    // Badge
    if inst {
        let bx = CARD_X + CARD_W - 120;
        fill(bx, cy + 16, 100, 26, C_INSTALLED);
        text(bx + 10, cy + 20, C_TITLE, "Installed");
    } else if is_sel {
        let bx = CARD_X + CARD_W - 100;
        fill(bx, cy + 16, 80, 26, C_BTN_INST);
        text(bx + 10, cy + 20, C_TITLE, "Install");
    }
}

fn clear_exit() {
    fill(0, 0, W, H, 0x0D1117FF);
    flush();
    std::process::exit(0);
}

// ── Main event loop ─────────────────────────────────────────────────────────
fn main() {
    let stdin = io::stdin();
    let mut cat_idx: usize = 0;
    let mut sel: usize = 0;
    let mut installed = read_installed();
    let mut status = String::new();

    draw_ui(cat_idx, sel, &installed, &status);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        // IPC replies from supervisor
        if let Some(reply) = raw.strip_prefix("REPLY:") {
            if reply.starts_with("installed ") {
                status = reply.to_string();
                installed = read_installed();
            } else if reply.starts_with("removed ") {
                status = reply.to_string();
                installed = read_installed();
            } else if reply.starts_with("error:") {
                status = format!("Error: {}", reply[6..].trim());
            } else if !reply.starts_with("no packages") {
                status = reply.chars().take(80).collect();
            }
            draw_ui(cat_idx, sel, &installed, &status);
            continue;
        }

        let cat = CATEGORIES[cat_idx];
        let count = visible(cat).len();

        match raw.as_str() {
            // Escape / Ctrl+C — exit
            "\x1b" | "\x1b[" | "\x03" => clear_exit(),

            // Arrow Up
            "\x1b[A" => {
                if sel > 0 { sel -= 1; }
                status.clear();
                draw_ui(cat_idx, sel, &installed, &status);
            }
            // Arrow Down
            "\x1b[B" => {
                if sel + 1 < count { sel += 1; }
                status.clear();
                draw_ui(cat_idx, sel, &installed, &status);
            }
            // Arrow Left — previous category
            "\x1b[D" => {
                if cat_idx > 0 { cat_idx -= 1; } else { cat_idx = CATEGORIES.len() - 1; }
                sel = 0;
                status.clear();
                draw_ui(cat_idx, sel, &installed, &status);
            }
            // Arrow Right — next category
            "\x1b[C" => {
                cat_idx = (cat_idx + 1) % CATEGORIES.len();
                sel = 0;
                status.clear();
                draw_ui(cat_idx, sel, &installed, &status);
            }

            // Enter — install or launch
            "" => {
                let idxs = visible(cat);
                if let Some(&ci) = idxs.get(sel) {
                    let app = &CATALOG[ci];
                    if is_installed(app.name, &installed) {
                        // Already installed — launch
                        println!("@supervisor: run /apps/{}/vyoma.toml", app.name);
                        let _ = io::stdout().flush();
                        println!("@supervisor: focus {}", app.name);
                        let _ = io::stdout().flush();
                        status = format!("Launched {}", app.name);
                    } else {
                        // Install via supervisor pkg-install
                        status = format!("Installing {}...", app.name);
                        draw_ui(cat_idx, sel, &installed, &status);
                        println!("@supervisor: pkg-install {}", app.name);
                        let _ = io::stdout().flush();
                        add_to_installed(app.name);
                        installed = read_installed();
                    }
                    draw_ui(cat_idx, sel, &installed, &status);
                }
            }

            // 'u' — uninstall selected
            "u" => {
                let idxs = visible(cat);
                if let Some(&ci) = idxs.get(sel) {
                    let app = &CATALOG[ci];
                    if is_installed(app.name, &installed) {
                        status = format!("Removing {}...", app.name);
                        draw_ui(cat_idx, sel, &installed, &status);
                        println!("@supervisor: pkg-remove {}", app.name);
                        let _ = io::stdout().flush();
                        remove_from_installed(app.name);
                        installed = read_installed();
                        status = format!("Removed {}", app.name);
                    } else {
                        status = format!("{} is not installed", app.name);
                    }
                    draw_ui(cat_idx, sel, &installed, &status);
                }
            }

            // 'r' — refresh installed list
            "r" => {
                installed = read_installed();
                status = "Catalog refreshed".to_string();
                draw_ui(cat_idx, sel, &installed, &status);
            }

            _ => {}
        }
    }
}
