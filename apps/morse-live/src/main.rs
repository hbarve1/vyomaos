// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
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

const MORSE: &[(&str, &str)] = &[
    ("A", ".-"),    ("B", "-..."),  ("C", "-.-."),
    ("D", "-.."),   ("E", "."),     ("F", "..-."),
    ("G", "--."),   ("H", "...."),  ("I", ".."),
    ("J", ".---"),  ("K", "-.-"),   ("L", ".-.."),
    ("M", "--"),    ("N", "-."),    ("O", "---"),
    ("P", ".--."),  ("Q", "--.-"),  ("R", ".-."),
    ("S", "..."),   ("T", "-"),     ("U", "..-"),
    ("V", "...-"),  ("W", ".--"),   ("X", "-..-"),
    ("Y", "-.--"),  ("Z", "--.."),
    ("0", "-----"), ("1", ".----"), ("2", "..---"),
    ("3", "...--"), ("4", "....-"), ("5", "....."),
    ("6", "-...."), ("7", "--..."), ("8", "---.."),
    ("9", "----."),
];

fn decode_sym(sym: &str) -> &'static str {
    MORSE.iter().find(|&&(_, c)| c == sym).map(|&(l, _)| l).unwrap_or("?")
}

fn is_exact(sym: &str) -> bool {
    MORSE.iter().any(|&(_, c)| c == sym)
}

struct App {
    current: String,
    message: String,
    history: Vec<String>,
}

impl App {
    fn new() -> Self { App { current: String::new(), message: String::new(), history: Vec::new() } }

    fn commit(&mut self) {
        if !self.current.is_empty() {
            let decoded = decode_sym(&self.current);
            self.history.push(format!("{}={}", self.current, decoded));
            if self.history.len() > 8 { self.history.remove(0); }
            self.message.push_str(decoded);
            self.current.clear();
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Morse Code Decoder");
        text(190, 8, C_HINT, ". dot  - dash  Space=decode  Enter=word  Bksp=del  N=clear  Q=quit");

        let mw = 600i32;

        // Input box
        fill(20, 44, mw, 80, C_CARD);
        border(20, 44, mw, 80, C_BORDER);
        text(32, 52, C_HINT, "Typing:");
        let sym_display = if self.current.is_empty() { "..." } else { &self.current };
        text(96, 52, C_SEL, sym_display);

        if self.current.is_empty() {
            text(96, 72, C_HINT, "type . and - to build a symbol");
        } else if is_exact(&self.current) {
            let decoded = decode_sym(&self.current);
            text(96, 72, C_ORANGE, &format!("-> {} (Space to commit)", decoded));
        } else {
            let candidates: Vec<&str> = MORSE.iter()
                .filter(|&&(_, c)| c.starts_with(self.current.as_str()))
                .map(|&(l, _)| l)
                .take(6)
                .collect();
            if candidates.is_empty() {
                text(96, 72, 0xFF7B72FF, "no match — press Backspace");
            } else {
                text(96, 72, C_HINT, &format!("possible: {}", candidates.join(" ")));
            }
        }
        text(32, 108, C_HINT, "Space = decode current symbol    Enter = add word space");

        // Message area
        fill(20, 136, mw, 160, C_CARD);
        border(20, 136, mw, 160, C_BORDER);
        text(32, 144, C_HINT, "Message:");
        let msg = if self.message.is_empty() { "(start typing morse...)" } else { &self.message };
        let cpl = 60usize;
        let bytes = msg.as_bytes();
        for i in 0..4 {
            let s = i * cpl;
            if s >= bytes.len() { break; }
            let e = (s + cpl).min(bytes.len());
            if let Ok(line) = std::str::from_utf8(&bytes[s..e]) {
                text(32, 162 + i as i32 * 22, C_TEXT, line);
            }
        }

        // History row
        fill(20, 308, mw, 60, C_CARD);
        border(20, 308, mw, 60, C_BORDER);
        text(32, 316, C_HINT, "Last decoded:");
        let hstart = self.history.len().saturating_sub(7);
        for (i, entry) in self.history[hstart..].iter().enumerate() {
            text(130 + i as i32 * 72, 316, C_HINT, entry);
        }

        // Right panel — morse chart
        let rpx = 636i32;
        fill(rpx, 44, 304, 644, C_CARD);
        border(rpx, 44, 304, 644, C_BORDER);
        text(rpx + 8, 52, C_ORANGE, "Morse Alphabet");

        for (i, &(letter, code)) in MORSE.iter().enumerate() {
            let col = (i / 18) as i32;
            let row = (i % 18) as i32;
            let ex = rpx + 8 + col * 148;
            let ey = 72 + row * 17;
            let tc = if code == self.current.as_str() {
                C_ORANGE
            } else if !self.current.is_empty() && code.starts_with(self.current.as_str()) {
                C_SEL
            } else if letter.chars().next().map(|c| c.is_alphabetic()).unwrap_or(false) {
                C_HINT
            } else {
                C_GREEN
            };
            text(ex, ey, tc, &format!("{:2}: {}", letter, code));
        }

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!(
            "Symbols decoded: {}  Message: {} chars",
            self.history.len(), self.message.len()
        ));
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "." => self.current.push('.'),
            "-" => self.current.push('-'),
            " " => self.commit(),
            "" => { self.commit(); self.message.push(' '); }
            "\x7f" => { self.current.pop(); }
            "n" | "N" => { self.current.clear(); self.message.clear(); self.history.clear(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
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
