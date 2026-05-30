// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::fs;
use std::io::{self, BufRead, Write};

const W: u32 = 900;
const H: u32 = 700;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const ROW_H: u32 = 36;
const SECTION_GAP: u32 = 16;
const LEFT_PAD: u32 = 24;

const C_BG: u32 = 0x1C1C1EFF;
const C_HEADER: u32 = 0x2C2C2EFF;
const C_BORDER: u32 = 0x3A3A3CFF;
const C_TEXT: u32 = 0xFFFFFFFF;
const C_DIM: u32 = 0x8E8E93FF;
const C_HINT: u32 = 0x636366FF;
const C_SEL_BG: u32 = 0x0A84FFFF;
const C_SECTION: u32 = 0xAEAEB2FF;
const C_VALUE: u32 = 0x30D158FF;
const C_STATUS_BG: u32 = 0x2C2C2EFF;
const C_WARN: u32 = 0xFF453AFF;
const C_DIALOG_BG: u32 = 0x2C2C2EFF;
const C_OVERLAY: u32 = 0x000000AA;
const SETTINGS_PATH: &str = "/data/settings.toml";

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

struct Settings {
    wallpaper_color: u32,
    font_size: u32,
    theme: String,
    boot_apps: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            wallpaper_color: 0x1C1C1EFF,
            font_size: 13,
            theme: "dark".to_string(),
            boot_apps: vec![
                "desktop".into(),
                "dock".into(),
                "clock".into(),
                "shell".into(),
            ],
        }
    }
}

impl Settings {
    fn load() -> Self {
        let content = match fs::read_to_string(SETTINGS_PATH) {
            Ok(c) => c,
            Err(_) => {
                let s = Settings::default();
                s.save();
                return s;
            }
        };
        let mut s = Settings::default();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            if let Some((key, val)) = line.split_once('=') {
                let key = key.trim();
                let val = val.trim();
                match key {
                    "wallpaper_color" => {
                        s.wallpaper_color = parse_u32_or_hex(val).unwrap_or(s.wallpaper_color);
                    }
                    "font_size" => {
                        s.font_size = val.parse().unwrap_or(s.font_size);
                    }
                    "theme" => {
                        s.theme = val.trim_matches('"').to_string();
                    }
                    "boot_apps" => {
                        s.boot_apps = parse_string_array(val);
                    }
                    _ => {}
                }
            }
        }
        s
    }

    fn save(&self) {
        let apps_str: Vec<String> = self.boot_apps.iter().map(|a| format!("\"{a}\"")).collect();
        let content = format!(
            "# VyomaOS Settings\n\
             \n\
             wallpaper_color = 0x{:08X}\n\
             font_size = {}\n\
             theme = \"{}\"\n\
             boot_apps = [{}]\n",
            self.wallpaper_color,
            self.font_size,
            self.theme,
            apps_str.join(", "),
        );
        let _ = fs::write(SETTINGS_PATH, content);
    }

    fn apply_wallpaper(&self) {
        println!("@supervisor: wallpaper 0x{:08X}", self.wallpaper_color);
        let _ = io::stdout().flush();
    }
}

fn parse_u32_or_hex(s: &str) -> Option<u32> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

fn parse_string_array(s: &str) -> Vec<String> {
    let s = s.trim();
    let inner = s.trim_start_matches('[').trim_end_matches(']');
    inner
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

#[derive(Clone)]
struct MenuItem {
    section: &'static str,
    label: &'static str,
    kind: ItemKind,
}

#[derive(Clone)]
enum ItemKind {
    WallpaperColor,
    FontSize,
    Theme,
    BootApps,
    RestartSystem,
    ShutDown,
}

const WALLPAPER_PRESETS: &[(u32, &str)] = &[
    (0x1C1C1EFF, "Dark Gray"),
    (0x0D1117FF, "Black"),
    (0x1E3A5FFF, "Navy"),
    (0x2D4A22FF, "Forest"),
    (0x3B2040FF, "Plum"),
    (0x4A1C1CFF, "Maroon"),
    (0x1A1A2EFF, "Midnight"),
    (0x2E2E2EFF, "Charcoal"),
];

const FONT_SIZES: &[u32] = &[10, 11, 12, 13, 14, 16, 18, 20];

const THEMES: &[&str] = &["dark", "light", "auto"];

const AVAILABLE_BOOT_APPS: &[&str] = &[
    "desktop", "dock", "clock", "shell", "calculator", "file-manager",
    "notes", "http-server", "system-monitor",
];

fn build_menu() -> Vec<MenuItem> {
    vec![
        MenuItem { section: "Display",    label: "Wallpaper Color", kind: ItemKind::WallpaperColor },
        MenuItem { section: "Display",    label: "Font Size",       kind: ItemKind::FontSize },
        MenuItem { section: "Appearance", label: "Theme",           kind: ItemKind::Theme },
        MenuItem { section: "Startup",    label: "Boot Apps",       kind: ItemKind::BootApps },
        MenuItem { section: "Power",      label: "Restart System",  kind: ItemKind::RestartSystem },
        MenuItem { section: "Power",      label: "Shut Down",       kind: ItemKind::ShutDown },
    ]
}

fn draw_all(items: &[MenuItem], cursor: usize, settings: &Settings, status: &str) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    text(LEFT_PAD, 16, C_TEXT, "System Preferences");
    text(W - 300, 16, C_HINT, "Up/Dn: move  L/R: change  S: save");

    let mut y = HEADER_H + 20;
    let mut last_section = "";

    for (i, item) in items.iter().enumerate() {
        if item.section != last_section {
            if !last_section.is_empty() {
                y += SECTION_GAP;
            }
            text(LEFT_PAD, y, C_SECTION, item.section);
            fill(LEFT_PAD, y + 20, W - LEFT_PAD * 2, 1, C_BORDER);
            y += 28;
            last_section = item.section;
        }

        let is_sel = i == cursor;
        if is_sel {
            fill(LEFT_PAD - 4, y - 2, W - LEFT_PAD * 2 + 8, ROW_H, C_SEL_BG);
        }

        let label_color = if is_sel { C_TEXT } else { C_DIM };
        text(LEFT_PAD + 8, y + 8, label_color, item.label);

        let value_str = get_value_string(item, settings);
        let val_color = if is_sel { C_TEXT } else { C_VALUE };
        let val_x = W - LEFT_PAD - (value_str.len() as u32 * 8).min(400);
        text(val_x, y + 8, val_color, &value_str);

        y += ROW_H;
    }

    if let Some(item) = items.get(cursor) {
        if matches!(item.kind, ItemKind::BootApps) {
            y += SECTION_GAP;
            text(LEFT_PAD + 16, y, C_HINT, "Left/Right to add/remove apps:");
            y += 24;
            for app in AVAILABLE_BOOT_APPS {
                let enabled = settings.boot_apps.iter().any(|a| a == app);
                let marker = if enabled { "[x]" } else { "[ ]" };
                let color = if enabled { C_VALUE } else { C_DIM };
                text(LEFT_PAD + 24, y, color, &format!("{marker} {app}"));
                y += 20;
            }
        }
    }

    fill(0, H - STATUS_H, W, STATUS_H, C_STATUS_BG);
    fill(0, H - STATUS_H, W, 1, C_BORDER);
    if !status.is_empty() {
        text(LEFT_PAD, H - STATUS_H + 8, C_VALUE, status);
    }

    flush();
}

fn get_value_string(item: &MenuItem, settings: &Settings) -> String {
    match item.kind {
        ItemKind::WallpaperColor => {
            let name = WALLPAPER_PRESETS
                .iter()
                .find(|(c, _)| *c == settings.wallpaper_color)
                .map(|(_, n)| *n)
                .unwrap_or("Custom");
            format!("{name} (0x{:08X})", settings.wallpaper_color)
        }
        ItemKind::FontSize => format!("{}pt", settings.font_size),
        ItemKind::Theme => settings.theme.clone(),
        ItemKind::BootApps => {
            let count = settings.boot_apps.len();
            format!("{count} apps")
        }
        ItemKind::RestartSystem => "Press Enter".to_string(),
        ItemKind::ShutDown => "Press Enter".to_string(),
    }
}

fn cycle_value(settings: &mut Settings, item: &MenuItem, forward: bool) -> bool {
    match item.kind {
        ItemKind::WallpaperColor => {
            let idx = WALLPAPER_PRESETS
                .iter()
                .position(|(c, _)| *c == settings.wallpaper_color)
                .unwrap_or(0);
            let next = if forward {
                (idx + 1) % WALLPAPER_PRESETS.len()
            } else if idx == 0 {
                WALLPAPER_PRESETS.len() - 1
            } else {
                idx - 1
            };
            settings.wallpaper_color = WALLPAPER_PRESETS[next].0;
            settings.apply_wallpaper();
            true
        }
        ItemKind::FontSize => {
            let idx = FONT_SIZES.iter().position(|&s| s == settings.font_size).unwrap_or(3);
            let next = if forward {
                (idx + 1).min(FONT_SIZES.len() - 1)
            } else {
                idx.saturating_sub(1)
            };
            settings.font_size = FONT_SIZES[next];
            true
        }
        ItemKind::Theme => {
            let idx = THEMES.iter().position(|&t| t == settings.theme).unwrap_or(0);
            let next = if forward {
                (idx + 1) % THEMES.len()
            } else if idx == 0 {
                THEMES.len() - 1
            } else {
                idx - 1
            };
            settings.theme = THEMES[next].to_string();
            true
        }
        ItemKind::BootApps => {
            // Toggle the next/prev available app in/out of boot list
            toggle_boot_app(settings, forward);
            true
        }
        ItemKind::RestartSystem | ItemKind::ShutDown => false,
    }
}

fn toggle_boot_app(settings: &mut Settings, forward: bool) {
    if forward {
        for app in AVAILABLE_BOOT_APPS {
            if !settings.boot_apps.iter().any(|a| a == app) {
                settings.boot_apps.push(app.to_string());
                return;
            }
        }
    } else {
        if settings.boot_apps.len() > 1 {
            settings.boot_apps.pop();
        }
    }
}

fn draw_confirm(action_label: &str) {
    fill(0, 0, W, H, C_OVERLAY);
    let dw: u32 = 400;
    let dh: u32 = 120;
    let dx = (W - dw) / 2;
    let dy = (H - dh) / 2;
    fill(dx, dy, dw, dh, C_DIALOG_BG);
    fill(dx, dy, dw, 1, C_BORDER);
    fill(dx, dy + dh - 1, dw, 1, C_BORDER);
    fill(dx, dy, 1, dh, C_BORDER);
    fill(dx + dw - 1, dy, 1, dh, C_BORDER);
    text(dx + 24, dy + 20, C_WARN, action_label);
    text(dx + 24, dy + 50, C_TEXT, "Are you sure? (y/n)");
    text(dx + 24, dy + 80, C_DIM, "Press Y to confirm, N or Esc to cancel");
    flush();
}

#[derive(Clone)]
enum PendingPower {
    Restart,
    ShutDown,
}

fn main() {
    let stdin = io::stdin();
    let items = build_menu();
    let mut cursor: usize = 0;
    let mut settings = Settings::load();
    let mut status = String::from("Settings loaded");
    let mut confirm: Option<PendingPower> = None;

    // Raise window
    println!("@supervisor: raise system-preferences");
    let _ = io::stdout().flush();

    draw_all(&items, cursor, &settings, &status);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if raw.starts_with("REPLY:") || raw.starts_with("VYOMA_SYSTEM:") {
            continue;
        }

        // ── Confirmation dialog mode ────────────────────────────────
        if let Some(ref pending) = confirm {
            match raw.as_str() {
                "y" | "Y" => {
                    let cmd = match pending {
                        PendingPower::Restart => "reboot",
                        PendingPower::ShutDown => "shutdown",
                    };
                    println!("@supervisor: {cmd}");
                    let _ = io::stdout().flush();
                    status = format!("Sent {cmd} command...");
                    confirm = None;
                }
                "n" | "N" | "\x1b" => {
                    status = String::from("Cancelled");
                    confirm = None;
                }
                _ => {
                    // Ignore other keys while dialog is open
                    continue;
                }
            }
            draw_all(&items, cursor, &settings, &status);
            continue;
        }

        // ── Normal mode ─────────────────────────────────────────────
        let mut changed = false;

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x00000000);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if cursor > 0 { cursor -= 1; }
                status.clear();
            }
            "\x1b[B" => {
                if cursor + 1 < items.len() { cursor += 1; }
                status.clear();
            }
            "\x1b[C" => {
                if let Some(item) = items.get(cursor) {
                    changed = cycle_value(&mut settings, item, true);
                }
                status = String::from("Modified (press S to save)");
            }
            "\x1b[D" => {
                if let Some(item) = items.get(cursor) {
                    changed = cycle_value(&mut settings, item, false);
                }
                status = String::from("Modified (press S to save)");
            }
            // Enter — power items show confirmation, others cycle forward
            "" => {
                if let Some(item) = items.get(cursor) {
                    match item.kind {
                        ItemKind::RestartSystem => {
                            confirm = Some(PendingPower::Restart);
                            draw_all(&items, cursor, &settings, &status);
                            draw_confirm("Restart System");
                            continue;
                        }
                        ItemKind::ShutDown => {
                            confirm = Some(PendingPower::ShutDown);
                            draw_all(&items, cursor, &settings, &status);
                            draw_confirm("Shut Down");
                            continue;
                        }
                        _ => {
                            changed = cycle_value(&mut settings, item, true);
                            status = String::from("Modified (press S to save)");
                        }
                    }
                }
            }
            "s" | "S" => {
                settings.save();
                settings.apply_wallpaper();
                status = String::from("Settings saved to /data/settings.toml");
            }
            "r" | "R" => {
                settings = Settings::load();
                status = String::from("Settings reloaded from disk");
            }
            "d" | "D" => {
                settings = Settings::default();
                changed = true;
                status = String::from("Defaults restored (press S to save)");
            }
            _ => {}
        }

        if changed { settings.save(); }

        draw_all(&items, cursor, &settings, &status);
    }
}
