use std::io::{self, BufRead, Write};

const W: u32 = 760;
const H: u32 = 620;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_CARD: u32    = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;

// Die types: sides, color, label
const DICE: [(u32, u32, &str); 6] = [
    (4,  0xFFD60AFF, "d4"),   // yellow
    (6,  0x0A84FFFF, "d6"),   // blue
    (8,  0x30D158FF, "d8"),   // green
    (10, 0xBF5AF2FF, "d10"),  // purple
    (12, 0xFF9F0AFF, "d12"),  // orange
    (20, 0xFF453AFF, "d20"),  // red
];

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn text_l(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},l,{s}");
}
fn border(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:rect_border:{x},{y},{w},{h},{rgba:#010x}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

struct Roll {
    die_type: usize,   // index into DICE
    count:    usize,
    results:  [u32; 8],
    total:    u32,
}

struct App {
    sel_type: usize,
    count:    usize,
    history:  Vec<Roll>,
    seed:     u64,
    last:     Option<(usize, usize)>, // last rolled (type, count) for reroll
}

impl App {
    fn new() -> Self {
        App {
            sel_type: 5, // default d20
            count: 1,
            history: Vec::new(),
            seed: 0xD1CE12345678ABCD,
            last: None,
        }
    }

    fn roll(&mut self, die_type: usize, count: usize) {
        let sides = DICE[die_type].0;
        let mut results = [0u32; 8];
        let mut total = 0u32;
        for i in 0..count {
            self.seed = lcg(self.seed);
            let v = (self.seed >> 33) as u32 % sides + 1;
            results[i] = v;
            total += v;
        }
        if self.history.len() >= 8 { self.history.remove(0); }
        self.history.push(Roll { die_type, count, results, total });
        self.last = Some((die_type, count));
    }
}

const DIE_W: u32 = 100;
const DIE_H: u32 = 64;
const DIE_GAP: u32 = 12;
const DIE_ROW_X: u32 = (W - 6 * (DIE_W + DIE_GAP) + DIE_GAP) / 2;
const DIE_ROW_Y: u32 = HEADER_H + 16;

const HIST_Y: u32 = DIE_ROW_Y + DIE_H + 48 + 32;
const RES_BOX: u32 = 44;
const RES_GAP: u32 = 6;

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Dice Roller");
    text(180, 16, C_HINT, "←→/Tab:die type  +/-:count  Space:roll  R:reroll  c:clear");

    // Die type selector row
    for (i, &(sides, color, label)) in DICE.iter().enumerate() {
        let dx = DIE_ROW_X + i as u32 * (DIE_W + DIE_GAP);
        let is_sel = i == app.sel_type;
        let bg = if is_sel { C_SEL_BG } else { C_CARD };
        fill(dx, DIE_ROW_Y, DIE_W, DIE_H, bg);
        border(dx, DIE_ROW_Y, DIE_W, DIE_H, if is_sel { C_SEL } else { color });
        fill(dx, DIE_ROW_Y, DIE_W, 4, color);
        text(dx + 20, DIE_ROW_Y + 14, color, label);
        text(dx + 16, DIE_ROW_Y + 34, C_HINT, &format!("1–{}", sides));
    }

    // Count + roll area
    let ctrl_y = DIE_ROW_Y + DIE_H + 16;
    let (sides, color, label) = DICE[app.sel_type];
    text(20, ctrl_y, C_HINT, &format!("Rolling: {} × {} ({})", app.count, label, sides));
    text(300, ctrl_y, C_HINT, "+/- to change count");

    // Count display
    fill(W / 2 - 24, ctrl_y - 4, 48, 28, C_CARD);
    border(W / 2 - 24, ctrl_y - 4, 48, 28, C_SEL);
    text(W / 2 - 6, ctrl_y + 2, C_TEXT, &app.count.to_string());

    // Last result large display
    if let Some(last) = app.history.last() {
        let res_y = ctrl_y + 40;
        text(20, res_y - 2, C_HINT, "Last roll:");
        let lx0 = 20u32;
        for i in 0..last.count {
            let rx = lx0 + i as u32 * (RES_BOX + RES_GAP);
            let vc = DICE[last.die_type].1;
            fill(rx, res_y + 14, RES_BOX, RES_BOX, C_CARD);
            border(rx, res_y + 14, RES_BOX, RES_BOX, vc);
            let vs = last.results[i].to_string();
            let tx = rx + (RES_BOX - vs.len() as u32 * 16) / 2;
            text_l(tx, res_y + 22, vc, &vs);
        }
        let total_x = lx0 + last.count as u32 * (RES_BOX + RES_GAP) + 16;
        text(total_x, res_y + 26, C_GREEN, &format!("= {}", last.total));
    }

    // History list
    fill(12, HIST_Y - 20, W - 24, 18, C_CARD);
    text(16, HIST_Y - 18, C_HINT, "History");
    fill(12, HIST_Y - 2, W - 24, 1, C_BORDER);

    for (i, roll) in app.history.iter().rev().enumerate().take(8) {
        let ry = HIST_Y + i as u32 * 52;
        if ry + 48 > H { break; }
        fill(12, ry, W - 24, 48, C_CARD);
        let (_, hc, hlabel) = DICE[roll.die_type];
        let prefix = format!("{}×{}: ", roll.count, hlabel);
        text(20, ry + 6, hc, &prefix);
        let mut rx = 20 + prefix.len() as u32 * 9;
        for j in 0..roll.count {
            let vs = roll.results[j].to_string();
            text(rx, ry + 6, C_TEXT, &vs);
            rx += vs.len() as u32 * 9 + 8;
            if j + 1 < roll.count { text(rx - 4, ry + 6, C_HINT, ","); }
        }
        text(20, ry + 26, C_GREEN, &format!("Total: {}", roll.total));
        let max_v = DICE[roll.die_type].0;
        if roll.count == 1 && roll.results[0] == max_v {
            text(120, ry + 26, C_ORANGE, "CRITICAL!");
        }
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise dice");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[C" | "\t" => { app.sel_type = (app.sel_type + 1) % 6; }
            "\x1b[D"        => { app.sel_type = (app.sel_type + 5) % 6; }
            "+" | "="       => { if app.count < 8 { app.count += 1; } }
            "-"             => { if app.count > 1 { app.count -= 1; } }
            " " | "\r" | "" => { let (t, c) = (app.sel_type, app.count); app.roll(t, c); }
            "r" | "R"       => { if let Some((t, c)) = app.last { app.roll(t, c); } }
            "c" | "C"       => { app.history.clear(); }
            _ => {}
        }
        draw(&app);
    }
}
