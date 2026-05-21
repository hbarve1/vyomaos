use std::io::{self, BufRead, Write};

const W: u32 = 900;
const H: u32 = 700;
const SIDEBAR_W: u32 = 200;
const HEADER_H: u32  = 56;
const STATUS_H: u32  = 28;
const CONTENT_W: u32 = W - SIDEBAR_W;
const CONTENT_H: u32 = H - HEADER_H - STATUS_H;

const C_BG: u32      = 0x161B22FF;
const C_SIDEBAR: u32 = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_DIM: u32     = 0x8B949EFF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_ORANGE: u32  = 0xFFA657FF;

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

const PANES: &[(&str, &str, u32)] = &[
    ("Appearance",  "appearance",  0x58A6FFFF),
    ("Display",     "display",     0x79C0FFFF),
    ("Sound",       "sound",       0xBC8CFFFF),
    ("Network",     "network",     0x3FB950FF),
    ("Security",    "security",    0xFFA657FF),
    ("About",       "about",       0x8B949EFF),
];

#[derive(Clone)]
struct Settings {
    dark_mode:    bool,
    accent:       usize,
    transparency: bool,
    font_size:    usize,
    muted:        bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { dark_mode: true, accent: 0, transparency: true, font_size: 1, muted: false }
    }
}

const ACCENTS: &[(&str, u32)] = &[
    ("Blue",   0x58A6FFFF),
    ("Green",  0x3FB950FF),
    ("Orange", 0xFFA657FF),
    ("Pink",   0xF78166FF),
    ("Purple", 0xBC8CFFFF),
    ("Red",    0xFF7B72FF),
    ("Gray",   0x8B949EFF),
];

const FONT_SIZES: &[&str] = &["Small", "Medium", "Large"];

fn draw_pane_list(cursor: usize, open_pane: Option<usize>) {
    fill(0, HEADER_H, SIDEBAR_W, CONTENT_H + STATUS_H, C_SIDEBAR);
    fill(SIDEBAR_W, HEADER_H, 1, H - HEADER_H, C_BORDER);

    text(12, HEADER_H + 12, C_HINT, "PREFERENCES");

    for (i, &(name, _, color)) in PANES.iter().enumerate() {
        let py = HEADER_H + 36 + i as u32 * 48;
        let is_sel = i == cursor;
        let is_open = open_pane == Some(i);
        if is_sel || is_open {
            fill(0, py - 4, SIDEBAR_W, 44, C_SEL_BG);
            fill(0, py - 4, 3, 44, C_SEL);
        }
        // Icon dot
        fill(16, py + 12, 18, 18, color);
        let tc = if is_sel || is_open { C_TEXT } else { C_DIM };
        text(42, py + 12, tc, name);
    }
}

fn draw_appearance(s: &Settings, field: usize) {
    let cx = SIDEBAR_W + 20;
    let mut y = HEADER_H + 20;

    text(cx, y, C_TEXT, "Appearance");
    y += 36;

    // Dark mode
    let dm_on = if s.dark_mode { C_GREEN } else { C_BORDER };
    fill(cx, y, 40, 20, dm_on);
    border(cx, y, 40, 20, C_BORDER);
    fill(cx + if s.dark_mode { 22 } else { 2 }, y + 2, 16, 16, C_TEXT);
    let f0 = if field == 0 { C_TEXT } else { C_DIM };
    text(cx + 52, y + 4, f0, "Dark Mode");
    y += 36;

    // Accent color
    let f1 = if field == 1 { C_TEXT } else { C_DIM };
    text(cx, y, f1, "Accent Color:");
    for (j, &(name, color)) in ACCENTS.iter().enumerate() {
        let ax = cx + 130 + j as u32 * 32;
        fill(ax, y, 24, 24, color);
        if j == s.accent { border(ax, y, 24, 24, C_TEXT); }
    }
    y += 36;

    // Transparency
    let f2 = if field == 2 { C_TEXT } else { C_DIM };
    let tr_on = if s.transparency { C_GREEN } else { C_BORDER };
    fill(cx, y, 40, 20, tr_on);
    border(cx, y, 40, 20, C_BORDER);
    fill(cx + if s.transparency { 22 } else { 2 }, y + 2, 16, 16, C_TEXT);
    text(cx + 52, y + 4, f2, "Transparency Effects");
}

fn draw_display(s: &Settings, field: usize) {
    let cx = SIDEBAR_W + 20;
    let mut y = HEADER_H + 20;

    text(cx, y, C_TEXT, "Display");
    y += 36;
    text(cx, y, C_DIM, "Resolution: 1440 x 900 (virtio-gpu)");
    y += 32;
    text(cx, y, C_DIM, "Color depth: 32-bit RGBA");
    y += 36;

    let f0 = if field == 0 { C_TEXT } else { C_DIM };
    text(cx, y, f0, "Font Size:");
    for (j, &sz) in FONT_SIZES.iter().enumerate() {
        let bx = cx + 120 + j as u32 * 90;
        let bg = if j == s.font_size { C_SEL_BG } else { 0x21262DFF };
        fill(bx, y - 4, 80, 28, bg);
        border(bx, y - 4, 80, 28, if j == s.font_size { C_SEL } else { C_BORDER });
        text(bx + 10, y + 4, if j == s.font_size { C_TEXT } else { C_DIM }, sz);
    }
}

fn draw_sound(s: &Settings, field: usize) {
    let cx = SIDEBAR_W + 20;
    let mut y = HEADER_H + 20;

    text(cx, y, C_TEXT, "Sound");
    y += 36;
    text(cx, y, C_HINT, "(Audio output not yet supported)");
    y += 36;

    let f0 = if field == 0 { C_TEXT } else { C_DIM };
    let m_on = if s.muted { C_ORANGE } else { C_GREEN };
    fill(cx, y, 40, 20, m_on);
    border(cx, y, 40, 20, C_BORDER);
    fill(cx + if !s.muted { 22 } else { 2 }, y + 2, 16, 16, C_TEXT);
    text(cx + 52, y + 4, f0, if s.muted { "Muted" } else { "Unmuted" });
}

fn draw_network(_s: &Settings, _field: usize) {
    let cx = SIDEBAR_W + 20;
    let mut y = HEADER_H + 20;
    text(cx, y, C_TEXT, "Network");
    y += 36;
    text(cx, y, C_DIM, "Interface: eth0 (virtio-net)");
    y += 28;
    text(cx, y, C_DIM, "DHCP: enabled");
    y += 28;
    text(cx, y, C_DIM, "Port forward: 8080 (host) → 8080 (guest)");
    y += 36;
    text(cx, y, C_HINT, "Press Enter to open Network Config");
}

fn draw_security(_s: &Settings, _field: usize) {
    let cx = SIDEBAR_W + 20;
    let mut y = HEADER_H + 20;
    text(cx, y, C_TEXT, "Security");
    y += 36;
    // Status items
    let items = &[
        ("WASM Sandbox",         "Active",   C_GREEN),
        ("Capability Manifest",  "Enforced", C_GREEN),
        ("seccomp BPF",          "Enabled",  C_GREEN),
        ("App Signatures",       "Verified", C_GREEN),
        ("Filesystem Isolation", "9P mount", C_GREEN),
    ];
    for &(label, status, color) in items.iter() {
        fill(cx, y, 12, 12, color);
        text(cx + 20, y, C_DIM, label);
        text(cx + 220, y, color, status);
        y += 28;
    }
}

fn draw_about(_s: &Settings, _field: usize) {
    let cx = SIDEBAR_W + 20;
    let mut y = HEADER_H + 20;
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},l,VyomaOS", cx, y, C_TEXT);
    y += 48;
    let rows = &[
        ("Version",   "0.80.0 (develop)"),
        ("Runtime",   "Wasmtime 43.0.0 (WASI Preview 2)"),
        ("Supervisor","Rust (static musl, x86_64)"),
        ("Kernel",    "Linux 5.10 (allnoconfig)"),
        ("Display",   "virtio-gpu DRM, 1440×900"),
        ("Storage",   "9P virtio (/data, 64 MB)"),
        ("Built",     "2026-05-21"),
    ];
    for &(label, val) in rows.iter() {
        text(cx, y, C_DIM, label);
        text(cx + 140, y, C_TEXT, val);
        y += 28;
    }
}

fn draw_content(pane: usize, s: &Settings, field: usize) {
    fill(SIDEBAR_W + 1, HEADER_H, CONTENT_W - 1, CONTENT_H, C_BG);
    match pane {
        0 => draw_appearance(s, field),
        1 => draw_display(s, field),
        2 => draw_sound(s, field),
        3 => draw_network(s, field),
        4 => draw_security(s, field),
        5 => draw_about(s, field),
        _ => {}
    }
}

fn max_fields(pane: usize) -> usize {
    match pane { 0 => 3, 1 => 1, 2 => 1, _ => 0 }
}

fn draw_all(sidebar_cursor: usize, open_pane: Option<usize>, s: &Settings, field: usize) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,System Preferences", 20, 20, C_TEXT);
    if open_pane.is_some() {
        text(W - 120, 20, C_HINT, "Bksp: back  Ctrl+W: save");
    } else {
        text(W - 180, 20, C_HINT, "↑↓: navigate  Enter: open  Esc: close");
    }

    draw_pane_list(sidebar_cursor, open_pane);

    if let Some(p) = open_pane {
        draw_content(p, s, field);
    } else {
        fill(SIDEBAR_W + 1, HEADER_H, CONTENT_W - 1, CONTENT_H, C_BG);
        text(SIDEBAR_W + 40, HEADER_H + 40, C_HINT, "Select a preference pane from the list");
    }

    // Status
    fill(0, H - STATUS_H, W, STATUS_H, C_HEADER);
    fill(0, H - STATUS_H, W, 1, C_BORDER);
    text(SIDEBAR_W + 12, H - STATUS_H + 8, C_HINT,
        if open_pane.is_some() { "Tab: next field  Space/←→: toggle/change  Ctrl+W: save" }
        else { "" });

    flush();
}

fn save_settings(s: &Settings) {
    let content = format!(
        "dark_mode = {}\naccent = {}\ntransparency = {}\nfont_size = {}\nmuted = {}\n",
        s.dark_mode, s.accent, s.transparency, s.font_size, s.muted
    );
    // Write via supervisor (filesystem capability needed)
    println!("@supervisor: write-file /data/system-prefs.toml {}", content.replace('\n', "\\n"));
    let _ = io::stdout().flush();
}

fn main() {
    let stdin = io::stdin();
    let mut sidebar_cursor = 0usize;
    let mut open_pane: Option<usize> = None;
    let mut settings = Settings::default();
    let mut field = 0usize;

    println!("@supervisor: raise system-preferences");
    let _ = io::stdout().flush();

    draw_all(sidebar_cursor, open_pane, &settings, field);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                if open_pane.is_some() {
                    open_pane = None;
                    field = 0;
                } else {
                    fill(0, 0, W, H, 0x0D1117FF);
                    flush();
                    std::process::exit(0);
                }
            }
            "\x7f" => {
                if open_pane.is_some() { open_pane = None; field = 0; }
            }
            "\x09" => { // Tab
                if let Some(p) = open_pane {
                    let mf = max_fields(p);
                    if mf > 0 { field = (field + 1) % mf; }
                }
            }
            "\x17" => { // Ctrl+W — save
                save_settings(&settings);
                if let Some(p) = open_pane {
                    if p == 1 {
                        let sz = ["s", "m", "l"][settings.font_size];
                        println!("@supervisor: font-size {sz}");
                        let _ = io::stdout().flush();
                    }
                }
            }
            "\x1b[A" => {
                if open_pane.is_none() && sidebar_cursor > 0 { sidebar_cursor -= 1; }
            }
            "\x1b[B" => {
                if open_pane.is_none() && sidebar_cursor + 1 < PANES.len() { sidebar_cursor += 1; }
            }
            "\x1b[D" | "\x1b[C" => {
                if let Some(p) = open_pane {
                    match (p, field) {
                        (0, 0) => settings.dark_mode = !settings.dark_mode,
                        (0, 1) => {
                            let n = ACCENTS.len();
                            if raw == "\x1b[C" { settings.accent = (settings.accent + 1) % n; }
                            else if settings.accent > 0 { settings.accent -= 1; }
                        }
                        (0, 2) => settings.transparency = !settings.transparency,
                        (1, 0) => {
                            let n = FONT_SIZES.len();
                            if raw == "\x1b[C" { settings.font_size = (settings.font_size + 1) % n; }
                            else if settings.font_size > 0 { settings.font_size -= 1; }
                        }
                        (2, 0) => settings.muted = !settings.muted,
                        _ => {}
                    }
                }
            }
            " " => {
                if let Some(p) = open_pane {
                    match (p, field) {
                        (0, 0) => settings.dark_mode = !settings.dark_mode,
                        (0, 2) => settings.transparency = !settings.transparency,
                        (2, 0) => settings.muted = !settings.muted,
                        _ => {}
                    }
                }
            }
            "" => {
                if let Some(p) = open_pane {
                    if p == 3 {
                        println!("@supervisor: run network-config");
                        let _ = io::stdout().flush();
                    }
                } else {
                    open_pane = Some(sidebar_cursor);
                    field = 0;
                }
            }
            _ => {}
        }

        draw_all(sidebar_cursor, open_pane, &settings, field);
    }
}
