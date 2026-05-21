use std::io::{self, BufRead, Write};

const W: u32 = 840;
const H: u32 = 640;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_CARD: u32    = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;
const C_DONE: u32    = 0x1A3A1AFF;

const HABITS: [&str; 10] = [
    "Exercise 30 min",
    "Drink 8 glasses water",
    "Read 20 pages",
    "Meditate 10 min",
    "No social media",
    "Sleep before 11pm",
    "Code something",
    "Eat vegetables",
    "Gratitude journal",
    "Walk 5000 steps",
];

const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

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

struct App {
    grid:    [[bool; 7]; 10],
    sel:     usize, // selected habit row
    today:   usize, // index of today in 0..7 (default = 6 = Sunday/rightmost)
}

impl App {
    fn new() -> Self {
        App {
            grid: [[false; 7]; 10],
            sel: 0,
            today: 6,
        }
    }

    fn streak(&self, habit: usize) -> usize {
        let mut s = 0;
        for d in (0..=self.today).rev() {
            if self.grid[habit][d] { s += 1; } else { break; }
        }
        s
    }

    fn toggle_today(&mut self) {
        let t = self.today;
        self.grid[self.sel][t] = !self.grid[self.sel][t];
    }

    fn next_day(&mut self) {
        // Shift all columns left, clear today (rightmost)
        for h in 0..10 {
            for d in 0..6 { self.grid[h][d] = self.grid[h][d + 1]; }
            self.grid[h][6] = false;
        }
    }

    fn reset(&mut self) {
        self.grid = [[false; 7]; 10];
    }

    fn completed_today(&self) -> usize {
        self.grid.iter().filter(|h| h[self.today]).count()
    }
}

const GRID_X: u32 = 260;
const GRID_Y: u32 = 100;
const CELL_W: u32 = 68;
const CELL_H: u32 = 46;
const HABIT_X: u32 = 20;

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Habit Tracker");
    text(200, 16, C_HINT, &format!("Today: {}/{} done", app.completed_today(), 10));
    text(460, 16, C_HINT, "↑↓:select  Space:toggle  n:next day  r:reset");

    // Day headers
    for (d, &day) in DAYS.iter().enumerate() {
        let dx = GRID_X + d as u32 * CELL_W + (CELL_W - 24) / 2;
        let col = if d == app.today { C_SEL } else { C_HINT };
        text(dx, GRID_Y - 22, col, day);
        if d == app.today {
            fill(GRID_X + d as u32 * CELL_W, GRID_Y - 26, CELL_W, 4, C_SEL);
        }
    }

    // Habits + grid
    for (h, &habit) in HABITS.iter().enumerate() {
        let hy = GRID_Y + h as u32 * CELL_H;
        let is_sel = h == app.sel;

        // Habit name
        let habit_bg = if is_sel { C_SEL_BG } else { C_BG };
        fill(HABIT_X, hy, GRID_X - HABIT_X - 8, CELL_H - 2, habit_bg);
        let streak = app.streak(h);
        let streak_str = if streak > 0 { format!(" {}d", streak) } else { String::new() };
        let hc = if is_sel { C_TEXT } else { C_HINT };
        text(HABIT_X + 6, hy + 14, hc, habit);
        if streak > 0 {
            text(HABIT_X + 6 + habit.len() as u32 * 8, hy + 14, C_ORANGE, &streak_str);
        }

        // Day cells
        for d in 0..7 {
            let dx = GRID_X + d as u32 * CELL_W;
            let done = app.grid[h][d];
            let is_today = d == app.today;

            let bg = if done { C_DONE } else { C_CARD };
            fill(dx + 4, hy + 4, CELL_W - 8, CELL_H - 8, bg);

            let dot_color = if done { C_GREEN } else { C_BORDER };
            let dot_x = dx + CELL_W / 2 - 10;
            let dot_y = hy + CELL_H / 2 - 10;
            fill(dot_x, dot_y, 20, 20, dot_color);

            if is_today {
                border(dx + 4, hy + 4, CELL_W - 8, CELL_H - 8, C_SEL);
            }
        }
    }

    // Summary row
    let sy = GRID_Y + 10 * CELL_H + 16;
    fill(HABIT_X, sy, W - HABIT_X * 2, 40, C_CARD);
    border(HABIT_X, sy, W - HABIT_X * 2, 40, C_BORDER);
    let total_done: usize = (0..10).map(|h| app.grid[h][app.today] as usize).sum();
    let total_streaks: usize = (0..10).map(|h| app.streak(h)).sum();
    text(HABIT_X + 16, sy + 12, C_GREEN, &format!("Today: {}/10 habits", total_done));
    text(HABIT_X + 240, sy + 12, C_ORANGE, &format!("Total streak days: {}", total_streaks));

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise habit-tracker");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => { if app.sel > 0 { app.sel -= 1; } }
            "\x1b[B" => { if app.sel + 1 < 10 { app.sel += 1; } }
            " " | "\r" | "" => { app.toggle_today(); }
            "n" | "N" => { app.next_day(); }
            "r" | "R" => { app.reset(); }
            _ => {}
        }
        draw(&app);
    }
}
