use std::io::{self, BufRead, Write};

const W: u32 = 960;
const H: u32 = 640;
const HEADER_H: u32 = 40;
const STATUS_H: u32 = 28;
const CANVAS_W: usize = 40;
const CANVAS_H: usize = 30;
const CELL: u32 = 6;
const CANVAS_PX_W: u32 = CANVAS_W as u32 * CELL; // 240
const CANVAS_PX_H: u32 = CANVAS_H as u32 * CELL; // 180
const CANVAS_X: u32 = (W - CANVAS_PX_W) / 2;
const CANVAS_Y: u32 = HEADER_H + 20;
const PROG_Y: u32 = CANVAS_Y + CANVAS_PX_H + 12;
const PROG_H: u32 = 12;
const PROG_W: u32 = CANVAS_PX_W;
const INFO_Y: u32 = PROG_Y + PROG_H + 10;
const TOTAL_FRAMES: usize = 60;
const PLAYLIST_LEN: usize = 3;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_PURPLE: u32 = 0xBC8CFFFF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const VIDEO_SEEDS: [u64; PLAYLIST_LEN] = [
    0xA0C5E_12345678ABu64,
    0xB1D6F_23456789BCu64,
    0xC2E7A_3456789CDEu64,
];

const VIDEO_NAMES: [&str; PLAYLIST_LEN] = [
    "Demo Reel #1",
    "System Viz #2",
    "Noise Study #3",
];

// Generate one frame's pixel data using LCG
fn gen_frame(video: usize, frame: usize) -> Vec<u32> {
    let mut s = VIDEO_SEEDS[video];
    // Advance seed by frame number to get distinct frames
    for _ in 0..frame * 7 {
        s = lcg(s);
    }
    let mut pixels = Vec::with_capacity(CANVAS_W * CANVAS_H);
    for _ in 0..CANVAS_W * CANVAS_H {
        s = lcg(s);
        // Different palettes per video
        let rgba = match video {
            0 => {
                // Colorful noise
                let r = ((s >> 48) & 0xFF) as u32;
                let g = ((s >> 32) & 0xFF) as u32;
                let b = ((s >> 16) & 0xFF) as u32;
                (r << 24) | (g << 16) | (b << 8) | 0xFF
            }
            1 => {
                // Blue/cyan tones — "system viz"
                let v = ((s >> 40) & 0xFF) as u32;
                let r = v / 4;
                let g = v / 2;
                let b = v;
                (r << 24) | (g << 16) | (b << 8) | 0xFF
            }
            _ => {
                // Grayscale noise
                let v = ((s >> 40) & 0xFF) as u32;
                (v << 24) | (v << 16) | (v << 8) | 0xFF
            }
        };
        pixels.push(rgba);
    }
    pixels
}

fn draw_frame(pixels: &[u32]) {
    for row in 0..CANVAS_H {
        let mut col = 0usize;
        while col < CANVAS_W {
            let rgba = pixels[row * CANVAS_W + col];
            let mut run = 1usize;
            while col + run < CANVAS_W && pixels[row * CANVAS_W + col + run] == rgba {
                run += 1;
            }
            fill(
                CANVAS_X + col as u32 * CELL,
                CANVAS_Y + row as u32 * CELL,
                run as u32 * CELL,
                CELL,
                rgba,
            );
            col += run;
        }
    }
}

struct Player {
    video:   usize,
    frame:   usize,
    playing: bool,
    speed:   usize, // 1=1x, 2=2x, 4=4x (frames per tick)
    tick:    u64,
}

impl Player {
    fn new() -> Self {
        Player {
            video: 0,
            frame: 0,
            playing: false,
            speed: 1,
            tick: 0,
        }
    }

    fn advance(&mut self) {
        if !self.playing { return; }
        self.frame = (self.frame + self.speed).min(TOTAL_FRAMES - 1);
        if self.frame >= TOTAL_FRAMES - 1 {
            self.playing = false; // end of video
        }
    }
}

fn draw(player: &Player) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Video Player");
    let play_icon = if player.playing { "▶ PLAYING" } else { "⏸ PAUSED" };
    let pc = if player.playing { C_GREEN } else { C_HINT };
    text(160, 12, pc, play_icon);
    text(W - 200, 12, C_HINT, &format!("Speed: {}x", player.speed));

    // Playlist tabs
    let tab_y = HEADER_H + 4;
    for (i, name) in VIDEO_NAMES.iter().enumerate() {
        let tx = CANVAS_X + i as u32 * (CANVAS_PX_W / PLAYLIST_LEN as u32);
        let tc = if i == player.video { C_SEL } else { C_HINT };
        text(tx, tab_y, tc, &format!("[{}] {}", i + 1, name));
    }

    // Canvas border
    border(CANVAS_X - 2, CANVAS_Y - 2, CANVAS_PX_W + 4, CANVAS_PX_H + 4, C_BORDER);

    // Render current frame
    let pixels = gen_frame(player.video, player.frame);
    draw_frame(&pixels);

    // Progress bar
    fill(CANVAS_X, PROG_Y, PROG_W, PROG_H, 0x1A1F27FF);
    border(CANVAS_X, PROG_Y, PROG_W, PROG_H, C_BORDER);
    let progress_w = (player.frame as u32 * PROG_W) / TOTAL_FRAMES as u32;
    if progress_w > 0 {
        fill(CANVAS_X, PROG_Y, progress_w, PROG_H, C_SEL);
    }
    // Playhead
    fill(CANVAS_X + progress_w, PROG_Y, 2, PROG_H, C_ORANGE);

    // Frame counter + time info
    let elapsed = player.frame as f32 / 30.0;
    let total = TOTAL_FRAMES as f32 / 30.0;
    text(CANVAS_X, INFO_Y, C_TEXT,
        &format!("Frame {}/{} · {:.1}s / {:.1}s · {}",
            player.frame + 1, TOTAL_FRAMES, elapsed, total, VIDEO_NAMES[player.video]));

    // Controls help
    let ctrl_y = INFO_Y + 20;
    text(CANVAS_X, ctrl_y, C_HINT,
        "Space=play/pause  N=next  P=prev  R=restart  1/2/3=video  +/-=speed  Ctrl+C=quit");

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let status = if player.playing {
        format!("{} · frame {}/{} · {}x speed", VIDEO_NAMES[player.video], player.frame + 1, TOTAL_FRAMES, player.speed)
    } else if player.frame >= TOTAL_FRAMES - 1 {
        format!("{} · END — press R to restart or 1/2/3 to switch video", VIDEO_NAMES[player.video])
    } else {
        format!("{} · PAUSED at frame {}/{}", VIDEO_NAMES[player.video], player.frame + 1, TOTAL_FRAMES)
    };
    text(8, sb_y + 6, C_HINT, &status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut player = Player::new();

    println!("@supervisor: raise video-player");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&player);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            player.advance();
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&player);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            " " => {
                if player.frame >= TOTAL_FRAMES - 1 {
                    player.frame = 0;
                }
                player.playing = !player.playing;
            }
            "n" | "N" | "\x1b[C" => {
                player.frame = (player.frame + 1).min(TOTAL_FRAMES - 1);
                player.playing = false;
            }
            "p" | "P" | "\x1b[D" => {
                player.frame = player.frame.saturating_sub(1);
                player.playing = false;
            }
            "r" | "R" => {
                player.frame = 0;
                player.playing = false;
            }
            "1" => { player.video = 0; player.frame = 0; player.playing = false; }
            "2" => { player.video = 1; player.frame = 0; player.playing = false; }
            "3" => { player.video = 2; player.frame = 0; player.playing = false; }
            "+" | "=" => {
                player.speed = match player.speed { 1 => 2, 2 => 4, _ => 4 };
            }
            "-" | "_" => {
                player.speed = match player.speed { 4 => 2, 2 => 1, _ => 1 };
            }
            _ => {}
        }
        draw(&player);
    }
}
