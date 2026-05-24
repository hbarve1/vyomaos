// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 760;
const H: i32 = 560;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_CARD: u32   = 0x161B22FF;
const C_PURPLE: u32 = 0xBC8CFFFF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

fn log2_x100(n: usize) -> usize {
    if n <= 1 { return 0; }
    let bits = (usize::BITS - n.leading_zeros() - 1) as usize;
    let lo = 1usize << bits;
    let frac = (n - lo) * 100 / lo;
    bits * 100 + frac
}

const UPPER:   &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER:   &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const DIGITS:  &[u8] = b"0123456789";
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{}|;:,.<>?";

struct App {
    length:  usize,
    upper:   bool,
    lower:   bool,
    digits:  bool,
    symbols: bool,
    history: Vec<String>,
    seed:    u64,
    copied:  bool,
}

impl App {
    fn new() -> Self {
        let mut app = App {
            length: 16, upper: true, lower: true, digits: true, symbols: true,
            history: Vec::new(), seed: 0x5AFE_1234_5678u64, copied: false,
        };
        app.generate();
        app
    }

    fn charset(&self) -> Vec<u8> {
        let mut v: Vec<u8> = Vec::new();
        if self.upper   { v.extend_from_slice(UPPER); }
        if self.lower   { v.extend_from_slice(LOWER); }
        if self.digits  { v.extend_from_slice(DIGITS); }
        if self.symbols { v.extend_from_slice(SYMBOLS); }
        v
    }

    fn enabled_count(&self) -> usize {
        [self.upper, self.lower, self.digits, self.symbols]
            .iter().filter(|&&x| x).count()
    }

    fn generate(&mut self) {
        let chars = self.charset();
        if chars.is_empty() { return; }
        self.seed = lcg(self.seed);
        let mut s = self.seed;
        let n = chars.len();
        let mut pwd = String::with_capacity(self.length);
        for _ in 0..self.length {
            s = lcg(s);
            pwd.push(chars[((s >> 33) as usize) % n] as char);
        }
        self.history.insert(0, pwd);
        if self.history.len() > 10 { self.history.pop(); }
        self.copied = false;
    }

    fn entropy_bits(&self) -> usize {
        let cs = self.charset().len();
        self.length * log2_x100(cs) / 100
    }

    fn strength(&self) -> (&str, u32) {
        match self.entropy_bits() {
            0..=39  => ("Weak",        C_RED),
            40..=59 => ("Fair",        C_YELLOW),
            60..=79 => ("Good",        C_ORANGE),
            80..=99 => ("Strong",      C_GREEN),
            _       => ("Very Strong", C_PURPLE),
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Password Generator");
        text(W - 384, 8, C_HINT, "Enter=new  C=copy  U/L/D/S=charset  Q=quit");

        // Length control row
        text(12,  40, C_HINT, "Length:");
        fill(76,  38, 32, 18, C_CARD);
        text(80,  40, C_ORANGE, &format!("{:2}", self.length));
        text(116, 40, C_HINT, "+/-  (8\u{2013}64)");

        // Charset toggles row
        text(12, 64, C_HINT, "Charset:");
        let toggle_vals  = [self.upper, self.lower, self.digits, self.symbols];
        let toggle_keys  = ["U", "L", "D", "S"];
        let toggle_names = ["A-Z", "a-z", "0-9", "!@#"];
        for i in 0..4usize {
            let tx = 76 + i as i32 * 90;
            let (bg, fc) = if toggle_vals[i] { (C_GREEN, C_BG) } else { (C_CARD, C_HINT) };
            fill(tx, 60, 80, 20, bg);
            println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", tx, 60, 80, 20, C_BORDER);
            text(tx + 4,  62, fc, toggle_keys[i]);
            text(tx + 18, 62, fc, toggle_names[i]);
        }
        let cs_size = self.charset().len();
        text(76 + 4 * 90 + 8, 64, C_HINT, &format!("{} chars", cs_size));

        // Password display card
        fill(8, 88, W - 16, 40, C_CARD);
        println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", 8, 88, W - 16, 40, C_BORDER);
        let pwd = self.history.first().map(|s| s.as_str()).unwrap_or("(none)");
        let max_c = ((W - 32) / 8) as usize;
        let display = if pwd.len() > max_c { &pwd[..max_c] } else { pwd };
        let pwd_c = if self.copied { C_GREEN } else { C_TEXT };
        text(16, 100, pwd_c, display);
        if self.copied {
            text(W - 108, 100, C_GREEN, "[COPIED]");
        }

        // Entropy bar
        let ebits = self.entropy_bits();
        let (strength_label, sc) = self.strength();
        let bar_max_w = 380i32;
        let bar_w = ((ebits as i32) * bar_max_w / 128).min(bar_max_w);
        fill(8, 136, bar_max_w, 14, C_CARD);
        fill(8, 136, bar_w.max(2), 14, sc);
        println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", 8, 136, bar_max_w, 14, C_BORDER);
        text(396, 136, sc, &format!("{} bits  {}", ebits, strength_label));

        // Crack time + stats
        let crack_str = match ebits {
            0..=29  => "Instant",
            30..=39 => "Minutes",
            40..=49 => "Days",
            50..=59 => "Years",
            60..=79 => "Centuries",
            80..=99 => "~10^14 years",
            _       => "Beyond universe age",
        };
        text(8, 158, C_HINT, &format!("Estimated crack time: {}", crack_str));
        text(8, 174, C_HINT,
             &format!("Charset: {} chars  |  Length: {}  |  Entropy: {} bits",
                 cs_size, self.length, ebits));

        // History panel
        let hp_y = 196i32;
        let hp_h = H - hp_y - 28;
        fill(8, hp_y, W - 16, hp_h, C_CARD);
        println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", 8, hp_y, W - 16, hp_h, C_BORDER);
        text(16, hp_y + 8, C_HINT, &format!("HISTORY ({})", self.history.len()));

        for (i, pwd_entry) in self.history.iter().enumerate() {
            let py = hp_y + 28 + i as i32 * 26;
            let nc = if i == 0 { C_TEXT } else { C_HINT };
            text(16, py, C_HINT, &format!("{:2}.", i + 1));
            let max_h = ((W - 80) / 8) as usize;
            let show = if pwd_entry.len() > max_h { &pwd_entry[..max_h] } else { pwd_entry.as_str() };
            text(44, py, nc, show);
            // Length badge for each entry
            text(W - 80, py, C_HINT, &format!("[{}]", pwd_entry.len()));
        }

        // Bottom bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT,
             &format!("Generated: {}  |  Length: {}  |  Charset: {} chars  |  Press Enter to generate",
                 self.history.len(), self.length, cs_size));

        flush();
    }

    fn toggle_safe(&mut self, which: u8) {
        let count = self.enabled_count();
        let is_last = count == 1;
        match which {
            0 => { if !(self.upper   && is_last) { self.upper   = !self.upper;   } }
            1 => { if !(self.lower   && is_last) { self.lower   = !self.lower;   } }
            2 => { if !(self.digits  && is_last) { self.digits  = !self.digits;  } }
            3 => { if !(self.symbols && is_last) { self.symbols = !self.symbols; } }
            _ => {}
        }
    }

    fn handle(&mut self, line: &str) {
        match line {
            "" => { self.generate(); self.draw(); }
            "+" | "=" => { if self.length < 64 { self.length += 1; } self.draw(); }
            "-" | "_" => { if self.length > 8  { self.length -= 1; } self.draw(); }
            "u" | "U" => { self.toggle_safe(0); self.draw(); }
            "l" | "L" => { self.toggle_safe(1); self.draw(); }
            "d" | "D" => { self.toggle_safe(2); self.draw(); }
            "s" | "S" => { self.toggle_safe(3); self.draw(); }
            "c" | "C" => {
                if let Some(pwd) = self.history.first() {
                    println!("@supervisor: clipboard-set {}", pwd);
                    self.copied = true;
                    self.draw();
                }
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
