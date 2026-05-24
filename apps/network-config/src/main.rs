// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1040;
const H: u32 = 600;
const C_BG: u32      = 0x0D1117FF;
const C_TITLE: u32   = 0xFFFFFFFF;
const C_ACCENT: u32  = 0x58A6FFFF;
const C_DIM: u32     = 0x8B949EFF;
const C_FIELD: u32   = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_OK: u32      = 0x3FB950FF;
const C_HINT: u32    = 0x6E7681FF;
const C_SEL: u32     = 0x1F4068FF;
const C_SECTION: u32 = 0x161B22FF;

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

const FIELD_COUNT: usize = 5;
const FIELD_LABELS: [&str; FIELD_COUNT] = ["Interface:", "IP Address:", "Netmask:", "Gateway:", "DNS Server:"];
const FIELD_PLACEHOLDERS: [&str; FIELD_COUNT] = ["eth0", "e.g. 192.168.1.100", "e.g. 255.255.255.0", "e.g. 192.168.1.1", "8.8.8.8"];

struct Config {
    mode:      String,
    interface: String,
    ip:        String,
    netmask:   String,
    gateway:   String,
    dns:       String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode:      "dhcp".into(),
            interface: "eth0".into(),
            ip:        String::new(),
            netmask:   String::new(),
            gateway:   String::new(),
            dns:       "8.8.8.8".into(),
        }
    }
}

impl Config {
    fn get(&self, idx: usize) -> &str {
        match idx {
            0 => &self.interface,
            1 => &self.ip,
            2 => &self.netmask,
            3 => &self.gateway,
            _ => &self.dns,
        }
    }
    fn get_mut(&mut self, idx: usize) -> &mut String {
        match idx {
            0 => &mut self.interface,
            1 => &mut self.ip,
            2 => &mut self.netmask,
            3 => &mut self.gateway,
            _ => &mut self.dns,
        }
    }
    fn to_toml(&self) -> String {
        format!(
            "[network]\nmode = \"{}\"\ninterface = \"{}\"\nip = \"{}\"\nnetmask = \"{}\"\ngateway = \"{}\"\ndns = \"{}\"\n",
            self.mode, self.interface, self.ip, self.netmask, self.gateway, self.dns
        )
    }
}

fn load_config() -> Config {
    let Ok(content) = std::fs::read_to_string("/data/network.toml") else {
        return Config::default();
    };
    let mut cfg = Config::default();
    for line in content.lines() {
        let line = line.trim();
        if let Some(v) = line.strip_prefix("mode = ") {
            cfg.mode = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("interface = ") {
            cfg.interface = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("ip = ") {
            cfg.ip = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("netmask = ") {
            cfg.netmask = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("gateway = ") {
            cfg.gateway = v.trim_matches('"').to_string();
        } else if let Some(v) = line.strip_prefix("dns = ") {
            cfg.dns = v.trim_matches('"').to_string();
        }
    }
    cfg
}

struct State {
    cfg:       Config,
    focused:   usize,
    saved_msg: String,
}

impl State {
    fn new() -> Self {
        Self { cfg: load_config(), focused: 0, saved_msg: String::new() }
    }
}

fn draw(s: &State) {
    fill(0, 0, W, H, C_BG);

    // Header
    text(20, 14, C_ACCENT, "Network Configuration");
    fill(0, 36, W, 1, C_BORDER);

    // Mode toggle section
    fill(20, 46, W - 40, 56, C_SECTION);
    border(20, 46, W - 40, 56, C_BORDER);
    text(34, 56, C_DIM, "Mode:");
    let (dhcp_bg, dhcp_tc) = if s.cfg.mode == "dhcp" { (C_SEL, C_ACCENT) } else { (C_FIELD, C_DIM) };
    let (stat_bg, stat_tc) = if s.cfg.mode == "static" { (C_SEL, C_ACCENT) } else { (C_FIELD, C_DIM) };
    fill(120, 52, 120, 32, dhcp_bg);
    border(120, 52, 120, 32, C_BORDER);
    text(148, 60, dhcp_tc, "DHCP");
    fill(256, 52, 120, 32, stat_bg);
    border(256, 52, 120, 32, C_BORDER);
    text(280, 60, stat_tc, "Static");
    text(400, 60, C_HINT, "d = DHCP   s = Static");

    // Fields header
    fill(20, 114, W - 40, 1, C_BORDER);
    text(20, 122, C_DIM, "Static IP Settings");

    let label_color = if s.cfg.mode == "dhcp" { C_HINT } else { C_DIM };
    for (i, (label, ph)) in FIELD_LABELS.iter().zip(FIELD_PLACEHOLDERS.iter()).enumerate() {
        let fy = 142 + i as u32 * 66;
        text(34, fy, label_color, label);
        let is_focused = s.focused == i && s.cfg.mode == "static";
        let bg = if is_focused { C_SEL } else { C_FIELD };
        let bc = if is_focused { C_ACCENT } else { C_BORDER };
        fill(34, fy + 18, W - 68, 30, bg);
        border(34, fy + 18, W - 68, 30, bc);
        let val = s.cfg.get(i);
        let (display, tc) = if val.is_empty() {
            (*ph, C_HINT)
        } else {
            (val, if s.cfg.mode == "dhcp" { C_HINT } else { C_TITLE })
        };
        text(44, fy + 24, tc, display);
    }

    if !s.saved_msg.is_empty() {
        text(20, H - 38, C_OK, &s.saved_msg);
    }
    text(20, H - 18, C_HINT, "Tab/Up/Down: navigate fields   d/s: toggle mode   Ctrl+W: save   Ctrl+C: quit");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut s = State::new();
    draw(&s);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x17" => {
                match std::fs::write("/data/network.toml", s.cfg.to_toml()) {
                    Ok(()) => s.saved_msg = "Saved to /data/network.toml".into(),
                    Err(e) => s.saved_msg = format!("Error saving: {e}"),
                }
                draw(&s);
            }
            "d" => {
                s.cfg.mode = "dhcp".into();
                s.saved_msg.clear();
                draw(&s);
            }
            "s" => {
                s.cfg.mode = "static".into();
                s.saved_msg.clear();
                draw(&s);
            }
            "\t" | "\x1b[B" => {
                if s.cfg.mode == "static" {
                    s.focused = (s.focused + 1) % FIELD_COUNT;
                    s.saved_msg.clear();
                    draw(&s);
                }
            }
            "\x1b[A" => {
                if s.cfg.mode == "static" {
                    s.focused = if s.focused == 0 { FIELD_COUNT - 1 } else { s.focused - 1 };
                    s.saved_msg.clear();
                    draw(&s);
                }
            }
            "\x7f" => {
                if s.cfg.mode == "static" {
                    s.cfg.get_mut(s.focused).pop();
                    s.saved_msg.clear();
                    draw(&s);
                }
            }
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if (c.is_ascii_graphic() || c == ' ') && s.cfg.mode == "static" {
                    s.cfg.get_mut(s.focused).push(c);
                    s.saved_msg.clear();
                    draw(&s);
                }
            }
            _ => {}
        }
    }
}
