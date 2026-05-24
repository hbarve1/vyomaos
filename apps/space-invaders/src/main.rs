// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 820;
const HEADER_H: u32 = 40;
const ALIEN_W: u32 = 40;
const ALIEN_H: u32 = 24;
const ALIEN_GAP: u32 = 8;
const ALIEN_ROWS: usize = 3;
const ALIEN_COLS: usize = 10;
const ALIEN_X0: i32 = 40;
const ALIEN_Y0: i32 = 80;
const ALIEN_COL_STEP: i32 = (ALIEN_W + ALIEN_GAP) as i32; // 48
const ALIEN_ROW_STEP: i32 = (ALIEN_H + ALIEN_GAP) as i32; // 32
const PLAYER_W: u32 = 40;
const PLAYER_H: u32 = 20;
const PLAYER_Y: u32 = 740;
const BULLET_W: u32 = 4;
const BULLET_H: u32 = 8;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_RED: u32    = 0xFF7B72FF;

const ALIEN_COLORS: [u32; ALIEN_ROWS] = [C_RED, C_ORANGE, C_GREEN];
const ALIEN_CHARS: [&str; ALIEN_ROWS] = ["^", "*", "@"];

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

#[derive(PartialEq, Clone, Copy)]
enum State { Waiting, Playing, Won, Lost }

struct Game {
    aliens:       [[bool; ALIEN_COLS]; ALIEN_ROWS],
    alien_count:  u32,
    off_x:        i32,
    off_y:        i32,
    dir:          i32,
    player_x:     i32,
    pbullet:      Option<(i32, i32)>,     // player bullet (x, y)
    abullets:     [(i32, i32, bool); 3],  // alien bullets
    score:        u32,
    state:        State,
    tick:         u32,
    rng:          u64,
}

impl Game {
    fn new() -> Self {
        Game {
            aliens:      [[true; ALIEN_COLS]; ALIEN_ROWS],
            alien_count: (ALIEN_ROWS * ALIEN_COLS) as u32,
            off_x: 0, off_y: 0, dir: 1,
            player_x: (W as i32 - PLAYER_W as i32) / 2,
            pbullet: None,
            abullets: [(0, 0, false); 3],
            score: 0,
            state: State::Waiting,
            tick: 0,
            rng: 0xACE1FACE12345678,
        }
    }

    fn lcg(&mut self) -> u64 {
        self.rng = self.rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.rng >> 33
    }

    fn alien_right_edge(&self) -> i32 {
        ALIEN_X0 + self.off_x + (ALIEN_COLS as i32 - 1) * ALIEN_COL_STEP + ALIEN_W as i32
    }

    fn alien_bottom_edge(&self) -> i32 {
        ALIEN_Y0 + self.off_y + (ALIEN_ROWS as i32 - 1) * ALIEN_ROW_STEP + ALIEN_H as i32
    }

    fn try_fire_alien(&mut self) {
        // Pick a random column with at least one alive alien
        let col = (self.lcg() as usize) % ALIEN_COLS;
        // Find bottom-most alive alien in that column
        let mut shooter: Option<(usize, usize)> = None;
        for row in (0..ALIEN_ROWS).rev() {
            if self.aliens[row][col] { shooter = Some((row, col)); break; }
        }
        if let Some((row, col)) = shooter {
            // Find inactive bullet slot
            for slot in &mut self.abullets {
                if !slot.2 {
                    let ax = ALIEN_X0 + self.off_x + col as i32 * ALIEN_COL_STEP + ALIEN_W as i32 / 2;
                    let ay = ALIEN_Y0 + self.off_y + row as i32 * ALIEN_ROW_STEP + ALIEN_H as i32;
                    *slot = (ax, ay, true);
                    break;
                }
            }
        }
    }

    fn step(&mut self) {
        if self.state != State::Playing { return; }
        self.tick += 1;

        // Move player bullet up
        if let Some((bx, by)) = self.pbullet {
            let ny = by - 12;
            if ny + BULLET_H as i32 < HEADER_H as i32 {
                self.pbullet = None;
            } else {
                // Check alien hits
                let mut hit = false;
                'outer: for row in 0..ALIEN_ROWS {
                    for col in 0..ALIEN_COLS {
                        if !self.aliens[row][col] { continue; }
                        let ax = ALIEN_X0 + self.off_x + col as i32 * ALIEN_COL_STEP;
                        let ay = ALIEN_Y0 + self.off_y + row as i32 * ALIEN_ROW_STEP;
                        if bx + BULLET_W as i32 > ax && bx < ax + ALIEN_W as i32
                            && ny + BULLET_H as i32 > ay && ny < ay + ALIEN_H as i32 {
                            self.aliens[row][col] = false;
                            self.alien_count -= 1;
                            self.score += 10 + (ALIEN_ROWS - 1 - row) as u32 * 5;
                            hit = true;
                            break 'outer;
                        }
                    }
                }
                if hit { self.pbullet = None; } else { self.pbullet = Some((bx, ny)); }
                if self.alien_count == 0 { self.state = State::Won; return; }
            }
        }

        // Move alien bullets down
        for slot in &mut self.abullets {
            if !slot.2 { continue; }
            slot.1 += 6;
            if slot.1 > H as i32 { slot.2 = false; continue; }
            // Check player hit
            let bx = slot.0; let by = slot.1;
            let px = self.player_x;
            if bx + BULLET_W as i32 > px && bx < px + PLAYER_W as i32
                && by + BULLET_H as i32 > PLAYER_Y as i32 && by < (PLAYER_Y + PLAYER_H) as i32 {
                self.state = State::Lost;
                return;
            }
        }

        // March aliens every 3 ticks
        if self.tick % 3 == 0 {
            self.off_x += self.dir * 2;
            let right = self.alien_right_edge();
            let left = ALIEN_X0 + self.off_x;
            if right > W as i32 - 20 || left < 20 {
                self.dir = -self.dir;
                self.off_x += self.dir * 2; // undo and reverse
                self.off_y += 20;
                if self.alien_bottom_edge() > PLAYER_Y as i32 {
                    self.state = State::Lost;
                    return;
                }
            }
        }

        // Alien shooting every 15 ticks
        if self.tick % 15 == 0 {
            self.try_fire_alien();
        }
    }

    fn fire_player(&mut self) {
        if self.pbullet.is_none() {
            let bx = self.player_x + PLAYER_W as i32 / 2 - BULLET_W as i32 / 2;
            let by = PLAYER_Y as i32 - BULLET_H as i32;
            self.pbullet = Some((bx, by));
        }
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_TEXT, "Space Invaders");
    text(200, 12, C_ORANGE, &format!("Score: {}  Aliens: {}", g.score, g.alien_count));
    text(500, 12, C_HINT, "Arrows: move  Space: fire  R: restart");

    // Ground line
    fill(0, PLAYER_Y + PLAYER_H, W, 2, C_BORDER);

    // Aliens
    for row in 0..ALIEN_ROWS {
        for col in 0..ALIEN_COLS {
            if !g.aliens[row][col] { continue; }
            let ax = (ALIEN_X0 + g.off_x + col as i32 * ALIEN_COL_STEP).max(0) as u32;
            let ay = (ALIEN_Y0 + g.off_y + row as i32 * ALIEN_ROW_STEP).max(0) as u32;
            fill(ax, ay, ALIEN_W, ALIEN_H, ALIEN_COLORS[row]);
            fill(ax, ay, ALIEN_W, 2, 0xFFFFFF30);
            text(ax + 14, ay + 4, C_TEXT, ALIEN_CHARS[row]);
        }
    }

    // Player ship
    fill(g.player_x.max(0) as u32, PLAYER_Y, PLAYER_W, PLAYER_H, C_SEL);
    fill(g.player_x.max(0) as u32 + 14, PLAYER_Y.saturating_sub(8), 12, 8, C_SEL);

    // Player bullet
    if let Some((bx, by)) = g.pbullet {
        fill(bx.max(0) as u32, by.max(0) as u32, BULLET_W, BULLET_H, C_GREEN);
    }

    // Alien bullets
    for &(bx, by, active) in &g.abullets {
        if active {
            fill(bx.max(0) as u32, by.max(0) as u32, BULLET_W, BULLET_H, C_RED);
        }
    }

    match g.state {
        State::Waiting => {
            text((W - 152) / 2, H / 2, C_HINT, "Space to start");
        }
        State::Won => {
            let bx = (W - 200) / 2;
            let by = (H - 64) / 2;
            fill(bx, by, 200, 64, C_HEADER);
            border(bx, by, 200, 64, C_GREEN);
            text(bx + 44, by + 10, C_GREEN, "You Win!");
            text(bx + 20, by + 32, C_HINT, &format!("Score: {}  R=restart", g.score));
        }
        State::Lost => {
            let bx = (W - 200) / 2;
            let by = (H - 64) / 2;
            fill(bx, by, 200, 64, C_HEADER);
            border(bx, by, 200, 64, C_RED);
            text(bx + 32, by + 10, C_RED, "Game Over!");
            text(bx + 20, by + 32, C_HINT, &format!("Score: {}  R=restart", g.score));
        }
        _ => {}
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise space-invaders");
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            if game.state == State::Playing {
                game.step();
                draw(&game);
            }
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[D" => {
                game.player_x = (game.player_x - 8).max(0);
            }
            "\x1b[C" => {
                game.player_x = (game.player_x + 8).min(W as i32 - PLAYER_W as i32);
            }
            " " | "" | "\r" => {
                match game.state {
                    State::Waiting => { game.state = State::Playing; }
                    State::Playing => { game.fire_player(); }
                    _ => {}
                }
            }
            "r" | "R" => { game = Game::new(); }
            _ => {}
        }
        draw(&game);
    }
}
