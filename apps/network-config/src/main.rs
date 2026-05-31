// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P48 — Network Configuration UI panel.
//!
//! Displays and edits DNS server, HTTP proxy, default port, and test URL.
//! Persists to `/data/network.toml`. Uses `@supervisor: http-get` for
//! connectivity testing.

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
const C_EDIT_BG: u32 = 0x3A3A3CFF;

const CONFIG_PATH: &str = "/data/network.toml";

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

// ── Persistent configuration ─────────────────────────────────────────

struct NetConfig {
    dns_server: String,
    proxy_enabled: bool,
    proxy_address: String,
    port: u16,
    test_url: String,
}

impl Default for NetConfig {
    fn default() -> Self {
        NetConfig {
            dns_server: "8.8.8.8".to_string(),
            proxy_enabled: false,
            proxy_address: String::new(),
            port: 8080,
            test_url: "http://example.com".to_string(),
        }
    }
}

impl NetConfig {
    fn load() -> Self {
        let content = match fs::read_to_string(CONFIG_PATH) {
            Ok(c) => c,
            Err(_) => {
                let cfg = NetConfig::default();
                cfg.save();
                return cfg;
            }
        };
        let mut cfg = NetConfig::default();
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with('[') {
                continue;
            }
            if let Some((key, val)) = line.split_once('=') {
                let key = key.trim();
                let val = val.trim().trim_matches('"');
                match key {
                    "dns_server" => cfg.dns_server = val.to_string(),
                    "proxy_enabled" => cfg.proxy_enabled = val == "true",
                    "proxy_address" => cfg.proxy_address = val.to_string(),
                    "port" => cfg.port = val.parse().unwrap_or(8080),
                    "test_url" => cfg.test_url = val.to_string(),
                    _ => {}
                }
            }
        }
        cfg
    }

    fn save(&self) {
        let content = format!(
            "# VyomaOS Network Configuration\n\
             \n\
             dns_server = \"{}\"\n\
             proxy_enabled = {}\n\
             proxy_address = \"{}\"\n\
             port = {}\n\
             test_url = \"{}\"\n",
            self.dns_server, self.proxy_enabled, self.proxy_address,
            self.port, self.test_url,
        );
        let _ = fs::write(CONFIG_PATH, content);
    }
}

// ── Menu definition ──────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Setting {
    DnsServer,
    ProxyEnabled,
    ProxyAddress,
    Port,
    TestUrl,
    RunTest,
}

struct MenuItem {
    section: &'static str,
    label: &'static str,
    kind: Setting,
}

fn build_menu() -> Vec<MenuItem> {
    vec![
        MenuItem { section: "DNS",     label: "DNS Server",        kind: Setting::DnsServer },
        MenuItem { section: "Proxy",   label: "Proxy Enabled",     kind: Setting::ProxyEnabled },
        MenuItem { section: "Proxy",   label: "Proxy Address",     kind: Setting::ProxyAddress },
        MenuItem { section: "Network", label: "Default Port",      kind: Setting::Port },
        MenuItem { section: "Test",    label: "Test URL",          kind: Setting::TestUrl },
        MenuItem { section: "Test",    label: "Test Connectivity",  kind: Setting::RunTest },
    ]
}

fn value_string(kind: Setting, cfg: &NetConfig, test_result: &str) -> String {
    match kind {
        Setting::DnsServer => cfg.dns_server.clone(),
        Setting::ProxyEnabled => {
            if cfg.proxy_enabled { "Enabled".into() } else { "Disabled".into() }
        }
        Setting::ProxyAddress => {
            if cfg.proxy_address.is_empty() {
                "(none)".into()
            } else {
                cfg.proxy_address.clone()
            }
        }
        Setting::Port => cfg.port.to_string(),
        Setting::TestUrl => cfg.test_url.clone(),
        Setting::RunTest => {
            if test_result.is_empty() { "Press Enter".into() } else { test_result.into() }
        }
    }
}

fn editable_value(kind: Setting, cfg: &NetConfig) -> String {
    match kind {
        Setting::DnsServer => cfg.dns_server.clone(),
        Setting::ProxyAddress => cfg.proxy_address.clone(),
        Setting::Port => cfg.port.to_string(),
        Setting::TestUrl => cfg.test_url.clone(),
        Setting::ProxyEnabled | Setting::RunTest => String::new(),
    }
}

// ── Rendering ────────────────────────────────────────────────────────

fn draw_all(
    items: &[MenuItem],
    cursor: usize,
    cfg: &NetConfig,
    status: &str,
    test_result: &str,
    editing: Option<usize>,
    edit_buf: &str,
) {
    fill(0, 0, W, H, C_BG);

    // Header bar
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    text(LEFT_PAD, 16, C_TEXT, "Network Configuration");
    text(W - 360, 16, C_HINT, "Up/Dn: move  Enter: edit  S: save  T: test");

    let mut y = HEADER_H + 20;
    let mut last_section = "";

    for (i, item) in items.iter().enumerate() {
        // Section headers
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

        if editing == Some(i) {
            // Inline edit field
            let edit_x = W / 2;
            let edit_w = W - edit_x - LEFT_PAD;
            fill(edit_x, y, edit_w, ROW_H - 4, C_EDIT_BG);
            let cursor_str = format!("{edit_buf}_");
            text(edit_x + 4, y + 8, C_TEXT, &cursor_str);
        } else {
            let vs = value_string(item.kind, cfg, test_result);
            let val_color = if is_sel { C_TEXT } else { C_VALUE };
            let vlen = (vs.len() as u32 * 8).min(400);
            let val_x = W - LEFT_PAD - vlen;
            text(val_x, y + 8, val_color, &vs);
        }

        y += ROW_H;
    }

    // Current state summary
    y += SECTION_GAP * 2;
    text(LEFT_PAD, y, C_SECTION, "Current State");
    fill(LEFT_PAD, y + 20, W - LEFT_PAD * 2, 1, C_BORDER);
    y += 28;
    text(LEFT_PAD + 8, y, C_DIM, &format!("DNS: {}", cfg.dns_server));
    y += 20;
    let proxy_str = if cfg.proxy_enabled {
        format!("Proxy: {} (enabled)", cfg.proxy_address)
    } else {
        "Proxy: none".to_string()
    };
    text(LEFT_PAD + 8, y, C_DIM, &proxy_str);
    y += 20;
    text(LEFT_PAD + 8, y, C_DIM, &format!("Port: {}", cfg.port));

    // Status bar
    fill(0, H - STATUS_H, W, STATUS_H, C_STATUS_BG);
    fill(0, H - STATUS_H, W, 1, C_BORDER);
    if !status.is_empty() {
        let c = if status.contains("Error") || status.contains("FAIL") {
            C_WARN
        } else {
            C_VALUE
        };
        text(LEFT_PAD, H - STATUS_H + 8, c, status);
    }

    flush();
}

// ── Entry point ──────────────────────────────────────────────────────

fn main() {
    let stdin = io::stdin();
    let items = build_menu();
    let mut cursor: usize = 0;
    let mut cfg = NetConfig::load();
    let mut status = String::from("Config loaded");
    let mut test_result = String::new();
    let mut editing: Option<usize> = None;
    let mut edit_buf = String::new();
    let mut pending_test = false;

    // Raise our window
    println!("@supervisor: raise network-config");
    let _ = io::stdout().flush();

    draw_all(&items, cursor, &cfg, &status, &test_result, editing, &edit_buf);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle IPC replies from supervisor
        if raw.starts_with("REPLY:http-get ") {
            if pending_test {
                let rest = &raw["REPLY:http-get ".len()..];
                if rest.starts_with("error") {
                    test_result = format!("FAIL: {rest}");
                    status = "Connectivity test failed".to_string();
                } else {
                    let code = rest.split_whitespace().next().unwrap_or("???");
                    test_result = format!("OK (HTTP {code})");
                    status = "Connectivity test passed".to_string();
                }
                pending_test = false;
                draw_all(&items, cursor, &cfg, &status, &test_result, editing, &edit_buf);
            }
            continue;
        }
        if raw.starts_with("REPLY:") || raw.starts_with("VYOMA_SYSTEM:") {
            continue;
        }

        // ── Edit mode input handling ─────────────────────────────────
        if editing.is_some() {
            match raw.as_str() {
                "\x1b" => {
                    editing = None;
                    edit_buf.clear();
                    status = "Edit cancelled".to_string();
                }
                "" => {
                    // Enter: commit the edit
                    if let Some(idx) = editing {
                        apply_edit(items[idx].kind, &edit_buf, &mut cfg);
                    }
                    editing = None;
                    edit_buf.clear();
                    status = "Modified (press S to save)".to_string();
                }
                "\x7f" | "\x08" => {
                    edit_buf.pop();
                }
                other => {
                    if !other.starts_with('\x1b') && other.len() <= 4 {
                        edit_buf.push_str(other);
                    }
                }
            }
            draw_all(&items, cursor, &cfg, &status, &test_result, editing, &edit_buf);
            continue;
        }

        // ── Normal mode input handling ───────────────────────────────
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
            "\x1b[C" | "\x1b[D" => {
                // Left/Right: toggle boolean fields
                if let Some(item) = items.get(cursor) {
                    if item.kind == Setting::ProxyEnabled {
                        cfg.proxy_enabled = !cfg.proxy_enabled;
                        status = "Modified (press S to save)".to_string();
                    }
                }
            }
            "" => {
                // Enter: context-dependent action
                if let Some(item) = items.get(cursor) {
                    match item.kind {
                        Setting::RunTest => {
                            test_result = "Testing...".to_string();
                            status = format!("Testing {}", cfg.test_url);
                            pending_test = true;
                            draw_all(
                                &items, cursor, &cfg, &status, &test_result,
                                editing, &edit_buf,
                            );
                            println!("@supervisor: http-get {}", cfg.test_url);
                            let _ = io::stdout().flush();
                            continue;
                        }
                        Setting::ProxyEnabled => {
                            cfg.proxy_enabled = !cfg.proxy_enabled;
                            status = "Modified (press S to save)".to_string();
                        }
                        _ => {
                            editing = Some(cursor);
                            edit_buf = editable_value(item.kind, &cfg);
                            status = "Editing -- Enter: apply, Esc: cancel".to_string();
                        }
                    }
                }
            }
            "s" | "S" => {
                cfg.save();
                status = format!("Saved to {CONFIG_PATH}");
            }
            "r" | "R" => {
                cfg = NetConfig::load();
                status = "Config reloaded from disk".to_string();
            }
            "d" | "D" => {
                cfg = NetConfig::default();
                status = "Defaults restored (press S to save)".to_string();
            }
            "t" | "T" => {
                test_result = "Testing...".to_string();
                status = format!("Testing {}", cfg.test_url);
                pending_test = true;
                draw_all(
                    &items, cursor, &cfg, &status, &test_result,
                    editing, &edit_buf,
                );
                println!("@supervisor: http-get {}", cfg.test_url);
                let _ = io::stdout().flush();
                continue;
            }
            _ => {}
        }

        draw_all(&items, cursor, &cfg, &status, &test_result, editing, &edit_buf);
    }
}

fn apply_edit(kind: Setting, buf: &str, cfg: &mut NetConfig) {
    let val = buf.trim();
    match kind {
        Setting::DnsServer => {
            if !val.is_empty() {
                cfg.dns_server = val.to_string();
            }
        }
        Setting::ProxyAddress => {
            cfg.proxy_address = val.to_string();
        }
        Setting::Port => {
            if let Ok(p) = val.parse::<u16>() {
                if p > 0 {
                    cfg.port = p;
                }
            }
        }
        Setting::TestUrl => {
            if !val.is_empty() {
                cfg.test_url = val.to_string();
            }
        }
        Setting::ProxyEnabled | Setting::RunTest => {}
    }
}
