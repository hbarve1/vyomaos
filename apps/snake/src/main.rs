// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::collections::VecDeque;
use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 680;
const HEADER_H: u32 = 40;
const GRID_X: u32   = 0;
const GRID_Y: u32   = HEADER_H;
const CELL: u32     = 18;
const GRID_W: u32   = W / CELL;
const GRID_H: u32   = (H - HEADER_H) / CELL;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x161B22FF;
const C_BORDER: u32 = 0x30363DFF;
const C_GRID: u32   = 0x161B22FF;
const C_HEAD: u32   = 0x58A6FFFF;
const C_BODY: u32   = 0x3FB950FF;
const C_FOOD: u32   = 0xFF7B72FF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_SCORE: u32  = 0xFFA657FF;

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

#[derive(Clone, Copy, PartialEq)]
enum Dir { Up, Down, Left, Right }

struct Game {
    snake:   VecDeque<(i32, i32)>,
    dir:     Dir,
    next_dir: Dir,
    food:    (i32, i32),
    score:   u32,
    high:    u32,
    alive:   bool,
    started: bool,
    tick:    u64,
}

impl Game {
    fn new() -> Self {
        let sx = GRID_W as i32 / 2;
        let sy = GRID_H as i32 / 2;
        let mut snake = VecDeque::new();
        snake.push_back((sx, sy));
        snake.push_back((sx - 1, sy));
        snake.push_back((sx - 2, sy));
        let food = Self::new_food_pos(&snake, 0);
        Game {
            snake, dir: Dir::Right, next_dir: Dir::Right,
            food, score: 0, high: 0, alive: true, started: false, tick: 0,
        }
    }

    fn new_food_pos(snake: &VecDeque<(i32, i32)>, seed: u64) -> (i32, i32) {
        let total = GRID_W as u64 * GRID_H as u64;
        let mut pos = (seed * 7919 + 31337) % total;
        loop {
            let fx = (pos % GRID_W as u64) as i32;
            let fy = (pos / GRID_W as u64) as i32;
            if !snake.contains(&(fx, fy)) {
                return (fx, fy);
            }
            pos = (pos + 1) % total;
        }
    }

    fn step(&mut self) {
        if !self.alive || !self.started { return; }
        self.dir = self.next_dir;
        self.tick += 1;

        let (hx, hy) = *self.snake.front().unwrap();
        let (nx, ny) = match self.dir {
            Dir::Up    => (hx, hy - 1),
            Dir::Down  => (hx, hy + 1),
            Dir::Left  => (hx - 1, hy),
            Dir::Right => (hx + 1, hy),
        };

        // Wall collision
        if nx < 0 || ny < 0 || nx >= GRID_W as i32 || ny >= GRID_H as i32 {
            self.alive = false;
            if self.score > self.high { self.high = self.score; }
            return;
        }

        // Self collision
        if self.snake.contains(&(nx, ny)) {
            self.alive = false;
            if self.score > self.high { self.high = self.score; }
            return;
        }

        self.snake.push_front((nx, ny));

        if (nx, ny) == self.food {
            self.score += 10;
            self.food = Self::new_food_pos(&self.snake, self.tick);
        } else {
            self.snake.pop_back();
        }
    }

    fn restart(&mut self) {
        let high = self.high.max(self.score);
        *self = Game::new();
        self.high = high;
        self.started = true;
    }
}

fn draw_cell(gx: i32, gy: i32, color: u32) {
    let px = GRID_X + gx as u32 * CELL + 1;
    let py = GRID_Y + gy as u32 * CELL + 1;
    fill(px, py, CELL - 2, CELL - 2, color);
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    text(16, 12, C_TEXT, "Snake");
    let score_str = format!("Score: {}  High: {}", g.score, g.high);
    text(100, 12, C_SCORE, &score_str);
    text(W - 260, 12, C_HINT, "Arrows: direction  Space: start  Esc: quit");

    // Grid background
    fill(GRID_X, GRID_Y, W, H - HEADER_H, C_GRID);

    // Draw subtle grid lines
    for gx in 0..=GRID_W {
        fill(GRID_X + gx * CELL, GRID_Y, 1, H - HEADER_H, 0x161B22FF);
    }

    if !g.started {
        let msg = "Press Space to start";
        let mw = msg.len() as u32 * 8;
        text((W - mw) / 2, H / 2 - 8, C_HINT, msg);
    } else if !g.alive {
        // Game over overlay
        fill(W / 4, H / 2 - 40, W / 2, 80, 0x0D1117FF);
        border(W / 4, H / 2 - 40, W / 2, 80, C_FOOD);
        let over_str = format!("GAME OVER — Score: {}", g.score);
        let ow = over_str.len() as u32 * 8;
        text((W - ow) / 2, H / 2 - 24, C_TEXT, &over_str);
        text((W - 152) / 2, H / 2 + 4, C_HINT, "Space to restart");
    } else {
        // Food
        draw_cell(g.food.0, g.food.1, C_FOOD);

        // Snake body
        for (i, &(sx, sy)) in g.snake.iter().enumerate() {
            let color = if i == 0 { C_HEAD } else {
                let fade = (255 - (i as u32 * 8).min(180)) as u32;
                (C_BODY & 0xFFFFFF00) | fade
            };
            draw_cell(sx, sy, color);
        }
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();
    let mut tick_count = 0u32;

    println!("@supervisor: ping");
    println!("@supervisor: raise snake");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            tick_count += 1;
            // Move every 3 pongs (~333ms)
            if tick_count % 3 == 0 && game.started {
                game.step();
                draw(&game);
            }
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            " " => {
                if !game.started || !game.alive {
                    game.restart();
                } else {
                    game.started = true;
                }
                draw(&game);
            }
            "\x1b[A" => {
                if game.dir != Dir::Down  { game.next_dir = Dir::Up; }
                if !game.started { game.started = true; }
                draw(&game);
            }
            "\x1b[B" => {
                if game.dir != Dir::Up    { game.next_dir = Dir::Down; }
                if !game.started { game.started = true; }
                draw(&game);
            }
            "\x1b[D" => {
                if game.dir != Dir::Right { game.next_dir = Dir::Left; }
                if !game.started { game.started = true; }
                draw(&game);
            }
            "\x1b[C" => {
                if game.dir != Dir::Left  { game.next_dir = Dir::Right; }
                if !game.started { game.started = true; }
                draw(&game);
            }
            _ => {}
        }
    }
}
