use std::io::{self, BufRead, Write};

const W: u32 = 1440;
const H: u32 = 900;
const C_BG: u32     = 0x0D1117E8;
const C_CARD: u32   = 0x161B22FF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_RUN: u32    = 0x3FB950FF;

const COLS: u32   = 3;
const CARD_W: u32 = 420;
const CARD_H: u32 = 140;
const GAP_X: u32  = 30;
const GAP_Y: u32  = 30;
const START_Y: u32 = 80;

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

struct App {
    name:   String,
    status: String,
}

fn parse_apps(reply: &str) -> Vec<App> {
    let payload = reply
        .trim_start_matches("REPLY:ps-raw:")
        .trim_start_matches("REPLY:ps-raw ");
    payload.split('|')
        .filter_map(|entry| {
            let mut fields = entry.split(':');
            let name   = fields.next()?.trim().to_string();
            let status = fields.next().unwrap_or("running").trim().to_string();
            let uptime = fields.next().unwrap_or("0").trim().to_string();
            if name.is_empty() || name == "mission-control" { return None; }
            let disp_status = format!("{} · {}s", status, uptime);
            Some(App { name, status: disp_status })
        })
        .collect()
}

fn card_pos(idx: u32) -> (u32, u32) {
    let row = idx / COLS;
    let col = idx % COLS;
    let total_w = COLS * CARD_W + (COLS - 1) * GAP_X;
    let start_x = (W - total_w) / 2;
    let cx = start_x + col * (CARD_W + GAP_X);
    let cy = START_Y + row * (CARD_H + GAP_Y);
    (cx, cy)
}

fn draw(apps: &[App], cursor: usize) {
    fill(0, 0, W, H, C_BG);

    // Header
    println!("VYOMA_DRAW:draw_text:{},{},{:#010x},l,Mission Control", W/2 - 80, 20, C_TEXT);
    text(W / 2 - 80, 50, C_HINT, "↑↓←→: navigate   Enter: focus   Esc: close");

    if apps.is_empty() {
        text(W / 2 - 60, H / 2, C_HINT, "No running apps");
        flush();
        return;
    }

    for (i, app) in apps.iter().enumerate() {
        let (cx, cy) = card_pos(i as u32);
        let is_sel = i == cursor;

        fill(cx, cy, CARD_W, CARD_H, C_CARD);
        let bc = if is_sel { C_SEL } else { C_BORDER };
        border(cx, cy, CARD_W, CARD_H, bc);

        // Selection indicator
        if is_sel {
            fill(cx, cy, CARD_W, 3, C_SEL);
        }

        // App initial letter (large)
        let initial = app.name.chars().next().unwrap_or('?').to_ascii_uppercase().to_string();
        println!("VYOMA_DRAW:draw_text:{},{},{:#010x},l,{}", cx + 20, cy + 30, if is_sel { C_SEL } else { C_DIM }, initial);

        // App name
        let name: String = app.name.chars().take(22).collect();
        text(cx + 60, cy + 36, C_TEXT, &name);

        // Status
        let st: String = app.status.chars().take(30).collect();
        text(cx + 60, cy + 60, C_DIM, &st);

        // Running dot
        fill(cx + CARD_W - 16, cy + 12, 8, 8, C_RUN);
    }

    let count = format!("{} apps running", apps.len());
    text(20, H - 20, C_HINT, &count);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut apps: Vec<App> = Vec::new();
    let mut cursor = 0usize;

    println!("@supervisor: ps-raw");
    println!("@supervisor: raise mission-control");
    let _ = io::stdout().flush();

    draw(&apps, cursor);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:ps-raw") {
            apps = parse_apps(&raw);
            if cursor >= apps.len() { cursor = apps.len().saturating_sub(1); }
            draw(&apps, cursor);
            continue;
        }

        if raw.starts_with("REPLY:") { continue; }

        let n = apps.len();
        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                if cursor >= COLS as usize { cursor -= COLS as usize; }
                draw(&apps, cursor);
            }
            "\x1b[B" => {
                if cursor + COLS as usize < n { cursor += COLS as usize; }
                draw(&apps, cursor);
            }
            "\x1b[D" => {
                if cursor > 0 { cursor -= 1; }
                draw(&apps, cursor);
            }
            "\x1b[C" => {
                if cursor + 1 < n { cursor += 1; }
                draw(&apps, cursor);
            }
            "" => {
                if let Some(app) = apps.get(cursor) {
                    println!("@supervisor: focus {}", app.name);
                    let _ = io::stdout().flush();
                }
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            _ => {}
        }
    }
}
