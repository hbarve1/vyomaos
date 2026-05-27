// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS Dock — full-width task bar at the bottom of the screen.
//!
//! Shows 12 pinned app icons with running-indicator dots, hover highlights,
//! and keyboard shortcuts (1-9, 0 for the 10th, q/w for 11-12).
//! Mouse clicks launch or raise the target app via `@supervisor: run`.
//! Polls `@supervisor: ps-raw` every ~2 s via ping/pong heartbeat to keep
//! the running-indicator dots current.

use std::io::{self, BufRead, Write};

const DEFAULT_SW: u32 = 1440;

// Colors
const C_BG:      u32 = 0x161B22EE;
const C_BORDER:  u32 = 0x30363DFF;
const C_LABEL:   u32 = 0xE6EDF3FF;
const C_DIM:     u32 = 0x8B949EFF;
const C_HINT:    u32 = 0x484F58FF;
const C_DOT:     u32 = 0x3FB950FF;
const C_HOVER:   u32 = 0x1F6FEBFF;
const C_ACTIVE:  u32 = 0x1C2E4AFF;

// Dock dimensions
const DOCK_H:    u32 = 60;
const ICON_W:    u32 = 64;
const ICON_H:    u32 = 44;
const ICON_Y:    u32 = 6;
const CORNER:    u32 = 6;

struct DockItem {
    label: &'static str,
    name:  &'static str,
    color: u32,
    key:   &'static str,
    icon:  Option<&'static str>,
}

const ITEMS: &[DockItem] = &[
    DockItem { label: "Finder",  name: "file-manager",   color: 0x1F6FEBFF, key: "1", icon: None },
    DockItem { label: "Shell",   name: "shell",          color: 0x3FB950FF, key: "2", icon: Some("/apps/shell/icon.png") },
    DockItem { label: "Browser", name: "browser",        color: 0x58A6FFFF, key: "3", icon: None },
    DockItem { label: "Editor",  name: "text-editor",    color: 0x79C0FFFF, key: "4", icon: None },
    DockItem { label: "Notes",   name: "notes",          color: 0xA5D6A7FF, key: "5", icon: Some("/apps/notes/icon.png") },
    DockItem { label: "Music",   name: "music-player",   color: 0xF48FB1FF, key: "6", icon: None },
    DockItem { label: "Monitor", name: "system-monitor", color: 0xFFA657FF, key: "7", icon: None },
    DockItem { label: "Calendar",name: "calendar",       color: 0xBC8CFFFF, key: "8", icon: None },
    DockItem { label: "Settings",name: "settings",       color: 0x8B949EFF, key: "9", icon: Some("/apps/settings/icon.png") },
    DockItem { label: "Photos",  name: "photo-editor",   color: 0xFF7B72FF, key: "0", icon: None },
    DockItem { label: "Store",   name: "app-store",      color: 0x56D364FF, key: "q", icon: None },
    DockItem { label: "Chess",   name: "chess",          color: 0xE3B341FF, key: "w", icon: None },
];

#[inline]
fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}
#[inline]
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},m,{s}");
}
#[inline]
fn text_s(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},s,{s}");
}
#[inline]
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn icon_spacing(sw: u32) -> u32 {
    let total_icons = ICON_W * ITEMS.len() as u32;
    if sw > total_icons { (sw - total_icons) / (ITEMS.len() as u32 + 1) } else { 4 }
}

fn icon_x(idx: usize, sw: u32) -> u32 {
    let gap = icon_spacing(sw);
    gap + idx as u32 * (ICON_W + gap)
}

fn parse_running(reply: &str) -> Vec<String> {
    reply.split('|')
        .filter_map(|e| e.split(':').next().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn draw_icon(ix: u32, item: &DockItem, running: bool, hover: bool) {
    let bg = if hover { C_ACTIVE } else { 0x21262DFF };
    fill(ix, ICON_Y, ICON_W, ICON_H, bg);

    // Colored accent stripe at top of icon
    fill(ix, ICON_Y, ICON_W, 3, item.color);

    // Fake rounded bottom corners
    fill(ix,                   ICON_Y + ICON_H - CORNER, CORNER, CORNER, C_BG);
    fill(ix + ICON_W - CORNER, ICON_Y + ICON_H - CORNER, CORNER, CORNER, C_BG);

    // Hover/active ring
    if hover {
        fill(ix - 1, ICON_Y - 1,     ICON_W + 2, 1, C_HOVER);
        fill(ix - 1, ICON_Y + ICON_H, ICON_W + 2, 1, C_HOVER);
        fill(ix - 1, ICON_Y,          1, ICON_H,    C_HOVER);
        fill(ix + ICON_W, ICON_Y,     1, ICON_H,    C_HOVER);
    }

    // PNG icon or label fallback
    if let Some(path) = item.icon {
        println!("VYOMA_DRAW:draw_image:{ix},{ICON_Y},32,32,{path}");
    } else {
        let label = if item.label.len() > 7 { &item.label[..7] } else { item.label };
        let lw = label.len() as u32 * 8;
        let lx = ix + (ICON_W - lw.min(ICON_W)) / 2;
        let lc = if hover { C_LABEL } else { C_DIM };
        text(lx, ICON_Y + (ICON_H - 14) / 2, lc, label);
    }

    // Running indicator dot
    if running {
        let dx = ix + ICON_W / 2 - 2;
        fill(dx, ICON_Y + ICON_H + 2, 4, 4, C_DOT);
    }

    // Key hint below dot
    let kx = ix + (ICON_W - 4) / 2;
    text_s(kx, ICON_Y + ICON_H + 8, C_HINT, item.key);
}

fn draw(sw: u32, running: &[String], hover_idx: Option<usize>) {
    fill(0, 0, sw, DOCK_H, C_BG);
    fill(0, 0, sw, 1,      C_BORDER);

    for (i, item) in ITEMS.iter().enumerate() {
        let ix = icon_x(i, sw);
        let is_running = running.iter().any(|r| r == item.name);
        let is_hover   = hover_idx == Some(i);
        draw_icon(ix, item, is_running, is_hover);
    }

    flush();
}

fn parse_screen_w(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("VYOMA_SYSTEM:screen:")?;
    let ws = rest.split(',').next()?;
    ws.parse().ok()
}

fn parse_mouse_x(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("VYOMA_INPUT:mouse:")?;
    let coords = rest.strip_prefix("move:")
        .or_else(|| rest.strip_prefix("click:"))?;
    coords.split(',').next()?.parse().ok()
}

fn hit_test_icon(mx: u32, sw: u32) -> Option<usize> {
    for i in 0..ITEMS.len() {
        let ix = icon_x(i, sw);
        if mx >= ix && mx < ix + ICON_W { return Some(i); }
    }
    None
}

fn launch(item: &DockItem) {
    println!("@supervisor: run /apps/{}/vyoma.toml", item.name);
    let _ = io::stdout().flush();
}

fn main() {
    let stdin = io::stdin();
    let mut sw = DEFAULT_SW;
    let mut running: Vec<String> = Vec::new();
    let mut hover_idx: Option<usize> = None;

    println!("@supervisor: raise dock");
    println!("@supervisor: ps-raw");
    let _ = io::stdout().flush();

    draw(sw, &running, hover_idx);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if let Some(w) = parse_screen_w(&raw) {
            sw = w;
            draw(sw, &running, hover_idx);
            continue;
        }

        if raw.starts_with("REPLY:ps-raw") || raw.starts_with("REPLY:") && raw.contains(':') {
            let payload = raw.trim_start_matches("REPLY:ps-raw:").trim_start_matches("REPLY:");
            if raw.starts_with("REPLY:ps-raw") {
                running = parse_running(payload);
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
                draw(sw, &running, hover_idx);
            }
            continue;
        }

        if raw.starts_with("REPLY:pong") {
            println!("@supervisor: ps-raw");
            let _ = io::stdout().flush();
            continue;
        }

        if raw.starts_with("VYOMA_INPUT:mouse:") {
            let mx = parse_mouse_x(&raw);
            let new_hover = mx.and_then(|x| hit_test_icon(x, sw));
            if raw.contains(":click:") && raw.contains(":left") {
                if let Some(idx) = new_hover { launch(&ITEMS[idx]); }
            }
            if new_hover != hover_idx {
                hover_idx = new_hover;
                draw(sw, &running, hover_idx);
            }
            continue;
        }

        // Keyboard shortcuts
        match raw.as_str() {
            "\x03" => {
                fill(0, 0, sw, DOCK_H, C_BG);
                flush();
                std::process::exit(0);
            }
            k => {
                if let Some(idx) = ITEMS.iter().position(|it| it.key == k) {
                    launch(&ITEMS[idx]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_count() {
        assert_eq!(ITEMS.len(), 12);
    }

    #[test]
    fn icon_x_first() {
        let gap = icon_spacing(DEFAULT_SW);
        assert_eq!(icon_x(0, DEFAULT_SW), gap);
    }

    #[test]
    fn hit_test_first() {
        let gap = icon_spacing(DEFAULT_SW);
        let x = gap + 10;
        assert_eq!(hit_test_icon(x, DEFAULT_SW), Some(0));
    }

    #[test]
    fn hit_test_gap_no_hit() {
        let gap = icon_spacing(DEFAULT_SW);
        // Click exactly in the first gap before item 0
        assert_eq!(hit_test_icon(gap / 2, DEFAULT_SW), None);
    }

    #[test]
    fn parse_running_basic() {
        let r = parse_running("desktop:run:10:0|dock:run:5:0|http-server:run:3:0");
        assert!(r.contains(&"desktop".to_string()));
        assert!(r.contains(&"dock".to_string()));
    }

    #[test]
    fn parse_screen_w_valid() {
        assert_eq!(parse_screen_w("VYOMA_SYSTEM:screen:1920,1080"), Some(1920));
    }

    #[test]
    fn parse_mouse_x_move() {
        assert_eq!(parse_mouse_x("VYOMA_INPUT:mouse:move:300,40"), Some(300));
    }

    #[test]
    fn parse_mouse_x_click() {
        assert_eq!(parse_mouse_x("VYOMA_INPUT:mouse:click:150,30:left"), Some(150));
    }
}
