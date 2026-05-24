// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;

const STEPS: usize = 16;
const NOTES: usize = 12; // one octave: C D D# E F F# G G# A A# B B#=C5

const ROW_LABEL_W: u32 = 56;
const STEP_LABEL_H: u32 = 20;
const CELL_W: u32 = (W - ROW_LABEL_W - 8) / STEPS as u32; // ~71
const CELL_H: u32 = 36;

const GRID_X: u32 = ROW_LABEL_W;
const GRID_Y: u32 = HEADER_H + STEP_LABEL_H;

const NOTE_NAMES: [&str; NOTES] = ["C4","C#4","D4","D#4","E4","F4","F#4","G4","G#4","A4","A#4","B4"];

// Note colors (spectrum)
const NOTE_COLORS: [u32; NOTES] = [
    0xFF4444FF, 0xFF6644FF, 0xFF8844FF, 0xFFAA44FF,
    0xFFDD44FF, 0x88CC44FF, 0x44CC88FF, 0x44CCCCFF,
    0x4488FFFF, 0x6644FFFF, 0xAA44FFFF, 0xFF44AAFF,
];

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD: u32    = 0x161B22FF;
const C_ACTIVE: u32  = 0x1C2D4EFF; // active step column bg

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

struct Composer {
    grid:      [[bool; STEPS]; NOTES], // [note][step]
    cursor_n:  usize, // note row (0=top=B4, 11=bottom=C4)
    cursor_s:  usize, // step column
    play_step: usize,
    playing:   bool,
    bpm:       u32,
    tick:      u64,
    ticks_per_step: u64,
}

impl Composer {
    fn new() -> Self {
        let mut c = Composer {
            grid: [[false; STEPS]; NOTES],
            cursor_n: 0,
            cursor_s: 0,
            play_step: 0,
            playing: false,
            bpm: 120,
            tick: 0,
            ticks_per_step: 8,
        };
        // Default pattern: simple melody
        c.grid[0][0] = true;  // B4 step 0
        c.grid[4][2] = true;  // E4 step 2
        c.grid[7][4] = true;  // G4 step 4
        c.grid[0][6] = true;  // B4 step 6
        c.grid[9][8] = true;  // A4 step 8
        c.grid[4][10] = true; // E4 step 10
        c.grid[7][12] = true; // G4 step 12
        c.grid[0][14] = true; // B4 step 14
        c.update_tps();
        c
    }

    fn update_tps(&mut self) {
        // ticks_per_step: lower BPM = more ticks between steps
        // We use ping-pong, each tick = 1 input event. Approximate: step every (120/bpm * 8) ticks
        self.ticks_per_step = (120 * 8 / self.bpm).max(1) as u64;
    }

    fn toggle(&mut self) {
        let n = self.cursor_n;
        let s = self.cursor_s;
        self.grid[n][s] = !self.grid[n][s];
    }

    fn save(&self) -> Result<(), String> {
        let mut out = format!("bpm={}\n", self.bpm);
        for n in 0..NOTES {
            let row: String = self.grid[n].iter().map(|&b| if b { '1' } else { '0' }).collect();
            out.push_str(&row);
            out.push('\n');
        }
        std::fs::write("/data/composer.txt", out).map_err(|e| e.to_string())
    }

    fn load(&mut self) -> Result<(), String> {
        let s = std::fs::read_to_string("/data/composer.txt").map_err(|e| e.to_string())?;
        let mut lines = s.lines();
        if let Some(first) = lines.next() {
            if let Some(rest) = first.strip_prefix("bpm=") {
                if let Ok(b) = rest.parse::<u32>() {
                    self.bpm = b.clamp(40, 200);
                    self.update_tps();
                }
            }
        }
        for (n, line) in lines.enumerate().take(NOTES) {
            for (s, ch) in line.chars().enumerate().take(STEPS) {
                self.grid[n][s] = ch == '1';
            }
        }
        Ok(())
    }

    fn clear(&mut self) {
        self.grid = [[false; STEPS]; NOTES];
        self.play_step = 0;
    }
}

fn draw(c: &Composer, status: &str) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Music Composer");
    let play_str = if c.playing { "■ PLAYING" } else { "◆ STOPPED" };
    let play_col = if c.playing { C_GREEN } else { C_HINT };
    text(220, 16, play_col, play_str);
    text(380, 16, C_HINT, &format!("BPM: {}", c.bpm));
    text(480, 16, C_HINT, "Space:toggle  ←→↑↓:move  P:play  R:clear  +/-:BPM  Ctrl+W:save  Ctrl+L:load");

    // Step number labels
    for s in 0..STEPS {
        let sx = GRID_X + s as u32 * CELL_W;
        let label = format!("{:2}", s + 1);
        let col = if c.playing && s == c.play_step { C_SEL } else { C_HINT };
        text(sx + (CELL_W - 16) / 2, HEADER_H + 4, col, &label);
    }

    // Grid
    for n in 0..NOTES {
        let ny = GRID_Y + n as u32 * CELL_H;
        let display_n = NOTES - 1 - n; // invert so C4 at bottom

        // Row label
        let label_col = if display_n == c.cursor_n { C_TEXT } else { C_HINT };
        fill(0, ny, ROW_LABEL_W - 4, CELL_H - 1, C_CARD);
        text(4, ny + (CELL_H - 14) / 2, label_col, NOTE_NAMES[display_n]);

        for s in 0..STEPS {
            let sx = GRID_X + s as u32 * CELL_W;
            let on = c.grid[display_n][s];
            let is_cursor = display_n == c.cursor_n && s == c.cursor_s;
            let is_active_step = c.playing && s == c.play_step;

            let cell_bg = if is_active_step { C_ACTIVE } else { C_BG };
            fill(sx, ny, CELL_W - 1, CELL_H - 1, cell_bg);

            if on {
                let note_col = NOTE_COLORS[display_n];
                let alpha_col = if is_active_step {
                    // Brighten active notes
                    let r = ((note_col >> 24) & 0xFF).min(255);
                    let g = ((note_col >> 16) & 0xFF).min(255);
                    let b = ((note_col >> 8) & 0xFF).min(255);
                    (r << 24) | (g << 16) | (b << 8) | 0xFF
                } else {
                    // Dim inactive notes slightly
                    let r = ((note_col >> 24) & 0xFF) * 3 / 4;
                    let g = ((note_col >> 16) & 0xFF) * 3 / 4;
                    let b = ((note_col >> 8) & 0xFF) * 3 / 4;
                    (r << 24) | (g << 16) | (b << 8) | 0xFF
                };
                fill(sx + 2, ny + 2, CELL_W - 5, CELL_H - 5, alpha_col);
            }

            if is_cursor {
                border(sx, ny, CELL_W - 1, CELL_H - 1, C_SEL);
            }
        }
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let note_count = (0..NOTES).map(|n| c.grid[n].iter().filter(|&&b| b).count()).sum::<usize>();
    let base = format!("{}  |  {} notes placed  |  step {}/{}",
        if status.is_empty() { "Piano Roll — 12 notes × 16 steps".to_string() } else { status.to_string() },
        note_count, c.play_step + 1, STEPS);
    text(8, sb_y + 6, C_HINT, &base);
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut c = Composer::new();
    let mut status = String::new();

    println!("@supervisor: raise music-composer");
    let _ = io::stdout().flush();

    // Start ping-pong for playback
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&c, &status);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            c.tick += 1;
            if c.playing && c.tick % c.ticks_per_step == 0 {
                c.play_step = (c.play_step + 1) % STEPS;
            }
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&c, &status);
            continue;
        }

        status.clear();
        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            // Up = higher pitch (visually upward = larger note index since B4 is at top)
            "\x1b[A" => { if c.cursor_n + 1 < NOTES { c.cursor_n += 1; } }
            "\x1b[B" => { c.cursor_n = c.cursor_n.saturating_sub(1); }
            "\x1b[C" => { if c.cursor_s + 1 < STEPS { c.cursor_s += 1; } }
            "\x1b[D" => { c.cursor_s = c.cursor_s.saturating_sub(1); }
            " " | "" => { c.toggle(); }
            "p" | "P" => {
                c.playing = !c.playing;
                if c.playing { c.play_step = 0; }
            }
            "r" | "R" => { c.clear(); status = "Cleared".to_string(); }
            "+" | "=" => {
                c.bpm = (c.bpm + 5).min(200);
                c.update_tps();
                status = format!("BPM: {}", c.bpm);
            }
            "-" => {
                c.bpm = c.bpm.saturating_sub(5).max(40);
                c.update_tps();
                status = format!("BPM: {}", c.bpm);
            }
            "\x17" => { // Ctrl+W
                match c.save() {
                    Ok(())  => status = "Saved to /data/composer.txt".to_string(),
                    Err(e)  => status = format!("Save error: {}", e),
                }
            }
            "\x0c" => { // Ctrl+L
                match c.load() {
                    Ok(())  => status = "Loaded from /data/composer.txt".to_string(),
                    Err(e)  => status = format!("Load error: {}", e),
                }
            }
            _ => {}
        }
        draw(&c, &status);
    }
}
