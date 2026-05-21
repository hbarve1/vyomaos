use std::io::{self, BufRead, Write};

const W: u32 = 1440;
const H: u32 = 872;
const MAX_PANES: usize = 4;

const C_BG: u32       = 0x0D1117FF;
const C_PANE: u32     = 0x161B22FF;
const C_BORDER: u32   = 0x30363DFF;
const C_ACTIVE: u32   = 0x58A6FFFF;
const C_TEXT: u32     = 0xE6EDF3FF;
const C_DIM: u32      = 0x8B949EFF;
const C_HINT: u32     = 0x6E7681FF;
const C_PROMPT: u32   = 0x3FB950FF;
const C_STATUS_BG: u32 = 0x161B22FF;

struct Pane {
    title:  String,
    lines:  Vec<String>,
    input:  String,
    scroll: usize,
}

impl Pane {
    fn new(idx: usize) -> Self {
        Pane {
            title: format!("Pane {}", idx + 1),
            lines: vec![
                format!("VyomaOS tmux — Pane {}", idx + 1),
                "Type commands below (simulated shell)".to_string(),
                "".to_string(),
            ],
            input: String::new(),
            scroll: 0,
        }
    }

    fn push_prompt(&mut self) {
        let cmd = self.input.trim().to_string();
        if cmd.is_empty() { return; }
        self.lines.push(format!("$ {cmd}"));
        let response = match cmd.as_str() {
            "ls"       => "data/  bin/  etc/".to_string(),
            "pwd"      => "/".to_string(),
            "whoami"   => "vyoma".to_string(),
            "uname -a" => "VyomaOS 0.88 wasm32-wasip2 Wasmtime/43".to_string(),
            "uptime"   => "up N/A, 1 user, load avg: 0.00".to_string(),
            "ps"       => "@supervisor: ps-raw (see process-inspector for details)".to_string(),
            "clear"    => { self.lines.clear(); String::new() }
            _          => format!("{}: command dispatched", cmd),
        };
        if !response.is_empty() { self.lines.push(response); }
        self.input.clear();
    }
}

fn pane_rect(idx: usize, n: usize) -> (u32, u32, u32, u32) {
    let status_h = 22u32;
    let usable_h = H - status_h;
    match n {
        1 => (0, 0, W, usable_h),
        2 => {
            let pw = W / 2;
            (idx as u32 * pw, 0, pw, usable_h)
        }
        3 | 4 => {
            let cols = 2u32;
            let rows = if n <= 2 { 1u32 } else { 2u32 };
            let pw = W / cols;
            let ph = usable_h / rows;
            let col = idx as u32 % cols;
            let row = idx as u32 / cols;
            (col * pw, row * ph, pw, ph)
        }
        _ => (0, 0, W, usable_h),
    }
}

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

fn draw_pane(p: &Pane, rx: u32, ry: u32, rw: u32, rh: u32, active: bool) {
    fill(rx, ry, rw, rh, C_PANE);
    let bc = if active { C_ACTIVE } else { C_BORDER };
    border(rx, ry, rw, rh, bc);

    // Title bar
    let title_bg = if active { 0x1F4068FF } else { 0x21262DFF };
    fill(rx + 1, ry + 1, rw - 2, 18, title_bg);
    let tc = if active { C_TEXT } else { C_DIM };
    text(rx + 8, ry + 3, tc, &p.title);
    if active {
        text(rx + rw - 64, ry + 3, C_ACTIVE, "ACTIVE");
    }

    // Content area
    let content_y = ry + 20;
    let content_h = rh - 38;
    let line_h = 16u32;
    let visible = (content_h / line_h) as usize;

    let start = if p.lines.len() > visible {
        p.lines.len() - visible
    } else {
        0
    };

    for (i, line) in p.lines[start..].iter().enumerate() {
        let ly = content_y + i as u32 * line_h;
        if ly + line_h > ry + rh - 18 { break; }
        let lc = if line.starts_with("$ ") { C_PROMPT } else { C_TEXT };
        let truncated: String = line.chars().take((rw / 8 - 2) as usize).collect();
        text(rx + 6, ly, lc, &truncated);
    }

    // Prompt row
    let prompt_y = ry + rh - 18;
    fill(rx + 1, prompt_y, rw - 2, 16, 0x0D1117FF);
    let prompt = format!("$ {}_", p.input);
    let truncated_prompt: String = prompt.chars().take((rw / 8 - 2) as usize).collect();
    text(rx + 6, prompt_y + 2, C_PROMPT, &truncated_prompt);
}

fn draw(panes: &[Pane], active: usize) {
    fill(0, 0, W, H, C_BG);

    let n = panes.len();
    for (i, pane) in panes.iter().enumerate() {
        let (rx, ry, rw, rh) = pane_rect(i, n);
        draw_pane(pane, rx, ry, rw, rh, i == active);
    }

    // Status bar
    let sb_y = H - 22;
    fill(0, sb_y, W, 22, C_STATUS_BG);
    fill(0, sb_y, W, 1, C_BORDER);
    let status = format!("tmux  [{}]  {}/{} panes", panes[active].title, active + 1, n);
    text(12, sb_y + 4, C_DIM, &status);
    text(W - 440, sb_y + 4, C_HINT, "Tab: next pane  Ctrl+N: new  Ctrl+W: close  Esc: quit");

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut panes: Vec<Pane> = vec![Pane::new(0), Pane::new(1)];
    let mut active = 0usize;

    println!("@supervisor: raise tmux");
    let _ = io::stdout().flush();

    draw(&panes, active);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        let n = panes.len();
        match raw.as_str() {
            "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x03" => {
                // Ctrl+C in active pane — add ^C to output
                panes[active].lines.push("^C".to_string());
                panes[active].input.clear();
                draw(&panes, active);
            }
            "\x09" => { // Tab
                active = (active + 1) % n;
                draw(&panes, active);
            }
            "\x0e" => { // Ctrl+N
                if n < MAX_PANES {
                    panes.push(Pane::new(n));
                    active = n;
                }
                draw(&panes, active);
            }
            "\x17" => { // Ctrl+W — close active pane
                if n > 1 {
                    panes.remove(active);
                    if active >= panes.len() { active = panes.len() - 1; }
                }
                draw(&panes, active);
            }
            "\x7f" => {
                panes[active].input.pop();
                draw(&panes, active);
            }
            "" => {
                panes[active].push_prompt();
                draw(&panes, active);
            }
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if c.is_ascii_graphic() || c == ' ' {
                    panes[active].input.push(c);
                    draw(&panes, active);
                }
            }
            _ => {}
        }
    }
}
