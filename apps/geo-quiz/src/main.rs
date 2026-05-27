// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

mod app;

use std::io::{self, BufRead, Write};

const W: i32 = 880;
const H: i32 = 640;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_CARD: u32   = 0x161B22FF;
const C_PURPLE: u32 = 0xBC8CFFFF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c);
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s);
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = app::App::new();
    app.draw();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        // Ignore supervisor system notifications (screen resize etc.)
        if line.starts_with("VYOMA_SYSTEM:") { continue; }
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
