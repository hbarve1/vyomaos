use std::io::{self, BufRead, Write};

const W: u32 = 640;
const H: u32 = 140;
const C_BG: u32     = 0x161B22F0;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_SEL_BG: u32 = 0x1F4068FF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;

const SPACE_W: u32  = 100;
const SPACE_H: u32  = 70;
const SPACE_GAP: u32 = 12;
const START_Y: u32  = 24;

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

fn space_x(idx: u32, n: u32) -> u32 {
    let total = n * SPACE_W + (n - 1) * SPACE_GAP;
    let start = if total < W { (W - total) / 2 } else { 8 };
    start + idx * (SPACE_W + SPACE_GAP)
}

fn draw(spaces: &[String], cursor: usize, active: usize) {
    fill(0, 0, W, H, C_BG);
    border(0, 0, W, H, C_BORDER);

    // Title
    text(16, 6, C_HINT, "Spaces");
    text(W - 200, 6, C_HINT, "←→: select  Enter: switch  n: new  Esc: close");

    let n = spaces.len() as u32;
    for (i, name) in spaces.iter().enumerate() {
        let sx = space_x(i as u32, n);
        let is_cursor = i == cursor;
        let is_active = i == active;

        let bg = if is_active { C_SEL_BG } else { 0x21262DFF };
        fill(sx, START_Y, SPACE_W, SPACE_H, bg);
        let bc = if is_cursor { C_SEL } else if is_active { C_GREEN } else { C_BORDER };
        border(sx, START_Y, SPACE_W, SPACE_H, bc);

        if is_active {
            fill(sx + 4, START_Y + 4, SPACE_W - 8, 4, C_GREEN);
        }

        let lbl: String = name.chars().take(8).collect();
        let lw = lbl.len() as u32 * 8;
        let lx = sx + (SPACE_W - lw.min(SPACE_W)) / 2;
        let tc = if is_cursor { C_TEXT } else { C_DIM };
        text(lx, START_Y + (SPACE_H - 16) / 2, tc, &lbl);

        let num = format!("{}", i + 1);
        let nx = sx + (SPACE_W - 8) / 2;
        text(nx, START_Y + SPACE_H - 20, C_HINT, &num);
    }

    flush();
}

fn parse_spaces_reply(reply: &str) -> (usize, usize) {
    // REPLY:spaces-list <count> <current>
    let payload = reply.trim_start_matches("REPLY:spaces-list ");
    let mut parts = payload.split_whitespace();
    let count: usize  = parts.next().unwrap_or("1").parse().unwrap_or(1);
    let current: usize = parts.next().unwrap_or("0").parse().unwrap_or(0);
    (count, current)
}

fn main() {
    let stdin = io::stdin();
    let mut spaces: Vec<String> = vec!["Space 1".to_string()];
    let mut cursor = 0usize;
    let mut active = 0usize;

    println!("@supervisor: spaces-list");
    println!("@supervisor: raise spaces-switcher");
    let _ = io::stdout().flush();

    draw(&spaces, cursor, active);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:spaces-list") {
            let (count, current) = parse_spaces_reply(&raw);
            spaces = (1..=count).map(|i| format!("Space {i}")).collect();
            active = current.min(spaces.len().saturating_sub(1));
            cursor = active;
            draw(&spaces, cursor, active);
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[D" => {
                if cursor > 0 { cursor -= 1; }
                draw(&spaces, cursor, active);
            }
            "\x1b[C" => {
                if cursor + 1 < spaces.len() { cursor += 1; }
                draw(&spaces, cursor, active);
            }
            "" => {
                active = cursor;
                println!("@supervisor: spaces-switch {cursor}");
                let _ = io::stdout().flush();
                draw(&spaces, cursor, active);
                // Brief pause then close
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "n" | "N" => {
                println!("@supervisor: spaces-create");
                let _ = io::stdout().flush();
                spaces.push(format!("Space {}", spaces.len() + 1));
                cursor = spaces.len() - 1;
                active = cursor;
                draw(&spaces, cursor, active);
            }
            _ => {}
        }
    }
}
