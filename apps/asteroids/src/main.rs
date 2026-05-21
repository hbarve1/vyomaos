use std::io::{self, BufRead, Write};

const W: u32 = 900;
const H: u32 = 800;
const HEADER_H: u32 = 48;
const PLAY_W: i32 = W as i32;
const PLAY_H: i32 = (H - HEADER_H) as i32;
const PLAY_Y: i32 = HEADER_H as i32;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_ROCK: u32   = 0x8B949EFF;
const C_BULLET: u32 = 0xFFD60AFF;
const C_SHIP: u32   = 0x58A6FFFF;

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

// Fixed-point: positions and velocities stored ×16 for sub-pixel movement
const FP: i32 = 16;

struct Rock {
    x: i32, y: i32,   // ×FP
    dx: i32, dy: i32, // ×FP per tick
    size: i32,         // pixel size (64, 32, or 16)
}

struct Bullet {
    x: i32, y: i32,
    dx: i32, dy: i32,
    age: u32,
}

struct Ship {
    x: i32, y: i32,
    dx: i32, dy: i32,
    dir: u8, // 0=up 1=right 2=down 3=left (last thrust direction)
    invincible: u32, // ticks of invincibility after respawn
}

struct Game {
    ship: Ship,
    rocks: Vec<Rock>,
    bullets: Vec<Bullet>,
    lives: i32,
    score: u32,
    wave: u32,
    seed: u64,
    over: bool,
    thrust_up: bool,
    thrust_dn: bool,
    thrust_lt: bool,
    thrust_rt: bool,
}

fn make_rocks(seed: u64, wave: u32) -> (Vec<Rock>, u64) {
    let count = (6 + wave * 2).min(14) as usize;
    let mut rocks = Vec::with_capacity(count);
    let mut s = seed;
    for i in 0..count {
        s = lcg(s);
        // Place away from ship center (W/2, H/2)
        let x = ((s >> 33) as i32 % PLAY_W + PLAY_W) % PLAY_W;
        s = lcg(s);
        let y = ((s >> 33) as i32 % PLAY_H + PLAY_H) % PLAY_H;
        s = lcg(s);
        let dx = ((s >> 40) as i32 % 3 - 1) * FP + (s as i32 & 7) - 4;
        s = lcg(s);
        let dy = ((s >> 40) as i32 % 3 - 1) * FP + (s as i32 & 7) - 4;
        // Ensure rocks don't start on top of ship
        let sx = PLAY_W / 2;
        let sy = PLAY_H / 2;
        let ex = if (x - sx).abs() < 80 { (x + 200) % PLAY_W } else { x };
        let ey = if (y - sy).abs() < 80 { (y + 200) % PLAY_H } else { y };
        rocks.push(Rock { x: ex * FP, y: (ey + PLAY_Y) * FP, dx, dy, size: 64 });
    }
    (rocks, s)
}

impl Game {
    fn new(seed: u64) -> Self {
        let s = lcg(seed);
        let (rocks, s2) = make_rocks(s, 0);
        Game {
            ship: Ship {
                x: (PLAY_W / 2) * FP,
                y: (PLAY_Y + PLAY_H / 2) * FP,
                dx: 0, dy: 0,
                dir: 0,
                invincible: 60,
            },
            rocks,
            bullets: Vec::new(),
            lives: 3,
            score: 0,
            wave: 0,
            seed: s2,
            over: false,
            thrust_up: false,
            thrust_dn: false,
            thrust_lt: false,
            thrust_rt: false,
        }
    }

    fn fire(&mut self) {
        if self.bullets.len() >= 4 { return; }
        // Bullet velocity based on ship direction
        let spd = 8 * FP;
        let (bdx, bdy) = match self.ship.dir {
            0 => (0, -spd),
            1 => (spd, 0),
            2 => (0, spd),
            _ => (-spd, 0),
        };
        self.bullets.push(Bullet {
            x: self.ship.x,
            y: self.ship.y,
            dx: self.ship.dx + bdx,
            dy: self.ship.dy + bdy,
            age: 0,
        });
    }

    fn tick(&mut self) {
        if self.over { return; }

        // Thrust
        let accel = FP / 2;
        let max_spd = 6 * FP;
        if self.thrust_up { self.ship.dy -= accel; self.ship.dir = 0; }
        if self.thrust_dn { self.ship.dy += accel; self.ship.dir = 2; }
        if self.thrust_lt { self.ship.dx -= accel; self.ship.dir = 3; }
        if self.thrust_rt { self.ship.dx += accel; self.ship.dir = 1; }

        // Clamp speed
        self.ship.dx = self.ship.dx.clamp(-max_spd, max_spd);
        self.ship.dy = self.ship.dy.clamp(-max_spd, max_spd);

        // Friction
        self.ship.dx = self.ship.dx * 15 / 16;
        self.ship.dy = self.ship.dy * 15 / 16;

        // Move ship with wrap
        self.ship.x += self.ship.dx;
        self.ship.y += self.ship.dy;
        self.ship.x = ((self.ship.x % (PLAY_W * FP)) + PLAY_W * FP) % (PLAY_W * FP);
        let play_top = PLAY_Y * FP;
        let play_bot = (PLAY_Y + PLAY_H) * FP;
        if self.ship.y < play_top { self.ship.y = play_bot - FP; }
        if self.ship.y >= play_bot { self.ship.y = play_top; }

        if self.ship.invincible > 0 { self.ship.invincible -= 1; }

        // Move rocks
        for rock in &mut self.rocks {
            rock.x += rock.dx;
            rock.y += rock.dy;
            rock.x = ((rock.x % (PLAY_W * FP)) + PLAY_W * FP) % (PLAY_W * FP);
            let play_top_y = PLAY_Y * FP;
            let play_bot_y = (PLAY_Y + PLAY_H) * FP;
            if rock.y < play_top_y { rock.y = play_bot_y - FP; }
            if rock.y >= play_bot_y { rock.y = play_top_y; }
        }

        // Move + age bullets
        self.bullets.retain_mut(|b| {
            b.x += b.dx;
            b.y += b.dy;
            b.x = ((b.x % (PLAY_W * FP)) + PLAY_W * FP) % (PLAY_W * FP);
            if b.y < PLAY_Y * FP { b.y = (PLAY_Y + PLAY_H) * FP - FP; }
            if b.y >= (PLAY_Y + PLAY_H) * FP { b.y = PLAY_Y * FP; }
            b.age += 1;
            b.age < 40
        });

        // Bullet-rock collision
        let mut new_rocks: Vec<Rock> = Vec::new();
        let mut killed: Vec<bool> = vec![false; self.rocks.len()];
        let mut bullets_hit: Vec<bool> = vec![false; self.bullets.len()];

        for (bi, bullet) in self.bullets.iter().enumerate() {
            if bullets_hit[bi] { continue; }
            let bx = bullet.x / FP;
            let by = bullet.y / FP;
            for (ri, rock) in self.rocks.iter().enumerate() {
                if killed[ri] { continue; }
                let rx = rock.x / FP;
                let ry = rock.y / FP;
                let rs = rock.size;
                if bx >= rx && bx < rx + rs && by >= ry && by < ry + rs {
                    // Hit
                    bullets_hit[bi] = true;
                    killed[ri] = true;
                    let points = match rs { 16 => 10, 32 => 20, _ => 40 };
                    self.score += points;
                    if rs > 16 {
                        // Split into 2 smaller rocks
                        self.seed = lcg(self.seed);
                        let d1x = ((self.seed >> 33) as i32 & 0xF) - 8 + rock.dx / 2;
                        self.seed = lcg(self.seed);
                        let d1y = ((self.seed >> 33) as i32 & 0xF) - 8 + rock.dy / 2;
                        let d2x = -d1x + rock.dx / 2;
                        let d2y = -d1y + rock.dy / 2;
                        new_rocks.push(Rock { x: rock.x, y: rock.y, dx: d1x, dy: d1y, size: rs / 2 });
                        new_rocks.push(Rock { x: rock.x + rs * FP / 2, y: rock.y, dx: d2x, dy: d2y, size: rs / 2 });
                    }
                    break;
                }
            }
        }

        // Apply kills
        let mut ri2 = 0;
        self.rocks.retain(|_| { let keep = !killed[ri2]; ri2 += 1; keep });
        let mut bi2 = 0;
        self.bullets.retain(|_| { let keep = !bullets_hit[bi2]; bi2 += 1; keep });
        self.rocks.extend(new_rocks);

        // Ship-rock collision
        if self.ship.invincible == 0 {
            let sx = self.ship.x / FP;
            let sy = self.ship.y / FP;
            for rock in &self.rocks {
                let rx = rock.x / FP;
                let ry = rock.y / FP;
                let rs = rock.size;
                if sx + 10 > rx && sx < rx + rs && sy + 10 > ry && sy < ry + rs {
                    self.lives -= 1;
                    if self.lives <= 0 {
                        self.over = true;
                    } else {
                        // Respawn ship at center
                        self.ship.x = (PLAY_W / 2) * FP;
                        self.ship.y = (PLAY_Y + PLAY_H / 2) * FP;
                        self.ship.dx = 0;
                        self.ship.dy = 0;
                        self.ship.invincible = 90;
                    }
                    break;
                }
            }
        }

        // New wave if all rocks gone
        if self.rocks.is_empty() {
            self.wave += 1;
            let (new_rocks, s) = make_rocks(self.seed, self.wave);
            self.rocks = new_rocks;
            self.seed = s;
        }
    }
}

fn draw_ship(ship: &Ship) {
    if ship.invincible > 0 && (ship.invincible / 5) % 2 == 0 { return; } // blink
    let sx = (ship.x / FP) as u32;
    let sy = (ship.y / FP) as u32;
    // Draw as cross: center + fins based on direction
    fill(sx.saturating_sub(6), sy.saturating_sub(2), 18, 6, C_SHIP);
    fill(sx.saturating_sub(2), sy.saturating_sub(8), 6, 22, C_SHIP);
    // Thrust indicator
    match ship.dir {
        0 => fill(sx.saturating_sub(3), sy + 8, 8, 6, C_ORANGE),
        1 => fill(sx.saturating_sub(12), sy.saturating_sub(3), 6, 8, C_ORANGE),
        2 => fill(sx.saturating_sub(3), sy.saturating_sub(14), 8, 6, C_ORANGE),
        _ => fill(sx + 6, sy.saturating_sub(3), 6, 8, C_ORANGE),
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Asteroids");
    text(160, 16, C_HINT, &format!("Score: {}  Wave: {}  Lives: {}", g.score, g.wave + 1, g.lives));
    text(480, 16, C_HINT, "Arrows:thrust  Space:fire  N:new");

    // Life indicators
    for i in 0..g.lives.max(0) as u32 {
        fill(W - 60 + i * 18, 18, 10, 14, C_SEL);
    }

    // Rocks
    for rock in &g.rocks {
        let rx = (rock.x / FP) as u32;
        let ry = (rock.y / FP) as u32;
        let rs = rock.size as u32;
        fill(rx, ry, rs, rs, C_ROCK);
        border(rx, ry, rs, rs, C_BG);
    }

    // Bullets
    for b in &g.bullets {
        let bx = (b.x / FP) as u32;
        let by = (b.y / FP) as u32;
        fill(bx, by, 4, 4, C_BULLET);
    }

    // Ship
    if !g.over {
        draw_ship(&g.ship);
    }

    // Game Over overlay
    if g.over {
        let bx = (W - 360) / 2;
        let by = HEADER_H + 200;
        fill(bx, by, 360, 100, C_HEADER);
        border(bx, by, 360, 100, C_RED);
        text(bx + 80, by + 16, C_RED, "Game Over!");
        text(bx + 24, by + 44, C_HINT, &format!("Score: {}  Wave: {}  N=new", g.score, g.wave + 1));
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let seed: u64 = 0xDEADBEEF12345678;
    let mut game = Game::new(seed);

    println!("@supervisor: raise asteroids");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") {
            game.tick();
            if !game.over {
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            draw(&game);
            continue;
        }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "n" | "N" => {
                let s = lcg(game.seed);
                game = Game::new(s);
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            "\x1b[A" => { game.thrust_up = true; game.thrust_dn = false; }
            "\x1b[B" => { game.thrust_dn = true; game.thrust_up = false; }
            "\x1b[C" => { game.thrust_rt = true; game.thrust_lt = false; }
            "\x1b[D" => { game.thrust_lt = true; game.thrust_rt = false; }
            " " => { game.fire(); }
            _ => {}
        }
        draw(&game);
    }
}
