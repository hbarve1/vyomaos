// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const ROWS_VISIBLE: usize = 30;

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

const CTRL_NAMES: &[&str] = &[
    "NUL","SOH","STX","ETX","EOT","ENQ","ACK","BEL",
    "BS", "HT", "LF", "VT", "FF", "CR", "SO", "SI",
    "DLE","DC1","DC2","DC3","DC4","NAK","SYN","ETB",
    "CAN","EM", "SUB","ESC","FS", "GS", "RS", "US",
];

fn char_display(code: usize) -> &'static str {
    if code < 32 { CTRL_NAMES[code] }
    else if code == 32 { "SPC" }
    else if code == 127 { "DEL" }
    else { "" }
}

fn row_color(code: usize) -> u32 {
    if code < 32 { C_ORANGE }
    else if code == 127 { C_RED }
    else { C_GREEN }
}

struct App {
    scroll:     usize,
    selected:   usize,
    search_mode: bool,
    search_buf: String,
}

impl App {
    fn new() -> Self { App { scroll: 0, selected: 0, search_mode: false, search_buf: String::new() } }

    fn jump_to(&mut self, code: usize) {
        let code = code.min(127);
        self.selected = code;
        if code < self.scroll || code >= self.scroll + ROWS_VISIBLE {
            self.scroll = code.saturating_sub(ROWS_VISIBLE / 2);
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "ASCII Table");
        text(140, 8, C_HINT, "Up/Dn:scroll  +/-:page  F:search  Q:quit");

        // Column headers
        let hy = 40i32;
        fill(0, hy, W, 18, C_CARD);
        let cols = [(12,"Dec"),(70,"Hex"),(118,"Oct"),(174,"Bin"),(264,"Char"),(320,"Name/Description")];
        for &(cx, label) in &cols {
            text(cx, hy + 4, C_ORANGE, label);
        }
        fill(0, hy + 18, W, 1, C_BORDER);

        // Table rows
        let row_h = 18i32;
        let table_y = 60i32;

        for vi in 0..ROWS_VISIBLE {
            let code = self.scroll + vi;
            if code > 127 { break; }
            let ry = table_y + vi as i32 * row_h;
            let sel = code == self.selected;

            if sel {
                fill(0, ry, W, row_h, 0x1C2433FF);
            } else if vi % 2 == 0 {
                fill(0, ry, W, row_h, 0x0A0E14FF);
            }

            let tc = row_color(code);

            // Dec
            text(12,  ry + 4, if sel { C_TEXT } else { tc }, &format!("{:3}", code));
            // Hex
            text(70,  ry + 4, if sel { C_TEXT } else { C_HINT }, &format!("0x{:02X}", code));
            // Oct
            text(118, ry + 4, if sel { C_TEXT } else { C_HINT }, &format!("0{:03o}", code));
            // Bin
            text(174, ry + 4, if sel { C_TEXT } else { C_HINT }, &format!("{:08b}", code));
            // Char
            let ch_disp = char_display(code);
            if !ch_disp.is_empty() {
                text(264, ry + 4, tc, ch_disp);
            } else {
                let ch_bytes = [code as u8];
                text(264, ry + 4, tc, std::str::from_utf8(&ch_bytes).unwrap_or("?"));
            }
            // Name / Description
            let name = ascii_name(code);
            text(320, ry + 4, if sel { C_TEXT } else { C_HINT }, name);

            if sel {
                fill(0, ry, 4, row_h, C_SEL);
            }
        }

        // Scrollbar
        let sb_x = W - 8;
        let sb_h = H - table_y - 28;
        fill(sb_x, table_y, 6, sb_h, C_CARD);
        let thumb_y = table_y + (self.scroll as i32 * sb_h / 128);
        let thumb_h = (ROWS_VISIBLE as i32 * sb_h / 128).max(8);
        fill(sb_x, thumb_y, 6, thumb_h, C_BORDER);

        // Detail panel for selected char
        let dp_y = H - 68;
        fill(0, dp_y, W, 44, C_CARD);
        border(0, dp_y, W, 44, C_BORDER);
        let code = self.selected;
        let tc = row_color(code);
        text(12, dp_y + 6, C_ORANGE, &format!("Char {}:", code));
        text(90, dp_y + 6, tc, &format!("Dec={} Hex=0x{:02X} Oct=0{:03o} Bin={:08b}", code, code, code, code));
        let ch_d = char_display(code);
        let ch_s = if !ch_d.is_empty() { ch_d.to_string() } else { format!("'{}'", code as u8 as char) };
        text(12, dp_y + 26, tc, &format!("Char: {}   {}", ch_s, ascii_name(code)));

        // Search mode
        if self.search_mode {
            fill(0, H - 24, W, 24, C_ORANGE);
            text(12, H - 18, 0x000000FF, &format!("Search: {}|  (Enter=go Esc=cancel)", self.search_buf));
        } else {
            fill(0, H - 24, W, 24, C_HEADER);
            text(12, H - 18, C_HINT, &format!("Row {}/127  F=search  +/-=page  selected={}",
                 self.scroll, self.selected));
        }
        flush();
    }

    fn handle(&mut self, line: &str) {
        if self.search_mode {
            match line {
                "\x1b" | "\x03" => { self.search_mode = false; self.search_buf.clear(); }
                "\x7f" => { self.search_buf.pop(); }
                "\r" | "" => {
                    // Try parse as decimal number first, then as char
                    if let Ok(n) = self.search_buf.parse::<usize>() {
                        self.jump_to(n);
                    } else if self.search_buf.len() == 1 {
                        self.jump_to(self.search_buf.as_bytes()[0] as usize);
                    }
                    self.search_mode = false;
                    self.search_buf.clear();
                }
                _ => {
                    if line.len() == 1 {
                        let b = line.as_bytes()[0];
                        if b.is_ascii_graphic() || b == b' ' { self.search_buf.push(b as char); }
                    }
                }
            }
            self.draw();
            return;
        }

        match line {
            "\x1b[A" => {
                if self.selected > 0 { self.selected -= 1; }
                if self.selected < self.scroll { self.scroll = self.selected; }
            }
            "\x1b[B" => {
                if self.selected < 127 { self.selected += 1; }
                if self.selected >= self.scroll + ROWS_VISIBLE {
                    self.scroll = self.selected + 1 - ROWS_VISIBLE;
                }
            }
            "+" | "=" => {
                self.scroll = (self.scroll + 16).min(128usize.saturating_sub(ROWS_VISIBLE));
                self.selected = self.selected.max(self.scroll).min(self.scroll + ROWS_VISIBLE - 1).min(127);
            }
            "-" => {
                self.scroll = self.scroll.saturating_sub(16);
                self.selected = self.selected.min(self.scroll + ROWS_VISIBLE - 1);
            }
            "f" | "F" | "/" => { self.search_mode = true; self.search_buf.clear(); }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.draw();
    }
}

fn ascii_name(code: usize) -> &'static str {
    match code {
        0   => "Null character",
        1   => "Start of Heading",
        2   => "Start of Text",
        3   => "End of Text",
        4   => "End of Transmission",
        5   => "Enquiry",
        6   => "Acknowledge",
        7   => "Bell (alert)",
        8   => "Backspace",
        9   => "Horizontal Tab",
        10  => "Line Feed (newline)",
        11  => "Vertical Tab",
        12  => "Form Feed",
        13  => "Carriage Return",
        14  => "Shift Out",
        15  => "Shift In",
        16  => "Data Link Escape",
        17  => "Device Control 1 (XON)",
        18  => "Device Control 2",
        19  => "Device Control 3 (XOFF)",
        20  => "Device Control 4",
        21  => "Negative Acknowledge",
        22  => "Synchronous Idle",
        23  => "End of Transmission Block",
        24  => "Cancel",
        25  => "End of Medium",
        26  => "Substitute",
        27  => "Escape",
        28  => "File Separator",
        29  => "Group Separator",
        30  => "Record Separator",
        31  => "Unit Separator",
        32  => "Space",
        33  => "Exclamation mark",
        34  => "Double quote",
        35  => "Hash / Number sign",
        36  => "Dollar sign",
        37  => "Percent",
        38  => "Ampersand",
        39  => "Single quote / Apostrophe",
        40  => "Left parenthesis",
        41  => "Right parenthesis",
        42  => "Asterisk",
        43  => "Plus sign",
        44  => "Comma",
        45  => "Hyphen-minus",
        46  => "Full stop / Period",
        47  => "Forward slash",
        48..=57  => "Digit",
        58  => "Colon",
        59  => "Semicolon",
        60  => "Less-than sign",
        61  => "Equals sign",
        62  => "Greater-than sign",
        63  => "Question mark",
        64  => "At sign",
        65..=90  => "Uppercase Latin letter",
        91  => "Left square bracket",
        92  => "Backslash",
        93  => "Right square bracket",
        94  => "Caret / Circumflex",
        95  => "Underscore",
        96  => "Grave accent / Backtick",
        97..=122 => "Lowercase Latin letter",
        123 => "Left curly brace",
        124 => "Vertical bar / Pipe",
        125 => "Right curly brace",
        126 => "Tilde",
        127 => "Delete (DEL)",
        _   => "",
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
