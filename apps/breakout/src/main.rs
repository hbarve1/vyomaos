// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 800;
const HEADER_H: u32 = 40;
const PADDLE_W: u32 = 100;
const PADDLE_H: u32 = 12;
const PADDLE_Y: u32 = 720;
const BALL_SZ: u32 = 12;
const BROWS: usize = 5;
const BCOLS: usize = 10;
const BRICK_W: u32 = 68;
const BRICK_H: u32 = 20;
const BRICK_GAP: u32 = 4;
const BRICK_X0: u32 = 20;
const BRICK_Y0: u32 = 60;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_RED: u32    = 0xFF7B72FF;

const BRICK_COLORS: [u32; BROWS] = [
    0xFF7B72FF, // red
    0xFFA657FF, // orange
    0xE3B341FF, // yellow
    0x3FB950FF, // green
    0x58A6FFFF, // blue
];

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
    paddle_x: i32,
    ball_x:   i32,
    ball_y:   i32,
    vel_x:    i32,
    vel_y:    i32,
    bricks:   [[bool; BCOLS]; BROWS],
    bricks_left: u32,
    score:    u32,
    lives:    u32,
    state:    State,
}

impl Game {
    fn new() -> Self {
        let px = (W as i32 - PADDLE_W as i32) / 2;
        Game {
            paddle_x: px,
            ball_x:   px + PADDLE_W as i32 / 2 - BALL_SZ as i32 / 2,
            ball_y:   PADDLE_Y as i32 - BALL_SZ as i32,
            vel_x: 4, vel_y: -4,
            bricks: [[true; BCOLS]; BROWS],
            bricks_left: (BROWS * BCOLS) as u32,
            score: 0, lives: 3,
            state: State::Waiting,
        }
    }

    fn park_ball(&mut self) {
        self.ball_x = self.paddle_x + PADDLE_W as i32 / 2 - BALL_SZ as i32 / 2;
        self.ball_y = PADDLE_Y as i32 - BALL_SZ as i32;
        self.vel_x = 4; self.vel_y = -4;
    }

    fn step(&mut self) {
        if self.state != State::Playing { return; }

        self.ball_x += self.vel_x;
        self.ball_y += self.vel_y;

        // Left / right walls
        if self.ball_x < 0 {
            self.ball_x = 0;
            self.vel_x = self.vel_x.abs();
        }
        if self.ball_x + BALL_SZ as i32 > W as i32 {
            self.ball_x = W as i32 - BALL_SZ as i32;
            self.vel_x = -self.vel_x.abs();
        }
        // Top wall
        if self.ball_y < HEADER_H as i32 {
            self.ball_y = HEADER_H as i32;
            self.vel_y = self.vel_y.abs();
        }

        // Paddle collision (ball moving downward)
        let bbot = self.ball_y + BALL_SZ as i32;
        let brit = self.ball_x + BALL_SZ as i32;
        if self.vel_y > 0
            && bbot >= PADDLE_Y as i32
            && self.ball_y < (PADDLE_Y + PADDLE_H) as i32
            && brit > self.paddle_x
            && self.ball_x < self.paddle_x + PADDLE_W as i32
        {
            self.ball_y = PADDLE_Y as i32 - BALL_SZ as i32;
            let center = self.paddle_x + PADDLE_W as i32 / 2;
            let offset = (self.ball_x + BALL_SZ as i32 / 2) - center;
            self.vel_x = (offset / 12).clamp(-5, 5);
            if self.vel_x == 0 { self.vel_x = if offset >= 0 { 1 } else { -1 }; }
            self.vel_y = -5;
        }

        // Ball below screen — lose life
        if self.ball_y > H as i32 {
            self.lives = self.lives.saturating_sub(1);
            if self.lives == 0 {
                self.state = State::Lost;
            } else {
                self.park_ball();
                self.state = State::Waiting;
            }
            return;
        }

        // Brick collisions (break at most one per tick)
        'bricks: for row in 0..BROWS {
            for col in 0..BCOLS {
                if !self.bricks[row][col] { continue; }
                let bx = BRICK_X0 as i32 + col as i32 * (BRICK_W + BRICK_GAP) as i32;
                let by = BRICK_Y0 as i32 + row as i32 * (BRICK_H + BRICK_GAP) as i32;
                let bx2 = bx + BRICK_W as i32;
                let by2 = by + BRICK_H as i32;

                if brit > bx && self.ball_x < bx2 && bbot > by && self.ball_y < by2 {
                    self.bricks[row][col] = false;
                    self.score += 10;
                    self.bricks_left -= 1;
                    // Reflect off closer face
                    let pen_x = (brit - bx).min(bx2 - self.ball_x);
                    let pen_y = (bbot - by).min(by2 - self.ball_y);
                    if pen_x < pen_y { self.vel_x = -self.vel_x; }
                    else { self.vel_y = -self.vel_y; }
                    if self.bricks_left == 0 { self.state = State::Won; }
                    break 'bricks;
                }
            }
        }
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_TEXT, "Breakout");
    text(120, 12, C_ORANGE, &format!("Score: {}  Lives: {}", g.score, g.lives));
    text(420, 12, C_HINT, "Arrows: paddle  Space: launch  R: restart");

    // Bricks
    for row in 0..BROWS {
        for col in 0..BCOLS {
            if !g.bricks[row][col] { continue; }
            let bx = BRICK_X0 + col as u32 * (BRICK_W + BRICK_GAP);
            let by = BRICK_Y0 + row as u32 * (BRICK_H + BRICK_GAP);
            fill(bx, by, BRICK_W, BRICK_H, BRICK_COLORS[row]);
            // Subtle highlight on top edge
            fill(bx, by, BRICK_W, 2, 0xFFFFFF18);
        }
    }

    // Paddle
    fill(g.paddle_x.max(0) as u32, PADDLE_Y, PADDLE_W, PADDLE_H, C_SEL);
    fill(g.paddle_x.max(0) as u32, PADDLE_Y, PADDLE_W, 2, 0xFFFFFF40);

    // Ball
    fill(g.ball_x.max(0) as u32, g.ball_y.max(0) as u32, BALL_SZ, BALL_SZ, C_TEXT);

    // Life indicators (small rectangles below paddle)
    for i in 0..g.lives.min(5) {
        fill(W - 20 - i * 18, PADDLE_Y + PADDLE_H + 4, 12, 6, C_GREEN);
    }

    // Overlays
    match g.state {
        State::Waiting => {
            let msg = "Press Space to launch the ball";
            let mw = msg.len() as u32 * 8;
            text((W - mw) / 2, H - 24, C_HINT, msg);
        }
        State::Won => {
            let bx = (W - 240) / 2;
            let by = (H - 64) / 2;
            fill(bx, by, 240, 64, C_HEADER);
            border(bx, by, 240, 64, C_GREEN);
            text(bx + 60, by + 12, C_GREEN, "You Win!");
            text(bx + 20, by + 34, C_HINT, &format!("Score: {}   R to restart", g.score));
        }
        State::Lost => {
            let bx = (W - 240) / 2;
            let by = (H - 64) / 2;
            fill(bx, by, 240, 64, C_HEADER);
            border(bx, by, 240, 64, C_RED);
            text(bx + 48, by + 12, C_RED, "Game Over!");
            text(bx + 20, by + 34, C_HINT, &format!("Score: {}   R to restart", g.score));
        }
        State::Playing => {}
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise breakout");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            game.step();
            draw(&game);
            if game.state == State::Playing {
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            "\x1b[D" => {
                game.paddle_x = (game.paddle_x - 8).max(0);
                if game.state == State::Waiting { game.park_ball(); }
            }
            "\x1b[C" => {
                game.paddle_x = (game.paddle_x + 8).min(W as i32 - PADDLE_W as i32);
                if game.state == State::Waiting { game.park_ball(); }
            }
            " " | "" | "\r" => {
                if game.state == State::Waiting {
                    game.state = State::Playing;
                    println!("@supervisor: ping");
                    let _ = io::stdout().flush();
                }
            }
            "r" | "R" => { game = Game::new(); }
            _ => {}
        }
        draw(&game);
    }
}
