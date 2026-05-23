use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 700;
const HEADER_H: u32 = 40;
const PADDLE_W: u32 = 12;
const PADDLE_H: u32 = 80;
const LEFT_X: u32 = 20;
const RIGHT_X: u32 = W - LEFT_X - PADDLE_W; // 768
const BALL_SZ: u32 = 12;
const WIN_SCORE: u32 = 7;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;

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
enum State { Waiting, Playing, Over }

struct Game {
    left_y:  i32,
    right_y: i32,
    ball_x:  i32,
    ball_y:  i32,
    vel_x:   i32,
    vel_y:   i32,
    score_l: u32,
    score_r: u32,
    winner:  u8,  // 0=none 1=player 2=ai
    state:   State,
}

impl Game {
    fn new() -> Self {
        let mid_y = ((H + HEADER_H) as i32 - PADDLE_H as i32) / 2;
        Game {
            left_y:  mid_y,
            right_y: mid_y,
            ball_x:  (W / 2 - BALL_SZ / 2) as i32,
            ball_y:  ((H + HEADER_H) / 2 - BALL_SZ / 2) as i32,
            vel_x: 5, vel_y: 4,
            score_l: 0, score_r: 0,
            winner: 0,
            state: State::Waiting,
        }
    }

    fn reset_ball(&mut self, dir: i32) {
        self.ball_x = (W / 2 - BALL_SZ / 2) as i32;
        self.ball_y = ((H + HEADER_H) / 2 - BALL_SZ / 2) as i32;
        self.vel_x = 5 * dir;
        self.vel_y = 4;
        self.state = State::Waiting;
    }

    fn update_ai(&mut self) {
        let ball_cy = self.ball_y + BALL_SZ as i32 / 2;
        let pad_cy = self.right_y + PADDLE_H as i32 / 2;
        let diff = ball_cy - pad_cy;
        let speed = 4i32;
        self.right_y += diff.clamp(-speed, speed);
        self.right_y = self.right_y.clamp(HEADER_H as i32, H as i32 - PADDLE_H as i32);
    }

    fn step(&mut self) {
        if self.state != State::Playing { return; }

        self.ball_x += self.vel_x;
        self.ball_y += self.vel_y;

        // Top/bottom walls
        if self.ball_y < HEADER_H as i32 {
            self.ball_y = HEADER_H as i32;
            self.vel_y = self.vel_y.abs();
        }
        if self.ball_y + BALL_SZ as i32 > H as i32 {
            self.ball_y = H as i32 - BALL_SZ as i32;
            self.vel_y = -self.vel_y.abs();
        }

        let bbot = self.ball_y + BALL_SZ as i32;
        let brit = self.ball_x + BALL_SZ as i32;

        // Left paddle (player)
        if self.vel_x < 0
            && brit >= LEFT_X as i32
            && self.ball_x <= (LEFT_X + PADDLE_W) as i32
            && bbot > self.left_y
            && self.ball_y < self.left_y + PADDLE_H as i32
        {
            self.ball_x = (LEFT_X + PADDLE_W) as i32;
            let off = (self.ball_y + BALL_SZ as i32 / 2) - (self.left_y + PADDLE_H as i32 / 2);
            self.vel_y = (off / 8).clamp(-6, 6);
            if self.vel_y == 0 { self.vel_y = 1; }
            self.vel_x = 5;
        }

        // Right paddle (AI)
        if self.vel_x > 0
            && self.ball_x + BALL_SZ as i32 >= RIGHT_X as i32
            && self.ball_x < (RIGHT_X + PADDLE_W) as i32
            && bbot > self.right_y
            && self.ball_y < self.right_y + PADDLE_H as i32
        {
            self.ball_x = RIGHT_X as i32 - BALL_SZ as i32;
            let off = (self.ball_y + BALL_SZ as i32 / 2) - (self.right_y + PADDLE_H as i32 / 2);
            self.vel_y = (off / 8).clamp(-6, 6);
            if self.vel_y == 0 { self.vel_y = -1; }
            self.vel_x = -5;
        }

        // AI movement
        self.update_ai();

        // Scoring
        if self.ball_x + BALL_SZ as i32 < 0 {
            self.score_r += 1;
            if self.score_r >= WIN_SCORE { self.winner = 2; self.state = State::Over; }
            else { self.reset_ball(1); }
        } else if self.ball_x > W as i32 {
            self.score_l += 1;
            if self.score_l >= WIN_SCORE { self.winner = 1; self.state = State::Over; }
            else { self.reset_ball(-1); }
        }
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);

    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_TEXT, "Pong");
    text(W/2 - 60, 12, C_GREEN, &format!("{}", g.score_l));
    text(W/2 - 16, 12, C_HINT, "vs");
    text(W/2 + 20, 12, C_RED, &format!("{}", g.score_r));
    text(W - 240, 12, C_HINT, "Arrows: move  Space: serve  R: restart");

    // Center dashed line
    let mut y = HEADER_H + 8;
    while y + 16 < H {
        fill(W/2 - 1, y, 2, 12, C_BORDER);
        y += 24;
    }

    // Paddles
    fill(LEFT_X, g.left_y.max(HEADER_H as i32) as u32, PADDLE_W, PADDLE_H, C_GREEN);
    fill(RIGHT_X, g.right_y.max(HEADER_H as i32) as u32, PADDLE_W, PADDLE_H, C_RED);

    // Ball
    fill(g.ball_x.max(0) as u32, g.ball_y.max(HEADER_H as i32) as u32, BALL_SZ, BALL_SZ, C_TEXT);

    // Overlays
    match g.state {
        State::Waiting => {
            let msg = "Press Space to serve";
            let mw = msg.len() as u32 * 8;
            text((W - mw) / 2, H - 24, C_HINT, msg);
        }
        State::Over => {
            let (msg, color) = if g.winner == 1 { ("You Win!", C_GREEN) } else { ("AI Wins!", C_RED) };
            let bx = (W - 200) / 2;
            let by = (H + HEADER_H) / 2 - 32;
            fill(bx, by, 200, 64, C_HEADER);
            border(bx, by, 200, 64, color);
            text(bx + 48, by + 12, color, msg);
            text(bx + 32, by + 34, C_HINT, "R to play again");
        }
        _ => {}
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut game = Game::new();

    println!("@supervisor: raise pong");
    println!("@supervisor: ping");
    let _ = io::stdout().flush();

    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw == "REPLY:pong" {
            game.step();
            draw(&game);
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            continue;
        }
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                game.left_y = (game.left_y - 8).max(HEADER_H as i32);
            }
            "\x1b[B" => {
                game.left_y = (game.left_y + 8).min(H as i32 - PADDLE_H as i32);
            }
            " " | "" | "\r" => {
                if game.state == State::Waiting {
                    game.state = State::Playing;
                }
            }
            "r" | "R" => { game = Game::new(); }
            _ => {}
        }
        draw(&game);
    }
}
