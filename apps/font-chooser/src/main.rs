use std::io::{self, BufRead, Write};

const W: u32 = 440;
const H: u32 = 340;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x1F4068FF;
const C_HINT: u32   = 0x6E7681FF;
const C_OK: u32     = 0x3FB950FF;

const BTN_W: u32 = 400;
const BTN_H: u32 = 72;
const BTN_X: u32 = 20;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text_m(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn text_s(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},s,{s}");
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

const OPTIONS: [(&str, &str); 3] = [
    ("Small  (8×8)",  "s"),
    ("Medium (8×16)", "m"),
    ("Large  (16×32)","l"),
];

fn draw(sel: usize, confirmed: Option<usize>) {
    fill(0, 0, W, H, C_BG);
    text_m(20, 14, C_ACCENT, "Font Size Chooser");
    fill(0, 36, W, 1, C_BORDER);

    for (i, (label, size)) in OPTIONS.iter().enumerate() {
        let by = 48 + i as u32 * (BTN_H + 8);
        let is_sel = i == sel;
        let bg = if is_sel { C_SEL } else { 0x161B22FF };
        let bc = if is_sel { C_ACCENT } else { C_BORDER };
        fill(BTN_X, by, BTN_W, BTN_H, bg);
        border(BTN_X, by, BTN_W, BTN_H, bc);
        let tc = if is_sel { C_TITLE } else { C_DIM };
        text_m(BTN_X + 14, by + 8, tc, label);
        // Preview text in the corresponding size
        match *size {
            "s" => text_s(BTN_X + 14, by + 30, C_DIM, "Preview: AaBbCc 0123"),
            "m" => text_m(BTN_X + 14, by + 30, C_DIM, "Preview: AaBbCc 0123"),
            _   => text_l(BTN_X + 14, by + 30, C_DIM, "Aa 01"),
        }
        if confirmed == Some(i) {
            text_m(BTN_X + 300, by + 28, C_OK, "✓ active");
        }
    }

    text_m(20, H - 18, C_HINT, "↑↓ navigate   Enter: select   Ctrl+C: quit");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut sel: usize = 1; // default Medium
    let mut confirmed: Option<usize> = Some(1);

    draw(sel, confirmed);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x1b[A" => {
                sel = if sel == 0 { OPTIONS.len() - 1 } else { sel - 1 };
                draw(sel, confirmed);
            }
            "\x1b[B" => {
                sel = (sel + 1) % OPTIONS.len();
                draw(sel, confirmed);
            }
            "" => {
                // Enter: select and send font-size command
                let size = OPTIONS[sel].1;
                println!("@supervisor: font-size {size}");
                let _ = io::stdout().flush();
                confirmed = Some(sel);
                draw(sel, confirmed);
            }
            _ => {}
        }
    }
}
