// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1240;
const H: u32 = 700;
const C_BG: u32      = 0x0D1117FF;
const C_TITLE: u32   = 0xFFFFFFFF;
const C_ACCENT: u32  = 0x58A6FFFF;
const C_DIM: u32     = 0x8B949EFF;
const C_FIELD: u32   = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_OK: u32      = 0x3FB950FF;
const C_ERR: u32     = 0xFF7B72FF;
const C_HINT: u32    = 0x6E7681FF;
const C_TERM_BG: u32 = 0x010409FF;
const C_TERM_FG: u32 = 0xC9D1D9FF;
const C_SEL: u32     = 0x1F4068FF;

const TERM_X: u32 = 20;
const TERM_Y: u32 = 80;
const TERM_W: u32 = W - 40;
const TERM_H: u32 = H - 140;
const LINE_H: u32 = 17;
const MAX_LINES: usize = (TERM_H / LINE_H) as usize;

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

#[derive(PartialEq)]
enum Phase { Form, Connecting, Connected(u32), Error(String) }

struct State {
    host:     String,
    port:     String,
    user:     String,
    field:    usize,  // 0=host 1=port 2=user
    phase:    Phase,
    term:     Vec<String>,
    input:    String,
    scroll:   usize,
}

impl State {
    fn new() -> Self {
        Self {
            host: String::new(),
            port: "22".into(),
            user: String::new(),
            field: 0,
            phase: Phase::Form,
            term: Vec::new(),
            input: String::new(),
            scroll: 0,
        }
    }
    fn push_term(&mut self, line: String) {
        for l in line.split("\\n") {
            if !l.is_empty() {
                self.term.push(l.to_string());
            }
        }
        let max_scroll = self.term.len().saturating_sub(MAX_LINES);
        self.scroll = max_scroll;
    }
}

fn draw_form(s: &State) {
    fill(0, 0, W, H, C_BG);
    text(20, 16, C_ACCENT, "SSH Client  (raw TCP terminal)");
    fill(0, 44, W, 1, C_BORDER);

    let fields = [("Host:", &s.host), ("Port:", &s.port), ("User:", &s.user)];
    for (i, (label, val)) in fields.iter().enumerate() {
        let fy = 64 + i as u32 * 56;
        text(20, fy, C_DIM, label);
        let bg = if i == s.field { C_SEL } else { C_FIELD };
        let bc = if i == s.field { C_ACCENT } else { C_BORDER };
        fill(20, fy + 18, 500, 32, bg);
        border(20, fy + 18, 500, 32, bc);
        let display = if val.is_empty() { match i { 0 => "hostname or IP", 1 => "22", _ => "username" } } else { val };
        let tc = if val.is_empty() { C_HINT } else { C_TITLE };
        text(30, fy + 26, tc, display);
    }

    text(20, H - 40, C_HINT, "Tab/Down: next field  Up: prev  Enter: connect  Ctrl+C: quit");
    flush();
}

fn draw_connecting(s: &State) {
    fill(0, 0, W, H, C_BG);
    text(20, 16, C_ACCENT, "SSH Client");
    let msg = format!("Connecting to {}:{}...", s.host, s.port);
    text(20, H / 2 - 8, C_DIM, &msg);
    flush();
}

fn draw_connected(s: &State, conn_id: u32) {
    fill(0, 0, W, H, C_BG);
    let header = format!("SSH Client  —  {}@{}:{}  (tcp id={})", s.user, s.host, s.port, conn_id);
    text(20, 14, C_ACCENT, &header);
    fill(0, 42, W, 1, C_BORDER);

    // Terminal output
    fill(TERM_X, TERM_Y, TERM_W, TERM_H, C_TERM_BG);
    for (i, line) in s.term.iter().skip(s.scroll).take(MAX_LINES).enumerate() {
        let y = TERM_Y + i as u32 * LINE_H;
        let clipped = if line.len() > 140 { &line[..140] } else { line.as_str() };
        text(TERM_X + 6, y + 2, C_TERM_FG, clipped);
    }

    // Input line
    let input_y = TERM_Y + TERM_H + 10;
    fill(TERM_X, input_y, TERM_W, 30, C_FIELD);
    border(TERM_X, input_y, TERM_W, 30, C_ACCENT);
    let prompt = format!("> {}", s.input);
    text(TERM_X + 8, input_y + 7, C_TITLE, &prompt);

    text(20, H - 22, C_HINT, "Enter: send  Up/Down: scroll  Ctrl+D: disconnect  Ctrl+C: quit");
    flush();
}

fn draw_error(msg: &str) {
    fill(0, 0, W, H, C_BG);
    text(20, 16, C_ERR, "Connection Failed");
    text(20, 50, C_DIM, msg);
    text(20, H - 22, C_HINT, "Any key to return to form");
    flush();
}

fn redraw(s: &State) {
    match &s.phase {
        Phase::Form => draw_form(s),
        Phase::Connecting => draw_connecting(s),
        Phase::Connected(id) => draw_connected(s, *id),
        Phase::Error(msg) => draw_error(msg),
    }
}

fn main() {
    let stdin = io::stdin();
    let mut s = State::new();

    redraw(&s);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        // Handle supervisor replies
        if let Some(rest) = raw.strip_prefix("REPLY:tcp-connect ") {
            if let Ok(id) = rest.trim().parse::<u32>() {
                s.phase = Phase::Connected(id);
                s.push_term(format!("Connected to {}:{}", s.host, s.port));
                // Immediately request any banner
                println!("@supervisor: tcp-recv {id}");
                let _ = io::stdout().flush();
            } else {
                s.phase = Phase::Error(rest.to_string());
            }
            redraw(&s);
            continue;
        }
        if let Some(rest) = raw.strip_prefix("REPLY:tcp-recv ") {
            if let Phase::Connected(_) = &s.phase {
                // rest = "<id> <data>"
                if let Some((_, data)) = rest.split_once(' ') {
                    if !data.trim().is_empty() {
                        s.push_term(data.to_string());
                        redraw(&s);
                    }
                }
            }
            continue;
        }
        if let Some(rest) = raw.strip_prefix("REPLY:tcp-send ") {
            // After send, immediately poll for response
            if let Phase::Connected(id) = s.phase {
                if rest.ends_with(" ok") {
                    println!("@supervisor: tcp-recv {id}");
                    let _ = io::stdout().flush();
                }
            }
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        // Ctrl+C — always quit
        if raw == "\x03" {
            if let Phase::Connected(id) = s.phase {
                println!("@supervisor: tcp-close {id}");
                let _ = io::stdout().flush();
            }
            fill(0, 0, W, H, 0x0D1117FF);
            flush();
            std::process::exit(0);
        }

        // Ctrl+D — disconnect
        if raw == "\x04" {
            if let Phase::Connected(id) = s.phase {
                println!("@supervisor: tcp-close {id}");
                let _ = io::stdout().flush();
            }
            s.phase = Phase::Form;
            s.term.clear();
            s.input.clear();
            s.scroll = 0;
            redraw(&s);
            continue;
        }

        match &s.phase {
            Phase::Form => {
                match raw.as_str() {
                    "\t" | "\x1b[B" => {
                        s.field = (s.field + 1) % 3;
                        redraw(&s);
                    }
                    "\x1b[A" => {
                        s.field = if s.field == 0 { 2 } else { s.field - 1 };
                        redraw(&s);
                    }
                    "" => {
                        // Enter: advance field or connect on last
                        if s.field < 2 {
                            s.field += 1;
                            redraw(&s);
                        } else {
                            let host = s.host.trim().to_string();
                            let port = if s.port.trim().is_empty() { "22" } else { s.port.trim() };
                            if host.is_empty() {
                                s.field = 0;
                                redraw(&s);
                            } else {
                                s.phase = Phase::Connecting;
                                redraw(&s);
                                let addr = format!("{host}:{port}");
                                println!("@supervisor: tcp-connect {addr}");
                                let _ = io::stdout().flush();
                            }
                        }
                    }
                    "\x7f" => {
                        match s.field {
                            0 => { s.host.pop(); }
                            1 => { s.port.pop(); }
                            _ => { s.user.pop(); }
                        }
                        redraw(&s);
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' {
                            match s.field {
                                0 => s.host.push(c),
                                1 => s.port.push(c),
                                _ => s.user.push(c),
                            }
                            redraw(&s);
                        }
                    }
                    _ => {}
                }
            }
            Phase::Connected(id) => {
                let id = *id;
                match raw.as_str() {
                    "\x1b[A" => {
                        s.scroll = s.scroll.saturating_sub(3);
                        redraw(&s);
                    }
                    "\x1b[B" => {
                        let max = s.term.len().saturating_sub(MAX_LINES);
                        s.scroll = (s.scroll + 3).min(max);
                        redraw(&s);
                    }
                    "" => {
                        // Enter: send input
                        let line = s.input.clone();
                        s.term.push(format!("> {line}"));
                        s.input.clear();
                        println!("@supervisor: tcp-send {id} {line}");
                        let _ = io::stdout().flush();
                        redraw(&s);
                    }
                    "\x7f" => {
                        s.input.pop();
                        redraw(&s);
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' {
                            s.input.push(c);
                            // Poll for incoming data on each keypress
                            println!("@supervisor: tcp-recv {id}");
                            let _ = io::stdout().flush();
                            redraw(&s);
                        }
                    }
                    _ => {}
                }
            }
            Phase::Error(_) => {
                // Any key to return to form
                s.phase = Phase::Form;
                redraw(&s);
            }
            Phase::Connecting => {}
        }
    }
}
