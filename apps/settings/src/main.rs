use std::io::{self, BufRead, Write};
use std::fs;

const W: u32 = 1440;
const H: u32 = 880;
const SB_W: u32 = 200;
const CONTENT_X: u32 = SB_W + 24;
const CONTENT_Y: u32 = 64;
const SETTINGS_PATH: &str = "/data/settings.toml";

const C_BG: u32     = 0x0D1117FF;
const C_SB: u32     = 0x161B22FF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_SEL: u32    = 0x1F4068FF;
const C_FIELD: u32  = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_OK: u32     = 0x3FB950FF;

const SECTIONS: &[&str] = &["Display", "Font", "Boot"];
const FONT_SIZES: &[u32] = &[8, 16, 32];

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

struct Settings {
    wallpaper: String,
    font_idx: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self { wallpaper: "0x0D1117FF".to_string(), font_idx: 1 }
    }
}

fn load_settings() -> Settings {
    let mut s = Settings::default();
    let content = fs::read_to_string(SETTINGS_PATH).unwrap_or_default();
    for line in content.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("wallpaper = ") {
            s.wallpaper = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("size = ") {
            if let Ok(n) = v.parse::<u32>() {
                if let Some(idx) = FONT_SIZES.iter().position(|&x| x == n) {
                    s.font_idx = idx;
                }
            }
        }
    }
    s
}

fn save_settings(s: &Settings) {
    let content = format!(
        "[display]\nwallpaper = \"{}\"\n\n[font]\nsize = {}\n",
        s.wallpaper, FONT_SIZES[s.font_idx]
    );
    let _ = fs::write(SETTINGS_PATH, content);
}

fn parse_rgba(s: &str) -> u32 {
    let hex = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")).unwrap_or(s);
    u32::from_str_radix(hex, 16).unwrap_or(0x0D1117FF)
}

fn draw_display(settings: &Settings, editing: bool, edit_buf: &str) {
    text(CONTENT_X, CONTENT_Y, C_DIM, "Wallpaper color (RGBA hex):");
    let fy = CONTENT_Y + 26;
    fill(CONTENT_X, fy, 320, 34, C_FIELD);
    border(CONTENT_X, fy, 320, 34, if editing { C_ACCENT } else { C_BORDER });
    let display = if editing { edit_buf } else { &settings.wallpaper };
    text(CONTENT_X + 10, fy + 9, C_TITLE, display);
    if editing {
        text(CONTENT_X, fy + 44, C_HINT, "Enter: confirm  Esc: cancel  Backspace: delete");
    } else {
        text(CONTENT_X, fy + 44, C_HINT, "Enter: edit field");
    }
    // Preview swatch
    text(CONTENT_X + 344, CONTENT_Y, C_DIM, "Preview:");
    fill(CONTENT_X + 344, fy, 80, 34, parse_rgba(&settings.wallpaper));
    border(CONTENT_X + 344, fy, 80, 34, C_BORDER);
}

fn draw_font(settings: &Settings) {
    text(CONTENT_X, CONTENT_Y, C_DIM, "Font size:");
    for (i, &sz) in FONT_SIZES.iter().enumerate() {
        let bx = CONTENT_X + i as u32 * 110;
        let by = CONTENT_Y + 28;
        let bg = if i == settings.font_idx { C_SEL } else { C_FIELD };
        let bc = if i == settings.font_idx { C_ACCENT } else { C_BORDER };
        fill(bx, by, 100, 40, bg);
        border(bx, by, 100, 40, bc);
        let label = format!("{sz}px");
        text(bx + 34, by + 12, C_TITLE, &label);
    }
    text(CONTENT_X, CONTENT_Y + 82, C_HINT, "Left/Right arrows: select size");
}

fn draw_boot() {
    text(CONTENT_X, CONTENT_Y, C_DIM, "/data/settings.toml:");
    let content = fs::read_to_string(SETTINGS_PATH)
        .unwrap_or_else(|_| "(not yet saved — press Ctrl+W to create)".to_string());
    let mut ly = CONTENT_Y + 26;
    for line in content.lines().take(24) {
        let color = if line.starts_with('[') { C_ACCENT } else { C_TITLE };
        text(CONTENT_X, ly, color, line);
        ly += 20;
    }
}

fn draw(section: usize, settings: &Settings, editing: bool, edit_buf: &str, saved: bool) {
    fill(0, 0, W, H, C_BG);

    // Sidebar
    fill(0, 0, SB_W, H, C_SB);
    text(16, 18, C_ACCENT, "Settings");
    fill(0, 44, SB_W, 1, C_BORDER);
    for (i, &name) in SECTIONS.iter().enumerate() {
        let sy = 56 + i as u32 * 48;
        let bg = if i == section { C_SEL } else { C_SB };
        fill(0, sy, SB_W, 44, bg);
        let lc = if i == section { C_TITLE } else { C_DIM };
        text(18, sy + 14, lc, name);
    }

    // Content area
    text(CONTENT_X, 18, C_TITLE, SECTIONS[section]);
    fill(CONTENT_X, 44, W - CONTENT_X - 20, 1, C_BORDER);

    match section {
        0 => draw_display(settings, editing, edit_buf),
        1 => draw_font(settings),
        2 => draw_boot(),
        _ => {}
    }

    // Footer bar
    fill(0, H - 34, W, 34, C_SB);
    let (msg, mc) = if saved {
        ("Saved to /data/settings.toml", C_OK)
    } else {
        ("Ctrl+W: save  Tab: switch section  Ctrl+C: quit", C_HINT)
    };
    text(CONTENT_X, H - 22, mc, msg);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut settings = load_settings();
    let mut section: usize = 0;
    let mut editing = false;
    let mut edit_buf = String::new();
    let mut saved = false;

    draw(section, &settings, editing, &edit_buf, saved);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if raw.starts_with("REPLY:") { continue; }

        // Ctrl+C — quit
        if raw == "\x03" {
            fill(0, 0, W, H, 0x0D1117FF);
            flush();
            std::process::exit(0);
        }

        // Ctrl+W — save
        if raw == "\x17" {
            if editing {
                if !edit_buf.is_empty() {
                    settings.wallpaper = edit_buf.clone();
                }
                edit_buf.clear();
                editing = false;
            }
            save_settings(&settings);
            println!("@supervisor: wallpaper {}", settings.wallpaper);
            let _ = io::stdout().flush();
            saved = true;
            draw(section, &settings, editing, &edit_buf, saved);
            continue;
        }

        // Tab — switch section
        if raw == "\t" {
            section = (section + 1) % SECTIONS.len();
            editing = false;
            edit_buf.clear();
            saved = false;
            draw(section, &settings, editing, &edit_buf, saved);
            continue;
        }

        // Escape
        if raw == "\x1b" {
            if editing {
                editing = false;
                edit_buf.clear();
                saved = false;
                draw(section, &settings, editing, &edit_buf, saved);
            }
            continue;
        }

        // Arrow keys
        if raw == "\x1b[A" || raw == "\x1b[B" || raw == "\x1b[C" || raw == "\x1b[D" {
            if section == 1 && !editing {
                if (raw == "\x1b[A" || raw == "\x1b[D") && settings.font_idx > 0 {
                    settings.font_idx -= 1;
                } else if (raw == "\x1b[B" || raw == "\x1b[C")
                    && settings.font_idx + 1 < FONT_SIZES.len()
                {
                    settings.font_idx += 1;
                }
                saved = false;
                draw(section, &settings, editing, &edit_buf, saved);
            }
            continue;
        }

        // Enter
        if raw.is_empty() {
            if section == 0 {
                if editing {
                    if !edit_buf.is_empty() {
                        settings.wallpaper = edit_buf.clone();
                    }
                    edit_buf.clear();
                    editing = false;
                } else {
                    edit_buf = settings.wallpaper.clone();
                    editing = true;
                }
                saved = false;
                draw(section, &settings, editing, &edit_buf, saved);
            }
            continue;
        }

        // Backspace
        if raw == "\x7f" {
            if editing {
                edit_buf.pop();
                draw(section, &settings, editing, &edit_buf, saved);
            }
            continue;
        }

        // Printable single char
        if raw.len() == 1 {
            let ch = raw.chars().next().unwrap();
            if editing && (ch.is_ascii_hexdigit() || "0xX#".contains(ch)) {
                edit_buf.push(ch);
                draw(section, &settings, editing, &edit_buf, saved);
            }
        }
    }
}
