// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 800;
const H: i32 = 560;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_CARD: u32   = 0x161B22FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

// Base index: 0=bin, 1=oct, 2=dec, 3=hex
const BASE_NAMES:   &[&str] = &["Binary",   "Octal",  "Decimal", "Hex"];
const BASE_KEYS:    &[&str] = &["B",         "O",      "D",       "X"];
const BASE_RADIX:   &[u64]  = &[2,           8,        10,        16];
const BASE_COLORS:  &[u32]  = &[0xFF7B72FF,  0xFFA657FF, 0x3FB950FF, 0x58A6FFFF];
const BASE_VALID:   &[&str] = &["01", "01234567", "0123456789", "0123456789abcdefABCDEF"];

fn is_valid_digit(c: u8, base: usize) -> bool {
    BASE_VALID[base].contains(c as char)
}

fn convert(buf: &str, from_base: usize) -> Option<u64> {
    if buf.is_empty() { return Some(0); }
    let radix = BASE_RADIX[from_base];
    let mut val: u64 = 0;
    for ch in buf.chars() {
        let d = ch.to_digit(radix as u32)? as u64;
        val = val.checked_mul(radix)?.checked_add(d)?;
    }
    Some(val)
}

fn format_base(val: u64, base: usize) -> String {
    if val == 0 { return "0".to_string(); }
    let radix = BASE_RADIX[base];
    let mut digits = Vec::new();
    let mut v = val;
    while v > 0 {
        let d = (v % radix) as u32;
        digits.push(char::from_digit(d, radix as u32).unwrap_or('?'));
        v /= radix;
    }
    digits.iter().rev().collect()
}

// Split binary representation into groups of 8
fn format_binary_groups(val: u64) -> String {
    let raw = format_base(val, 0);
    let pad_to = if raw.len() <= 8 { 8 }
                 else if raw.len() <= 16 { 16 }
                 else if raw.len() <= 32 { 32 }
                 else { 64 };
    let padded = format!("{:0>width$}", raw, width = pad_to);
    padded.as_bytes().chunks(8)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>().join(" ")
}

struct App {
    buf:    String,
    base:   usize,
    value:  Option<u64>,
}

impl App {
    fn new() -> Self {
        App { buf: String::new(), base: 2, value: Some(0) }
    }

    fn reparse(&mut self) {
        self.value = convert(&self.buf, self.base);
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Number Base Converter");
        text(260, 8, C_HINT, "B/O/D/X or 1-4: select base  Bksp: delete  C: clear  Q: quit");

        // Active base selector tabs
        for (i, name) in BASE_NAMES.iter().enumerate() {
            let tx = 20 + i as i32 * 190;
            let sel = i == self.base;
            let bg = if sel { 0x1A2040FF } else { C_CARD };
            fill(tx, 40, 186, 28, bg);
            border(tx, 40, 186, 28, if sel { BASE_COLORS[i] } else { C_BORDER });
            let tc = if sel { BASE_COLORS[i] } else { C_HINT };
            text(tx + 8, 48, tc, &format!("{} ({})", name, BASE_KEYS[i]));
        }

        // Input box
        let val = self.value;
        let overflow = val.is_none();
        let ib_c = if overflow { C_RED } else { C_SEL };
        fill(20, 76, W - 40, 44, C_CARD);
        border(20, 76, W - 40, 44, ib_c);
        text(32, 88, C_HINT, &format!("Input ({}):", BASE_NAMES[self.base]));
        let cursor = format!("{}|", self.buf);
        text(180, 88, BASE_COLORS[self.base], &cursor);
        if overflow {
            text(W - 110, 88, C_RED, "OVERFLOW");
        }

        // 4 output panels
        let pv = val.unwrap_or(0);
        let panel_y = 132i32;
        let panel_h = 80i32;
        let panel_gap = 8i32;
        let panel_w = (W - 40 - panel_gap * 3) / 4;

        for i in 0..4usize {
            let px = 20 + i as i32 * (panel_w + panel_gap);
            let sel = i == self.base;
            let bg = if sel { 0x121820FF } else { C_CARD };
            fill(px, panel_y, panel_w, panel_h, bg);
            border(px, panel_y, panel_w, panel_h, if sel { BASE_COLORS[i] } else { C_BORDER });
            text(px + 8, panel_y + 8, BASE_COLORS[i], BASE_NAMES[i]);
            let repr = if overflow { "ERROR".to_string() } else { format_base(pv, i) };
            let tc = if overflow { C_RED } else { C_TEXT };
            // Truncate to fit
            let max_c = (panel_w / 8 - 2) as usize;
            let disp: String = repr.chars().rev().take(max_c).collect::<String>().chars().rev().collect();
            text(px + 8, panel_y + 30, tc, &disp);
            text(px + 8, panel_y + 56, C_HINT, &format!("base {}", BASE_RADIX[i]));
        }

        // Binary groups display
        let bg_y = panel_y + panel_h + 16;
        fill(20, bg_y, W - 40, 44, C_CARD);
        border(20, bg_y, W - 40, 44, C_BORDER);
        text(32, bg_y + 8, C_HINT, "Bits:");
        if !overflow {
            text(88, bg_y + 8, 0xFF7B72FF, &format_binary_groups(pv));
        }

        // Bit-width breakdown
        let bw_y = bg_y + 60;
        fill(20, bw_y, W - 40, 80, C_CARD);
        border(20, bw_y, W - 40, 80, C_BORDER);
        text(32, bw_y + 8, C_ORANGE, "Bit widths:");
        if !overflow {
            let widths = [
                ("8-bit",  0xFFu64,          if pv <= 0xFF { C_GREEN } else { C_RED }),
                ("16-bit", 0xFFFFu64,        if pv <= 0xFFFF { C_GREEN } else { C_RED }),
                ("32-bit", 0xFFFFFFFFu64,    if pv <= 0xFFFFFFFF { C_GREEN } else { C_RED }),
                ("64-bit", u64::MAX,          C_GREEN),
            ];
            for (wi, (label, _max, tc)) in widths.iter().enumerate() {
                text(32 + wi as i32 * 180, bw_y + 30, *tc, label);
                let fits = if *tc == C_GREEN { "fits" } else { "overflow" };
                text(32 + wi as i32 * 180, bw_y + 52, C_HINT, fits);
            }
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        let dec_str = val.map(|v| format!("{}", v)).unwrap_or_else(|| "OVERFLOW".into());
        text(12, H - 18, C_HINT, &format!("Value: {}  Base: {}", dec_str, BASE_RADIX[self.base]));
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "b" | "B" | "1" => self.base = 0,
            "o" | "O" | "2" => self.base = 1,
            "d" | "D" | "3" => self.base = 2,
            "x" | "X" | "4" => self.base = 3,
            "c" | "C" => { self.buf.clear(); self.value = Some(0); }
            "\x7f" => { self.buf.pop(); self.reparse(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {
                if line.len() == 1 {
                    let b = line.as_bytes()[0];
                    if is_valid_digit(b, self.base) {
                        self.buf.push(b as char);
                        self.reparse();
                    }
                }
            }
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
