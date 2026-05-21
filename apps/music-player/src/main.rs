use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 560;
const HEADER_H: u32 = 48;

const BAR_COUNT: usize = 32;
const BAR_W: u32 = 16;
const BAR_GAP: u32 = 4;
const VIZ_X: u32 = (W - BAR_COUNT as u32 * (BAR_W + BAR_GAP)) / 2;
const VIZ_Y: u32 = HEADER_H + 16;
const VIZ_H: u32 = 140;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

// Bhaskara I sin approximation, input 0..359 degrees, output 0..256
fn sin_approx(deg: i64) -> i64 {
    let d = ((deg % 360) + 360) % 360;
    if d > 180 { return -sin_approx(d - 180); }
    let n = d * (180 - d);
    (4 * n * 256) / (40500 + n)
}

const FALLBACK_TRACKS: [&str; 5] = [
    "Midnight Horizon.raw",
    "Digital Rain.raw",
    "Neon Pulse.raw",
    "Crystal Cave.raw",
    "Solar Wind.raw",
];

const FALLBACK_DURATIONS: [u32; 5] = [214, 187, 263, 195, 241];

struct Player {
    tracks:    Vec<String>,
    durations: Vec<u32>,   // seconds
    current:   usize,
    playing:   bool,
    progress:  u32,        // ticks elapsed (each tick ~1/8 sec)
    volume:    u32,        // 0..100
    phase:     u32,        // for waveform animation
    bars:      [u32; BAR_COUNT],
    seed:      u64,
}

impl Player {
    fn new() -> Self {
        let mut tracks: Vec<String> = Vec::new();
        let mut durations: Vec<u32> = Vec::new();

        if let Ok(entries) = std::fs::read_dir("/data") {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".raw") {
                    let dur = std::fs::metadata(entry.path())
                        .map(|m| (m.len() / 88200) as u32)
                        .unwrap_or(180);
                    tracks.push(name);
                    durations.push(dur.max(10));
                }
            }
        }

        if tracks.is_empty() {
            for t in FALLBACK_TRACKS { tracks.push(t.to_string()); }
            for d in FALLBACK_DURATIONS { durations.push(d); }
        }

        Player {
            tracks,
            durations,
            current: 0,
            playing: false,
            progress: 0,
            volume: 80,
            phase: 0,
            bars: [0; BAR_COUNT],
            seed: 0xABCD1234EF567890,
        }
    }

    fn duration_ticks(&self) -> u32 {
        self.durations[self.current] * 8
    }

    fn progress_secs(&self) -> u32 {
        self.progress / 8
    }

    fn next_track(&mut self) {
        self.current = (self.current + 1) % self.tracks.len();
        self.progress = 0;
    }

    fn prev_track(&mut self) {
        if self.progress_secs() > 3 {
            self.progress = 0;
        } else {
            self.current = if self.current == 0 { self.tracks.len() - 1 } else { self.current - 1 };
            self.progress = 0;
        }
    }

    fn tick(&mut self) {
        if !self.playing { return; }
        self.progress += 1;
        self.phase = self.phase.wrapping_add(4);

        // Animate bars
        self.seed = lcg(self.seed);
        for i in 0..BAR_COUNT {
            let deg = (self.phase as i64 + i as i64 * 11) % 360;
            let s = sin_approx(deg).abs();
            self.seed = lcg(self.seed);
            let noise = (self.seed >> 56) as i64 & 0x1F;
            let amp = ((s + noise) as u32 * VIZ_H / 300).min(VIZ_H - 4);
            // Smooth toward target
            if amp > self.bars[i] { self.bars[i] = (self.bars[i] + 4).min(amp); }
            else { self.bars[i] = self.bars[i].saturating_sub(3).max(amp); }
            self.bars[i] = self.bars[i].clamp(4, VIZ_H - 4);
        }

        if self.progress >= self.duration_ticks() {
            self.next_track();
        }
    }
}

fn hue_color(i: usize) -> u32 {
    // Simple hue spectrum across 32 bars
    let h = i * 360 / BAR_COUNT;
    let s = 220u32;
    let v = 200u32;
    let hi = (h / 60) as u32;
    let f = (h % 60) as u32;
    let p = v * (255 - s) / 255;
    let q = v * (255 - s * f / 60) / 255;
    let t = v * (255 - s * (60 - f) / 60) / 255;
    let (r, g, b) = match hi {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

fn fmt_time(secs: u32) -> String {
    format!("{:02}:{:02}", secs / 60, secs % 60)
}

fn draw(p: &Player) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Music Player");
    text(180, 16, C_HINT, &format!("Vol: {}%", p.volume));
    text(320, 16, C_HINT, "Space:play/pause  N:next  P:prev  +/-:vol");

    // Waveform visualizer
    fill(0, VIZ_Y - 4, W, VIZ_H + 8, C_CARD);
    for i in 0..BAR_COUNT {
        let bx = VIZ_X + i as u32 * (BAR_W + BAR_GAP);
        let bh = p.bars[i].max(4);
        let by = VIZ_Y + VIZ_H - bh;
        let color = hue_color(i);
        fill(bx, by, BAR_W, bh, color);
    }

    // Track list
    let list_y = VIZ_Y + VIZ_H + 24;
    let track_count = p.tracks.len().min(5);
    for i in 0..track_count {
        let ty = list_y + i as u32 * 28;
        let is_current = i == p.current;
        if is_current {
            fill(20, ty - 4, W - 40, 26, C_CARD);
            border(20, ty - 4, W - 40, 26, C_SEL);
        }
        let num = format!("{:2}.", i + 1);
        let tc = if is_current { C_SEL } else { C_HINT };
        text(32, ty + 4, tc, &num);
        let name = p.tracks[i].trim_end_matches(".raw");
        text(64, ty + 4, if is_current { C_TEXT } else { C_HINT }, name);
        let dur = fmt_time(p.durations[i]);
        text(W - 80, ty + 4, C_HINT, &dur);
    }

    // Current track info + progress bar
    let info_y = H - 80;
    let track_name = p.tracks[p.current].trim_end_matches(".raw");
    let play_icon = if p.playing { "▶" } else { "⏸" };
    text(40, info_y, C_TEXT, &format!("{} {}", play_icon, track_name));

    let elapsed = fmt_time(p.progress_secs());
    let total = fmt_time(p.durations[p.current]);
    text(40, info_y + 22, C_HINT, &format!("{} / {}", elapsed, total));

    // Progress bar
    let bar_x = 40u32;
    let bar_w = W - 80;
    let bar_y = info_y + 48;
    fill(bar_x, bar_y, bar_w, 8, C_CARD);
    let prog = if p.duration_ticks() > 0 { bar_w * p.progress / p.duration_ticks() } else { 0 };
    fill(bar_x, bar_y, prog.min(bar_w), 8, C_SEL);
    border(bar_x, bar_y, bar_w, 8, C_BORDER);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut player = Player::new();

    println!("@supervisor: raise music-player");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&player);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") {
            player.tick();
            if player.playing {
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            draw(&player);
            continue;
        }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            " " => {
                player.playing = !player.playing;
                if player.playing {
                    println!("@supervisor: ping");
                    let _ = io::stdout().flush();
                }
            }
            "n" | "N" => { player.next_track(); }
            "p" | "P" => { player.prev_track(); }
            "+" | "=" => { player.volume = (player.volume + 5).min(100); }
            "-" => { player.volume = player.volume.saturating_sub(5); }
            _ => {}
        }
        draw(&player);
    }
}
