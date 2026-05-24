// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1100;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const INPUT_H: u32 = 36;
const LINE_H: u32 = 18;
const CHAR_W: u32 = 8;
const CONTENT_Y: u32 = HEADER_H;
const INPUT_Y: u32 = H - INPUT_H;
const VISIBLE_LINES: usize = ((INPUT_Y - CONTENT_Y) / LINE_H) as usize; // 37
const MAX_HISTORY: usize = 200;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;

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

const CMDS: &[&str] = &[
    "help", "clear", "ls", "echo", "date", "version",
    "whoami", "pwd", "cat", "ping", "exit",
];

const FAKE_FILES: &[(&str, &str)] = &[
    ("code.rs",    "fn main() {\n    println!(\"Hello from VyomaOS!\");\n}"),
    ("readme.md",  "# VyomaOS\nWASM-first OS. Boot < 5s. Capability-secure by design."),
    ("notes.txt",  "Meeting: discuss WASM runtime\nTODO: add GPU acceleration"),
    ("data.csv",   "name,value,tag\nalpha,42,a\nbeta,17,b\ngamma,99,c"),
];

struct Term {
    output:      Vec<(String, u32)>,
    cmd_hist:    Vec<String>,
    cmd_hist_idx: Option<usize>,
    input:       String,
    scroll:      usize,
    ticks:       u64,
    pending_ping: bool,
}

impl Term {
    fn new() -> Self {
        let mut t = Term {
            output: Vec::new(),
            cmd_hist: Vec::new(),
            cmd_hist_idx: None,
            input: String::new(),
            scroll: 0,
            ticks: 0,
            pending_ping: false,
        };
        t.push("VyomaOS Terminal v0.1.0", C_GREEN);
        t.push("Type 'help' for available commands.", C_HINT);
        t.push("", C_TEXT);
        t
    }

    fn push(&mut self, s: &str, color: u32) {
        if self.output.len() >= MAX_HISTORY { self.output.remove(0); }
        self.output.push((s.to_string(), color));
        self.scroll = 0;
    }

    fn push_prompt(&mut self) {
        // Append prompt line as a special marker — not actually pushed to output
        // The prompt is rendered inline with input
    }

    fn tab_complete(&mut self) {
        let prefix = self.input.clone();
        let matches: Vec<&str> = CMDS.iter()
            .filter(|&&c| c.starts_with(prefix.as_str()))
            .copied()
            .collect();
        if matches.len() == 1 {
            self.input = matches[0].to_string();
        } else if matches.len() > 1 {
            self.push(&format!("  {}", matches.join("  ")), C_HINT);
        }
    }

    fn hist_up(&mut self) {
        if self.cmd_hist.is_empty() { return; }
        let new_idx = match self.cmd_hist_idx {
            None => self.cmd_hist.len() - 1,
            Some(i) => i.saturating_sub(1),
        };
        self.cmd_hist_idx = Some(new_idx);
        self.input = self.cmd_hist[new_idx].clone();
    }

    fn hist_down(&mut self) {
        match self.cmd_hist_idx {
            None => {}
            Some(i) => {
                if i + 1 < self.cmd_hist.len() {
                    self.cmd_hist_idx = Some(i + 1);
                    self.input = self.cmd_hist[i + 1].clone();
                } else {
                    self.cmd_hist_idx = None;
                    self.input.clear();
                }
            }
        }
    }

    fn exec(&mut self, raw_cmd: &str) -> bool {
        let raw_cmd = raw_cmd.trim();
        if raw_cmd.is_empty() { return false; }

        // Echo the command with prompt
        let prompt_line = format!("vyoma> {}", raw_cmd);
        self.push(&prompt_line, C_ORANGE);

        // Save to cmd history
        if self.cmd_hist.last().map(|s| s.as_str()) != Some(raw_cmd) {
            self.cmd_hist.push(raw_cmd.to_string());
            if self.cmd_hist.len() > 50 { self.cmd_hist.remove(0); }
        }
        self.cmd_hist_idx = None;

        let parts: Vec<&str> = raw_cmd.splitn(2, ' ').collect();
        let cmd = parts[0];
        let arg = parts.get(1).copied().unwrap_or("");

        match cmd {
            "help" => {
                self.push("Available commands:", C_SEL);
                for c in CMDS {
                    self.push(&format!("  {}", c), C_TEXT);
                }
            }
            "clear" => {
                self.output.clear();
            }
            "ls" => {
                self.push("/data/", C_SEL);
                for &(name, _) in FAKE_FILES {
                    self.push(&format!("  {}", name), C_TEXT);
                }
            }
            "echo" => {
                self.push(arg, C_TEXT);
            }
            "date" => {
                let day = self.ticks / 86400 + 1;
                let hour = (self.ticks % 86400) / 3600;
                let min = (self.ticks % 3600) / 60;
                let sec = self.ticks % 60;
                self.push(&format!("2026-05-21 {:02}:{:02}:{:02} (day +{})", hour, min, sec, day), C_TEXT);
            }
            "version" => {
                self.push("VyomaOS 0.1.0 — WASM-first capability-secure OS", C_TEXT);
                self.push("Kernel: Linux 5.10 | Runtime: Wasmtime 43.0.0", C_HINT);
            }
            "whoami" => { self.push("guest", C_TEXT); }
            "pwd" => { self.push("/home/guest", C_TEXT); }
            "cat" => {
                let target = arg.trim_start_matches("/data/");
                match FAKE_FILES.iter().find(|&&(n, _)| n == target) {
                    Some(&(_, content)) => {
                        for line in content.split('\n') {
                            self.push(line, C_TEXT);
                        }
                    }
                    None => { self.push(&format!("cat: {}: No such file or directory", arg), C_RED); }
                }
            }
            "ping" => {
                self.pending_ping = true;
                return true; // signal to send @supervisor: ping
            }
            "exit" => {
                self.push("Goodbye.", C_HINT);
                return false;
            }
            _ => {
                self.push(&format!("{}: command not found. Try 'help'.", cmd), C_RED);
            }
        }
        false
    }
}

fn draw(t: &Term) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Terminal");
    text(140, 16, C_HINT, "VyomaOS shell emulator");
    text(500, 16, C_HINT, "Tab:complete  Up/Dn:history  PgUp/Dn:scroll");

    // Scrollback
    let total = t.output.len();
    let scroll_clamped = t.scroll.min(total.saturating_sub(VISIBLE_LINES));
    let start = total.saturating_sub(VISIBLE_LINES + scroll_clamped);
    let end = (start + VISIBLE_LINES).min(total);

    for (i, idx) in (start..end).enumerate() {
        let (ref line, color) = t.output[idx];
        let y = CONTENT_Y + i as u32 * LINE_H + 2;
        // Truncate line to fit
        let display = if line.len() * CHAR_W as usize > (W - 16) as usize {
            &line[..(W as usize - 16) / CHAR_W as usize]
        } else {
            line.as_str()
        };
        text(8, y, color, display);
    }

    // Scroll indicator
    if t.scroll > 0 {
        text(W - 120, CONTENT_Y + 4, C_HINT, &format!("[+{} lines]", t.scroll));
    }

    // Input line
    fill(0, INPUT_Y, W, INPUT_H, C_CARD);
    fill(0, INPUT_Y, W, 1, C_BORDER);
    let prompt = "vyoma> ";
    text(8, INPUT_Y + 10, C_ORANGE, prompt);
    let cursor_x = 8 + (prompt.len() + t.input.len()) as u32 * CHAR_W;
    text(8 + prompt.len() as u32 * CHAR_W, INPUT_Y + 10, C_TEXT, &t.input);
    fill(cursor_x, INPUT_Y + 8, 2, LINE_H, C_SEL);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut term = Term::new();

    println!("@supervisor: raise terminal");
    let _ = io::stdout().flush();
    draw(&term);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            if term.pending_ping {
                term.pending_ping = false;
                term.push("pong (supervisor responded)", C_GREEN);
                draw(&term);
            }
            term.ticks += 1;
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b" => {}
            "\x1b[A" => { term.hist_up(); }
            "\x1b[B" => { term.hist_down(); }
            "\x1b[5~" => { term.scroll += VISIBLE_LINES / 2; }
            "\x1b[6~" => { term.scroll = term.scroll.saturating_sub(VISIBLE_LINES / 2); }
            "\x1b[H" => { term.scroll = MAX_HISTORY; }
            "\x1b[F" => { term.scroll = 0; }
            "\t" => { term.tab_complete(); }
            "\x7f" => { term.input.pop(); }
            "" => {
                let cmd = term.input.clone();
                term.input.clear();
                let do_ping = term.exec(&cmd);
                if do_ping {
                    term.push("PING sent to supervisor...", C_HINT);
                    println!("@supervisor: ping");
                    let _ = io::stdout().flush();
                }
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7f { term.input.push(b as char); }
            }
            _ => {}
        }
        draw(&term);
    }
}
