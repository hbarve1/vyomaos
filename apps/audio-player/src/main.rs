use std::io::{self, BufRead, Write};
use std::time::{Duration, Instant};

const W: u32 = 440;
const H: u32 = 260;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x1F4068FF;
const C_OK: u32     = 0x3FB950FF;
const C_HINT: u32   = 0x6E7681FF;
const C_PROG: u32   = 0x58A6FFFF;

const BAR_X: u32 = 20;
const BAR_Y: u32 = 190;
const BAR_W: u32 = W - 40;
const BAR_H: u32 = 10;

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

fn list_tracks() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/data") else { return Vec::new() };
    let mut tracks: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.ends_with(".raw"))
        .collect();
    tracks.sort();
    tracks
}

struct State {
    tracks:   Vec<String>,
    current:  usize,
    playing:  bool,
    progress: u32,  // 0..BAR_W
    last_tick: Instant,
}

impl State {
    fn new() -> Self {
        let tracks = list_tracks();
        Self {
            tracks,
            current:  0,
            playing:  false,
            progress: 0,
            last_tick: Instant::now(),
        }
    }
    fn track_name(&self) -> &str {
        self.tracks.get(self.current).map(|s| s.as_str()).unwrap_or("(no .raw files in /data)")
    }
    fn tick(&mut self) -> bool {
        if self.playing && self.last_tick.elapsed() >= Duration::from_secs(1) {
            self.progress = (self.progress + 1).min(BAR_W);
            if self.progress >= BAR_W {
                self.next_track();
            }
            self.last_tick = Instant::now();
            return true;
        }
        false
    }
    fn next_track(&mut self) {
        if !self.tracks.is_empty() {
            self.current = (self.current + 1) % self.tracks.len();
        }
        self.progress = 0;
        self.last_tick = Instant::now();
    }
    fn prev_track(&mut self) {
        if !self.tracks.is_empty() {
            self.current = if self.current == 0 { self.tracks.len() - 1 } else { self.current - 1 };
        }
        self.progress = 0;
        self.last_tick = Instant::now();
    }
}

fn draw(s: &State) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Audio Player  (stub)");
    fill(0, 34, W, 1, C_BORDER);

    // Track list (up to 5 visible)
    let start = if s.current >= 4 { s.current - 3 } else { 0 };
    for (i, name) in s.tracks.iter().skip(start).take(5).enumerate() {
        let abs = start + i;
        let ty = 44 + i as u32 * 18;
        let tc = if abs == s.current { C_TITLE } else { C_DIM };
        let marker = if abs == s.current {
            if s.playing { "▶ " } else { "‖ " }
        } else {
            "  "
        };
        let display = if name.len() > 48 { &name[..48] } else { name.as_str() };
        text(20, ty, tc, &format!("{marker}{display}"));
    }
    if s.tracks.is_empty() {
        text(20, 52, C_DIM, "No .raw files in /data");
    }

    // Controls bar
    fill(0, 150, W, 1, C_BORDER);
    let play_label = if s.playing { "  [◀◀]  [‖]  [▶▶]  " } else { "  [◀◀]  [▶]  [▶▶]  " };
    text(W / 2 - 80, 160, C_TITLE, play_label);

    // Progress bar
    fill(BAR_X, BAR_Y, BAR_W, BAR_H, 0x21262DFF);
    border(BAR_X, BAR_Y, BAR_W, BAR_H, C_BORDER);
    if s.progress > 0 {
        fill(BAR_X, BAR_Y, s.progress, BAR_H, C_PROG);
    }

    // Time cosmetic
    let elapsed = s.progress;
    let total = BAR_W;
    text(BAR_X, BAR_Y + BAR_H + 6, C_DIM, &format!("{elapsed}s / {total}s"));

    text(20, H - 18, C_HINT, "Space: play/pause   ←→: track   ↑↓: browse   Ctrl+C: quit");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut s = State::new();
    draw(&s);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") {
            // Advance progress on any reply if playing
            if s.tick() { draw(&s); }
            continue;
        }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            " " => {
                s.playing = !s.playing;
                s.last_tick = Instant::now();
                draw(&s);
            }
            "\x1b[C" => {
                s.next_track();
                draw(&s);
            }
            "\x1b[D" => {
                s.prev_track();
                draw(&s);
            }
            "\x1b[A" => {
                if s.current > 0 { s.current -= 1; }
                draw(&s);
            }
            "\x1b[B" => {
                if s.current + 1 < s.tracks.len() { s.current += 1; }
                draw(&s);
            }
            _ => {
                // Advance progress on any keypress if playing
                if s.tick() { draw(&s); }
            }
        }
    }
}
