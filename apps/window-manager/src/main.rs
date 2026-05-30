// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! VyomaOS window manager — event-driven tiling layout engine.
//!
//! Listens for VYOMA_SYSTEM events on stdin and maintains a tiling layout
//! for all display apps. Reserves top 24px for menu bar and bottom 64px
//! for dock, splitting the remaining area in a 2-column grid.

use std::io::{BufRead, Write};

/// Reserved pixels for the menu bar at the top.
const MENU_BAR_H: u32 = 24;
/// Reserved pixels for the dock at the bottom.
const DOCK_H: u32 = 64;
/// Maximum columns in the tiling grid.
const COLS: u32 = 2;

/// Apps excluded from tiling layout management.
const SKIP_APPS: &[&str] = &["window-manager", "shell", "dock"];

/// Tracks current screen size and managed display apps.
struct LayoutState {
    screen_w: u32,
    screen_h: u32,
    /// Names of display apps currently running (sorted).
    apps: Vec<String>,
}

impl LayoutState {
    fn new() -> Self {
        Self {
            screen_w: 0,
            screen_h: 0,
            apps: Vec::new(),
        }
    }

    /// Returns true if we have a valid screen size.
    fn has_screen(&self) -> bool {
        self.screen_w > 0 && self.screen_h > 0
    }

    /// Add an app if not already tracked and not in the skip list.
    /// Returns true if the app was actually added.
    fn add_app(&mut self, name: &str) -> bool {
        if SKIP_APPS.contains(&name) {
            return false;
        }
        if self.apps.iter().any(|a| a == name) {
            return false;
        }
        self.apps.push(name.to_string());
        self.apps.sort();
        true
    }

    /// Remove an app. Returns true if it was present.
    fn remove_app(&mut self, name: &str) -> bool {
        let before = self.apps.len();
        self.apps.retain(|a| a != name);
        self.apps.len() < before
    }

    /// Compute tiling rectangles for all managed apps.
    /// Layout: 2-column grid in the usable area (below menu bar, above dock).
    fn compute_tiles(&self) -> Vec<(&str, u32, u32, u32, u32)> {
        if !self.has_screen() || self.apps.is_empty() {
            return Vec::new();
        }

        let usable_y = MENU_BAR_H;
        let usable_h = self.screen_h.saturating_sub(MENU_BAR_H + DOCK_H);
        let usable_w = self.screen_w;

        if usable_h == 0 || usable_w == 0 {
            return Vec::new();
        }

        let n = self.apps.len() as u32;
        let cols = if n == 1 { 1 } else { COLS.min(n) };
        let rows = (n + cols - 1) / cols;

        let tile_w = usable_w / cols;
        let tile_h = usable_h / rows;

        self.apps
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let i = i as u32;
                let col = i % cols;
                let row = i / cols;

                // Last column in a row gets remaining width
                let w = if col == cols - 1 {
                    usable_w - col * tile_w
                } else {
                    tile_w
                };
                // Last row gets remaining height
                let h = if row == rows - 1 {
                    usable_y + usable_h - (usable_y + row * tile_h)
                } else {
                    tile_h
                };

                let x = col * tile_w;
                let y = usable_y + row * tile_h;

                (name.as_str(), x, y, w, h)
            })
            .collect()
    }

    /// Emit resize commands for all managed apps.
    fn emit_layout(&self) {
        let tiles = self.compute_tiles();
        for (name, x, y, w, h) in tiles {
            println!("@supervisor: resize {name} {x},{y},{w},{h}");
        }
        let _ = std::io::stdout().flush();
    }
}

/// Parse a VYOMA_SYSTEM message from a line of stdin.
enum SystemEvent<'a> {
    Screen(u32, u32),
    DisplayProfile(&'a str),
    AppLaunched(&'a str),
    AppExited(&'a str),
    Unknown,
}

fn parse_system_event(line: &str) -> SystemEvent<'_> {
    let payload = match line.strip_prefix("VYOMA_SYSTEM:") {
        Some(p) => p,
        None => return SystemEvent::Unknown,
    };

    if let Some(rest) = payload.strip_prefix("screen:") {
        if let Some((ws, hs)) = rest.split_once(',') {
            if let (Ok(w), Ok(h)) = (ws.parse::<u32>(), hs.parse::<u32>()) {
                return SystemEvent::Screen(w, h);
            }
        }
    } else if let Some(rest) = payload.strip_prefix("display_profile:") {
        return SystemEvent::DisplayProfile(rest);
    } else if let Some(rest) = payload.strip_prefix("app_launched:") {
        return SystemEvent::AppLaunched(rest);
    } else if let Some(rest) = payload.strip_prefix("app_exited:") {
        return SystemEvent::AppExited(rest);
    }

    SystemEvent::Unknown
}

fn main() {
    eprintln!("[window-manager] starting event-driven layout engine");

    let mut state = LayoutState::new();
    let stdin = std::io::stdin();

    // Request initial app list
    println!("@supervisor: list");
    let _ = std::io::stdout().flush();

    let mut got_initial_list = false;

    for raw in stdin.lock().lines() {
        let line = match raw {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle initial REPLY from @supervisor: list
        if !got_initial_list {
            if let Some(reply) = line.strip_prefix("REPLY:") {
                got_initial_list = true;
                // Parse pipe-separated app list
                for app_name in reply.split('|').map(|s| s.trim()) {
                    if !app_name.is_empty() {
                        state.add_app(app_name);
                    }
                }
                if state.has_screen() {
                    state.emit_layout();
                }
                continue;
            }
        }

        // Parse VYOMA_SYSTEM events
        match parse_system_event(&line) {
            SystemEvent::Screen(w, h) => {
                eprintln!("[window-manager] screen size: {w}x{h}");
                state.screen_w = w;
                state.screen_h = h;
                if !state.apps.is_empty() {
                    state.emit_layout();
                }
            }
            SystemEvent::DisplayProfile(kind) => {
                eprintln!("[window-manager] display profile: {kind}");
                // Could adjust layout strategy per profile in the future
            }
            SystemEvent::AppLaunched(name) => {
                eprintln!("[window-manager] app launched: {name}");
                if state.add_app(name) && state.has_screen() {
                    state.emit_layout();
                }
            }
            SystemEvent::AppExited(name) => {
                eprintln!("[window-manager] app exited: {name}");
                if state.remove_app(name) && state.has_screen() {
                    state.emit_layout();
                }
            }
            SystemEvent::Unknown => {
                // Handle "retile" command for manual re-layout
                if line.trim() == "retile" && state.has_screen() {
                    state.emit_layout();
                }
                // Ignore other messages (REPLY from resize commands, etc.)
            }
        }
    }
}
