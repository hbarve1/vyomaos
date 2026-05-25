// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

mod theory;
mod app;

use std::io::{self, BufRead, Write};

const W: i32 = 880;
const H: i32 = 680;

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

const NOTES: [&str; 12] = [
    "C","C#","D","D#","E","F","F#","G","G#","A","A#","B"
];

// Scale intervals (half-steps from root)
const SCALE_NAMES: [&str; 8] = [
    "Major", "Natural Minor", "Dorian", "Phrygian",
    "Lydian", "Mixolydian", "Locrian", "Pentatonic",
];
const SCALE_INTERVALS: [[u8; 7]; 8] = [
    [2, 2, 1, 2, 2, 2, 1], // Major
    [2, 1, 2, 2, 1, 2, 2], // Natural Minor
    [2, 1, 2, 2, 2, 1, 2], // Dorian
    [1, 2, 2, 2, 1, 2, 2], // Phrygian
    [2, 2, 2, 1, 2, 2, 1], // Lydian
    [2, 2, 1, 2, 2, 1, 2], // Mixolydian
    [1, 2, 2, 1, 2, 2, 2], // Locrian
    [2, 2, 3, 2, 3, 0, 0], // Pentatonic (5 notes, padded)
];
const SCALE_LEN: [usize; 8] = [7, 7, 7, 7, 7, 7, 7, 5];

// Chord types: intervals from root
const CHORD_NAMES: [&str; 7] = ["Major", "Minor", "7th", "Maj7", "Dim", "Aug", "Sus4"];
// Each chord: intervals between successive chord tones
const CHORD_INTERVALS: [[u8; 3]; 7] = [
    [4, 3, 5], // Major:   1 3 5
    [3, 4, 5], // Minor:   1 b3 5
    [4, 3, 3], // Dom 7th: 1 3 5 b7
    [4, 3, 4], // Maj7:    1 3 5 7
    [3, 3, 6], // Dim:     1 b3 b5 (bb7)
    [4, 4, 4], // Aug:     1 3 #5
    [5, 2, 5], // Sus4:    1 4 5
];

// Named intervals
const INTERVAL_NAMES: [&str; 13] = [
    "Unison","m2","M2","m3","M3","P4","Tritone","P5","m6","M6","m7","M7","Octave"
];

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

// Precomputed sin/cos * 1000 for 12 clock positions (30-degree steps)
// (sin, cos) * 1000 for 0°,30°,60°,...,330°
static CIRCLE_POS: [(i64, i64); 12] = [
    (0, 1000),    // 0°  C
    (500, 866),   // 30° C#
    (866, 500),   // 60° D
    (1000, 0),    // 90° D#
    (866, -500),  // 120° E
    (500, -866),  // 150° F
    (0, -1000),   // 180° F#
    (-500, -866), // 210° G
    (-866, -500), // 240° G#
    (-1000, 0),   // 270° A
    (-866, 500),  // 300° A#
    (-500, 866),  // 330° B
];

fn main() {
    let stdin = io::stdin();
    let mut app = app::App::new();
    app.draw();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
