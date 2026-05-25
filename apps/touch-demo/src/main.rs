// touch-demo — TM08
//
// Renders colored circles at tap positions.
// On swipe, shifts a scroll_offset and redraws the canvas.
// Requires: display = true, touch = true in vyoma.toml

use std::io::{self, BufRead};

// ── Color constants ───────────────────────────────────────────────────────────

const BG:      u32 = 0x1E1E2EFF;
const WHITE:   u32 = 0xFFFFFFFF;
const CYAN:    u32 = 0x89DCEBff;
const MAGENTA: u32 = 0xCBA6F7FF;
const YELLOW:  u32 = 0xF9E2AFFF;
const GREEN:   u32 = 0xA6E3A1FF;

// Canvas dimensions (portrait mobile).
const CANVAS_W: u32 = 1080;
const CANVAS_H: u32 = 500;

// Maximum tap circles to remember (so we can redraw on scroll).
const MAX_TAPS: usize = 64;

// ── State ─────────────────────────────────────────────────────────────────────

struct State {
    taps:          Vec<(u32, u32)>,
    scroll_offset: i32,
    tap_count:     u32,
    swipe_count:   u32,
}

impl State {
    fn new() -> Self {
        Self {
            taps: Vec::new(),
            scroll_offset: 0,
            tap_count: 0,
            swipe_count: 0,
        }
    }

    fn add_tap(&mut self, x: u32, y: u32) {
        if self.taps.len() >= MAX_TAPS {
            self.taps.remove(0);
        }
        self.taps.push((x, y));
        self.tap_count += 1;
    }

    fn apply_swipe(&mut self, dy: i32) {
        self.scroll_offset += dy;
        self.swipe_count += 1;
    }
}

// ── Drawing ───────────────────────────────────────────────────────────────────

/// Draw the full frame: background, status text, all tap circles.
fn draw_frame(s: &State) {
    println!("VYOMA_DRAW:fill_rect:0,0,{CANVAS_W},{CANVAS_H},{BG}");
    println!("VYOMA_DRAW:draw_text:8,8,{WHITE},m,touch-demo");
    println!(
        "VYOMA_DRAW:draw_text:8,30,{CYAN},s,taps: {}  swipes: {}  scroll: {}",
        s.tap_count, s.swipe_count, s.scroll_offset
    );

    // Draw a colored circle (20×20 square) at each tap position,
    // adjusted by the current scroll offset.
    let colors = [CYAN, MAGENTA, YELLOW, GREEN];
    for (i, &(tx, ty)) in s.taps.iter().enumerate() {
        let color = colors[i % colors.len()];
        let draw_y = ty as i32 + s.scroll_offset;
        // Only draw if visible within the canvas.
        if draw_y >= 0 && draw_y + 20 <= CANVAS_H as i32 {
            let cx = tx.saturating_sub(10);
            println!("VYOMA_DRAW:fill_rect:{cx},{draw_y},20,20,{color}");
        }
    }

    println!("VYOMA_DRAW:flush");
}

// ── Touch event parsing ───────────────────────────────────────────────────────

/// Parse `VYOMA_INPUT:touch:tap:<x>,<y>` → `Some((x, y))`.
fn parse_tap(line: &str) -> Option<(u32, u32)> {
    let coords = line.strip_prefix("VYOMA_INPUT:touch:tap:")?;
    let (xs, ys) = coords.split_once(',')?;
    let x = xs.parse::<u32>().ok()?;
    let y = ys.parse::<u32>().ok()?;
    Some((x, y))
}

/// Parse `VYOMA_INPUT:touch:swipe:<dx>,<dy>` → `Some((dx, dy))`.
fn parse_swipe(line: &str) -> Option<(i32, i32)> {
    let deltas = line.strip_prefix("VYOMA_INPUT:touch:swipe:")?;
    let (dxs, dys) = deltas.split_once(',')?;
    let dx = dxs.parse::<i32>().ok()?;
    let dy = dys.parse::<i32>().ok()?;
    Some((dx, dy))
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    eprintln!("touch-demo started");
    let mut state = State::new();
    draw_frame(&state);

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if let Some((x, y)) = parse_tap(&line) {
            state.add_tap(x, y);
            draw_frame(&state);
        } else if let Some((_dx, dy)) = parse_swipe(&line) {
            state.apply_swipe(dy);
            draw_frame(&state);
        }
    }
}
