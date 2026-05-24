// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 860;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;
const CHAR_H: u32 = 16;
const TERM_H: u32 = H - HEADER_H - STATUS_H; // 784
const VISIBLE_ROWS: usize = (TERM_H / CHAR_H) as usize; // 49
const HIST_ROWS: usize = VISIBLE_ROWS - 1; // 48 rows for scrollback
const LINE_MAX: usize = (W as usize - 16) / CHAR_W as usize; // 168
const SCROLLBACK: usize = 500;
const BLINK_EVERY: u64 = 20;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;

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

const PROMPT: &str = "vyoma$ ";
const PROMPT_LEN: usize = 7;

struct Term {
    lines:      Vec<(String, u32)>,
    input:      String,
    scroll:     usize,
    tick:       u64,
    cursor_vis: bool,
    history:    Vec<String>,
    hist_idx:   Option<usize>,
}

impl Term {
    fn new() -> Self {
        let mut t = Term {
            lines: Vec::new(),
            input: String::new(),
            scroll: 0,
            tick: 0,
            cursor_vis: true,
            history: Vec::new(),
            hist_idx: None,
        };
        t.push("VyomaOS Terminal v2.0  (type 'help' for commands)", C_ORANGE);
        t.push("", C_HINT);
        t
    }

    fn push(&mut self, s: &str, col: u32) {
        let disp = if s.len() > LINE_MAX { &s[..LINE_MAX] } else { s };
        self.lines.push((disp.to_string(), col));
        if self.lines.len() > SCROLLBACK {
            self.lines.remove(0);
        }
        self.scroll = 0; // snap to bottom on new output
    }

    fn run_cmd(&mut self, raw: &str) {
        let cmd = raw.trim();
        if cmd.is_empty() {
            self.push("", C_HINT);
            return;
        }
        self.history.push(cmd.to_string());
        self.hist_idx = None;

        // Echo command line in history
        let prompt_line = format!("{}{}", PROMPT, cmd);
        self.push(&prompt_line, C_TEXT);

        let parts: Vec<&str> = cmd.splitn(3, ' ').collect();
        match parts[0] {
            "help" => {
                self.push("Available commands:", C_HINT);
                self.push("  help        — show this help", C_HINT);
                self.push("  ls [path]   — list files", C_HINT);
                self.push("  cat <file>  — show file contents", C_HINT);
                self.push("  echo <text> — print text", C_HINT);
                self.push("  clear       — clear screen", C_HINT);
                self.push("  date        — show current date", C_HINT);
                self.push("  uname       — system information", C_HINT);
                self.push("  history     — command history", C_HINT);
                self.push("  exit        — use Ctrl+C to exit", C_HINT);
            }
            "ls" => {
                let path = parts.get(1).copied().unwrap_or(".");
                match path {
                    "/data" | "/data/" => {
                        self.push("kanban.csv     notes.csv      settings.toml", C_TEXT);
                        self.push("screenshot.ppm weather.toml   session.toml", C_TEXT);
                    }
                    "/apps" | "/apps/" => {
                        self.push("kanban.wasm         notes.wasm          presentation.wasm", C_TEXT);
                        self.push("md-editor.wasm      term2.wasm          chess.wasm", C_TEXT);
                        self.push("tetris.wasm         snake.wasm          minesweeper.wasm", C_TEXT);
                    }
                    "/etc" | "/etc/vyoma" => {
                        self.push("boot.toml", C_TEXT);
                    }
                    _ => {
                        self.push("hello.rs   fib.rs   README.md   boot.log", C_TEXT);
                        self.push("  /data      /apps     /etc", C_SEL);
                    }
                }
            }
            "cat" => {
                if parts.len() < 2 {
                    self.push("usage: cat <filename>", C_RED);
                } else {
                    self.cat_file(parts[1]);
                }
            }
            "echo" => {
                let msg = if parts.len() > 1 { &cmd[5..] } else { "" };
                self.push(msg, C_TEXT);
            }
            "clear" => {
                self.lines.clear();
                self.push("VyomaOS Terminal v2.0", C_ORANGE);
            }
            "date" => {
                self.push("Wed May 21 2026 14:48:00 UTC", C_YELLOW);
            }
            "uname" => {
                self.push("VyomaOS 1.0.0 wasm32-wasip2 vyomaos/amd64", C_GREEN);
                self.push("Kernel: Linux 5.10 allnoconfig  Supervisor: Rust 1.79", C_HINT);
                self.push("Runtime: Wasmtime 43.0.0 (WASI Preview 2)", C_HINT);
            }
            "history" => {
                if self.history.is_empty() {
                    self.push("(no history)", C_HINT);
                } else {
                    let hist: Vec<String> = self.history.iter().enumerate()
                        .map(|(i, c)| format!("  {:3}  {}", i + 1, c))
                        .collect();
                    for h in hist { self.push(&h, C_HINT); }
                }
            }
            "exit" | "quit" => {
                self.push("Use Ctrl+C to exit the terminal.", C_HINT);
            }
            "pwd" => {
                self.push("/home/vyoma", C_TEXT);
            }
            "whoami" => {
                self.push("vyoma", C_GREEN);
            }
            "ps" => {
                self.push("PID  NAME              STATUS", C_HINT);
                self.push("  1  supervisor        running", C_GREEN);
                self.push("  2  term2             running (this)", C_SEL);
                self.push("  3  menu-bar          running", C_GREEN);
                self.push("  4  dock              running", C_GREEN);
            }
            _ => {
                let msg = format!("term2: command not found: {}  (try 'help')", parts[0]);
                self.push(&msg, C_RED);
            }
        }
    }

    fn cat_file(&mut self, name: &str) {
        match name {
            "hello.rs" => {
                self.push("fn main() {", C_TEXT);
                self.push("    println!(\"Hello, VyomaOS!\");", C_GREEN);
                self.push("}", C_TEXT);
            }
            "fib.rs" => {
                self.push("fn fib(n: u32) -> u32 {", C_TEXT);
                self.push("    if n <= 1 { n } else { fib(n-1) + fib(n-2) }", C_TEXT);
                self.push("}", C_TEXT);
                self.push("fn main() {", C_TEXT);
                self.push("    for i in 0..10 { print!(\"{} \", fib(i)); }", C_GREEN);
                self.push("}", C_TEXT);
                self.push("// Output: 0 1 1 2 3 5 8 13 21 34", C_HINT);
            }
            "README.md" => {
                self.push("# VyomaOS", C_ORANGE);
                self.push("", C_HINT);
                self.push("A WASM-first, capability-secure operating system.", C_TEXT);
                self.push("Built with Rust + WebAssembly.", C_TEXT);
                self.push("", C_HINT);
                self.push("## Quick Start", C_SEL);
                self.push("  make build && make run-gui", C_GREEN);
                self.push("", C_HINT);
                self.push("See github.com/hbarve1/vyomaos", C_HINT);
            }
            "boot.log" => {
                self.push("[  0.000] Linux 5.10 kernel initializing", C_HINT);
                self.push("[  0.042] supervisor (PID 1) starting", C_GREEN);
                self.push("[  0.156] wasmtime 43.0.0 loaded", C_GREEN);
                self.push("[  0.248] parsing /etc/vyoma/boot.toml", C_HINT);
                self.push("[  0.312] spawning 12 apps from manifest", C_HINT);
                self.push("[  0.428] display: virtio-gpu 1440x900@32bpp", C_GREEN);
                self.push("[  0.512] 9P filesystem mounted at /data", C_HINT);
                self.push("[  0.540] boot complete in 0.540s", C_ORANGE);
            }
            _ => {
                let msg = format!("cat: {}: No such file or directory", name);
                self.push(&msg, C_RED);
                self.push("Try: hello.rs  fib.rs  README.md  boot.log", C_HINT);
            }
        }
    }
}

fn draw(t: &Term) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Terminal v2");
    text(180, 16, C_HINT, "PgUp/Dn:scroll  Ctrl+C:exit");
    // Blink indicator
    let bli = if t.cursor_vis { "●" } else { "○" };
    text(W - 32, 16, C_GREEN, bli);

    let term_area_y = HEADER_H;

    // Scrollback display
    let n = t.lines.len();
    let scroll = t.scroll.min(n.saturating_sub(1));
    let end = n.saturating_sub(scroll);
    let start = end.saturating_sub(HIST_ROWS);

    for (vi, li) in (start..end).enumerate() {
        let (ref txt, col) = t.lines[li];
        if txt.is_empty() { continue; }
        let ly = term_area_y + vi as u32 * CHAR_H;
        let disp = if txt.len() > LINE_MAX { &txt[..LINE_MAX] } else { txt };
        text(8, ly, col, disp);
    }

    // Input line (last visible row)
    let input_y = term_area_y + HIST_ROWS as u32 * CHAR_H;
    fill(0, input_y - 1, W, 1, 0x1C2128FF);
    text(8, input_y, C_GREEN, PROMPT);
    let input_x = 8 + PROMPT_LEN as u32 * CHAR_W;
    let disp_input = if t.input.len() + PROMPT_LEN > LINE_MAX {
        &t.input[..LINE_MAX - PROMPT_LEN]
    } else {
        &t.input
    };
    text(input_x, input_y, C_TEXT, disp_input);

    // Cursor
    if t.cursor_vis {
        let cx = input_x + disp_input.len() as u32 * CHAR_W;
        fill(cx, input_y, CHAR_W, CHAR_H, C_SEL);
    }

    border(0, term_area_y, W, TERM_H, C_BORDER);

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let info = format!("{} lines  scroll:{}", t.lines.len(), t.scroll);
    text(8, sb_y + 6, C_HINT, &info);
    text(W - 200, sb_y + 6, C_HINT, "VyomaOS Terminal v2.0");

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut term = Term::new();

    println!("@supervisor: raise term2");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&term);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            term.tick += 1;
            if term.tick % BLINK_EVERY == 0 {
                term.cursor_vis = !term.cursor_vis;
            }
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&term);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x7f" => { term.input.pop(); }
            "" => {
                let cmd = term.input.clone();
                term.input.clear();
                term.run_cmd(&cmd);
            }
            "\x1b[5~" => { // PgUp
                term.scroll = (term.scroll + HIST_ROWS).min(term.lines.len().saturating_sub(1));
            }
            "\x1b[6~" => { // PgDn
                term.scroll = term.scroll.saturating_sub(HIST_ROWS);
            }
            "\x1b[A" => { // Up — history prev
                if term.history.is_empty() {}
                else {
                    let new_idx = match term.hist_idx {
                        None => term.history.len() - 1,
                        Some(i) => i.saturating_sub(1),
                    };
                    term.hist_idx = Some(new_idx);
                    term.input = term.history[new_idx].clone();
                }
            }
            "\x1b[B" => { // Down — history next
                if let Some(i) = term.hist_idx {
                    if i + 1 < term.history.len() {
                        term.hist_idx = Some(i + 1);
                        term.input = term.history[i + 1].clone();
                    } else {
                        term.hist_idx = None;
                        term.input.clear();
                    }
                }
            }
            s => {
                if s.len() == 1 {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f {
                        term.input.push(b as char);
                        term.scroll = 0; // snap to bottom on input
                    }
                }
            }
        }
        draw(&term);
    }
}
