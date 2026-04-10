//! VyomaOS ticker — live heartbeat indicator
//!
//! Draws a small status bar in the top-right corner that updates every second.
//! Proves the GUI pipeline (WASM → supervisor → framebuffer) is alive.
//!
//! Display: top-right corner, 280×36 px, shows uptime seconds + a rotating
//! spinner so it's obvious at a glance whether rendering is running.

use std::{io::Write, thread, time::Duration};

const X: u32 = 980;  // right side of 1280px screen
const Y: u32 = 8;    // just below top edge
const W: u32 = 292;
const H: u32 = 36;

const C_BG:     u32 = 0x21262DFF;
const C_BORDER: u32 = 0x58A6FFFF;
const C_TEXT:   u32 = 0xFFFFFFFF;
const C_DIM:    u32 = 0x8B949EFF;
const C_GREEN:  u32 = 0x3FB950FF;

const SPINNER: [&str; 4] = ["|", "/", "-", "\\"];

fn main() {
    let mut tick: u64 = 0;
    loop {
        let spinner = SPINNER[(tick % 4) as usize];
        let uptime  = format!("up {}s", tick);

        // Background + border
        fill(X, Y, W, H, C_BG);
        fill(X, Y, W, 2, C_BORDER);      // top border
        fill(X, Y + H - 2, W, 2, C_BORDER); // bottom border

        // Spinner dot
        fill(X + 6, Y + 12, 10, 10, C_GREEN);

        // Labels
        text(X + 20, Y + 10, C_TEXT,  spinner);
        text(X + 36, Y + 10, C_DIM,   "ticker");
        text(X + 108, Y + 10, C_TEXT, &uptime);

        flush();

        tick += 1;
        thread::sleep(Duration::from_secs(1));
    }
}

#[inline] fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba}");
}
#[inline] fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba},{s}");
}
#[inline] fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}
