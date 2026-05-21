use std::io::{self, BufRead, Write};

const W: u32 = 840;
const H: u32 = 640;
const HEADER_H: u32 = 40;
const STATUS_H: u32 = 24;
const CONTENT_Y: u32 = HEADER_H;
const CONTENT_H: u32 = H - HEADER_H - STATUS_H;

// Timer ring geometry
const RING_CX: u32 = 260;
const RING_CY: u32 = CONTENT_Y + 160;
const RING_R: u32 = 110;
const RING_T: u32 = 14; // thickness

// Task list panel
const TASK_X: u32 = 500;
const TASK_W: u32 = W - TASK_X - 8;

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
const C_CARD: u32   = 0x161B22FF;

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

// Draw a progress ring using arc segments (fill_rect approximation with concentric squares)
fn draw_ring(cx: u32, cy: u32, r: u32, t: u32, progress: f32, fg: u32, bg: u32) {
    // Draw background ring then overlay progress arc
    // We approximate with 72 segments (5° each)
    let segs = 72u32;
    for seg in 0..segs {
        let angle = seg as f32 * std::f32::consts::PI * 2.0 / segs as f32 - std::f32::consts::FRAC_PI_2;
        let filled = seg as f32 / segs as f32 < progress;
        let col = if filled { fg } else { bg };

        // Outer and inner points for this segment
        let ox = (cx as f32 + (r as f32) * angle.cos()) as u32;
        let oy = (cy as f32 + (r as f32) * angle.sin()) as u32;
        fill(ox.saturating_sub(t / 2), oy.saturating_sub(t / 2), t, t, col);
    }
}

#[derive(PartialEq, Clone, Copy)]
enum SessionType { Work, ShortBreak, LongBreak }

impl SessionType {
    fn name(self) -> &'static str {
        match self { SessionType::Work => "WORK", SessionType::ShortBreak => "SHORT BREAK", SessionType::LongBreak => "LONG BREAK" }
    }
    fn color(self) -> u32 {
        match self { SessionType::Work => C_ORANGE, SessionType::ShortBreak => C_GREEN, SessionType::LongBreak => C_SEL }
    }
    fn default_secs(self) -> u32 {
        match self { SessionType::Work => 25 * 60, SessionType::ShortBreak => 5 * 60, SessionType::LongBreak => 15 * 60 }
    }
}

struct Task {
    name:      String,
    pomodoros: u32,
}

struct App {
    session:       SessionType,
    work_secs:     u32,
    short_secs:    u32,
    long_secs:     u32,
    remaining:     u32,
    running:       bool,
    pomodoros_done: u32,
    streak:        u32,
    longest_streak: u32,
    total_work_secs: u32,
    tasks:         Vec<Task>,
    task_sel:      usize,
    adding:        bool,
    add_buf:       String,
    status:        String,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            session: SessionType::Work,
            work_secs: 25 * 60,
            short_secs: 5 * 60,
            long_secs: 15 * 60,
            remaining: 25 * 60,
            running: false,
            pomodoros_done: 0,
            streak: 0,
            longest_streak: 0,
            total_work_secs: 0,
            tasks: Vec::new(),
            task_sel: 0,
            adding: false,
            add_buf: String::new(),
            status: String::from("Enter=start/pause  N=new task  Del=remove  +/-=adjust interval  Tab=session"),
        };
        a.tasks.push(Task { name: "Write supervisor docs".to_string(), pomodoros: 2 });
        a.tasks.push(Task { name: "Implement P163 pomodoro-pro".to_string(), pomodoros: 1 });
        a.tasks.push(Task { name: "Review WASM security model".to_string(), pomodoros: 0 });
        a
    }

    fn current_total(&self) -> u32 {
        match self.session {
            SessionType::Work => self.work_secs,
            SessionType::ShortBreak => self.short_secs,
            SessionType::LongBreak => self.long_secs,
        }
    }

    fn tick(&mut self) {
        if !self.running { return; }
        if self.remaining > 0 {
            self.remaining -= 1;
            if self.session == SessionType::Work {
                self.total_work_secs += 1;
            }
        }
        if self.remaining == 0 {
            self.running = false;
            if self.session == SessionType::Work {
                self.pomodoros_done += 1;
                self.streak += 1;
                if self.streak > self.longest_streak { self.longest_streak = self.streak; }
                if !self.tasks.is_empty() {
                    self.tasks[self.task_sel].pomodoros += 1;
                }
                // Auto-advance: every 4 pomodoros → long break
                if self.pomodoros_done % 4 == 0 {
                    self.session = SessionType::LongBreak;
                    self.remaining = self.long_secs;
                } else {
                    self.session = SessionType::ShortBreak;
                    self.remaining = self.short_secs;
                }
                self.status = format!("Pomodoro {} complete! Starting {}", self.pomodoros_done, self.session.name());
            } else {
                self.streak = 0;
                self.session = SessionType::Work;
                self.remaining = self.work_secs;
                self.status = "Break over! Ready for next pomodoro.".to_string();
            }
        }
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Pomodoro Pro");
    let session_col = app.session.color();
    text(180, 12, session_col, app.session.name());
    let run_label = if app.running { "▶ RUNNING" } else { "⏸ PAUSED" };
    let run_col = if app.running { C_GREEN } else { C_HINT };
    text(W - 120, 12, run_col, run_label);

    // Timer ring
    let total = app.current_total().max(1);
    let progress = 1.0 - (app.remaining as f32 / total as f32);
    let ring_bg = 0x1A1F27FF;
    draw_ring(RING_CX, RING_CY, RING_R, RING_T, 1.0, ring_bg, ring_bg);
    draw_ring(RING_CX, RING_CY, RING_R, RING_T, progress, session_col, ring_bg);

    // Time display in center
    let mins = app.remaining / 60;
    let secs = app.remaining % 60;
    let time_str = format!("{:02}:{:02}", mins, secs);
    text(RING_CX - 28, RING_CY - 10, session_col, &time_str);
    text(RING_CX - 24, RING_CY + 12, C_HINT, app.session.name());

    // Pomodoro dots (●○) below ring
    let dots_y = RING_CY + RING_R + 20;
    let dot_cycle = app.pomodoros_done % 4;
    for i in 0..4u32 {
        let dx = RING_CX - 32 + i * 20;
        let dot = if i < dot_cycle { "●" } else { "○" };
        let dc = if i < dot_cycle { C_ORANGE } else { C_HINT };
        text(dx, dots_y, dc, dot);
    }
    text(RING_CX - 32, dots_y + 18, C_HINT, &format!("Pomodoro {} of 4", dot_cycle));

    // Stats panel below ring
    let stats_y = dots_y + 44;
    fill(16, stats_y, TASK_X - 32, 100, C_CARD);
    border(16, stats_y, TASK_X - 32, 100, C_BORDER);
    text(24, stats_y + 8, C_HINT, "Stats Today");
    text(24, stats_y + 26, C_TEXT, &format!("Pomodoros:  {}", app.pomodoros_done));
    text(24, stats_y + 44, C_TEXT, &format!("Work time:  {}m {}s", app.total_work_secs / 60, app.total_work_secs % 60));
    text(24, stats_y + 62, C_TEXT, &format!("Cur streak: {}  Best: {}", app.streak, app.longest_streak));

    // Interval config
    let cfg_y = stats_y + 110;
    text(16, cfg_y, C_HINT, &format!("Work: {}m  Short: {}m  Long: {}m  (+/- to adjust)",
        app.work_secs / 60, app.short_secs / 60, app.long_secs / 60));

    // Task list panel
    fill(TASK_X, CONTENT_Y, TASK_W, CONTENT_H, C_CARD);
    border(TASK_X, CONTENT_Y, TASK_W, CONTENT_H, C_BORDER);
    text(TASK_X + 8, CONTENT_Y + 6, C_HINT, "Tasks");
    fill(TASK_X, CONTENT_Y + 22, TASK_W, 1, C_BORDER);

    if app.adding {
        let ay = CONTENT_Y + 28;
        fill(TASK_X + 4, ay, TASK_W - 8, 20, 0x1C2D4EFF);
        border(TASK_X + 4, ay, TASK_W - 8, 20, C_SEL);
        text(TASK_X + 8, ay + 4, C_TEXT, &format!("{}▌", app.add_buf));
    }

    for (i, task) in app.tasks.iter().enumerate() {
        let ty = CONTENT_Y + 30 + (if app.adding { 24 } else { 0 }) + i as u32 * 22;
        if ty + 22 > CONTENT_Y + CONTENT_H { break; }
        if i == app.task_sel {
            fill(TASK_X + 2, ty, TASK_W - 4, 20, 0x1C2D4EFF);
        }
        let max_name = (TASK_W / 8 - 6) as usize;
        let name = if task.name.len() > max_name { &task.name[..max_name] } else { &task.name };
        let tc = if i == app.task_sel { C_SEL } else { C_TEXT };
        text(TASK_X + 8, ty + 4, tc, name);
        // Pomodoro count dots on right
        let dots: String = (0..task.pomodoros.min(8)).map(|_| "●").collect();
        let dots_x = TASK_X + TASK_W - dots.len() as u32 * 8 - 8;
        text(dots_x, ty + 4, C_ORANGE, &dots);
    }

    if app.tasks.is_empty() {
        text(TASK_X + 8, CONTENT_Y + 40, C_HINT, "No tasks — press N to add");
    }

    // Keyboard hint at bottom of task panel
    let hint_y = CONTENT_Y + CONTENT_H - 20;
    text(TASK_X + 4, hint_y, C_HINT, "N=add  Del=remove  ↑↓=select");

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise pomodoro-pro");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            app.tick();
            if app.running {
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            draw(&app);
            continue;
        }

        if app.adding {
            match raw.as_str() {
                "\x03" | "\x1b" => { app.adding = false; app.add_buf.clear(); }
                "\x7f" => { app.add_buf.pop(); }
                "" => {
                    if !app.add_buf.is_empty() && app.tasks.len() < 10 {
                        app.tasks.push(Task { name: app.add_buf.clone(), pomodoros: 0 });
                        app.task_sel = app.tasks.len() - 1;
                        app.status = format!("Task added: {}", app.add_buf);
                    }
                    app.add_buf.clear();
                    app.adding = false;
                }
                s if s.len() == 1 && !s.chars().next().map(|c| c.is_control()).unwrap_or(true) => {
                    if app.add_buf.len() < 40 { app.add_buf.push_str(s); }
                }
                _ => {}
            }
            draw(&app);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "" => {
                app.running = !app.running;
                if app.running {
                    println!("@supervisor: ping");
                    let _ = io::stdout().flush();
                    app.status = format!("{} — {:02}:{:02} remaining", app.session.name(), app.remaining / 60, app.remaining % 60);
                } else {
                    app.status = "Paused. Press Enter to resume.".to_string();
                }
            }
            "\t" => {
                app.running = false;
                app.session = match app.session {
                    SessionType::Work => SessionType::ShortBreak,
                    SessionType::ShortBreak => SessionType::LongBreak,
                    SessionType::LongBreak => SessionType::Work,
                };
                app.remaining = app.current_total();
                app.status = format!("Switched to {}", app.session.name());
            }
            "n" | "N" => {
                if app.tasks.len() < 10 {
                    app.adding = true;
                    app.add_buf.clear();
                }
            }
            "\x7f" | "\x1b[3~" => {
                if !app.tasks.is_empty() {
                    let name = app.tasks[app.task_sel].name.clone();
                    app.tasks.remove(app.task_sel);
                    if app.task_sel > 0 && app.task_sel >= app.tasks.len() {
                        app.task_sel -= 1;
                    }
                    app.status = format!("Removed: {}", name);
                }
            }
            "\x1b[A" => {
                if app.task_sel > 0 { app.task_sel -= 1; }
            }
            "\x1b[B" => {
                if app.task_sel + 1 < app.tasks.len() { app.task_sel += 1; }
            }
            "+" | "=" => {
                match app.session {
                    SessionType::Work => app.work_secs = (app.work_secs + 5 * 60).min(60 * 60),
                    SessionType::ShortBreak => app.short_secs = (app.short_secs + 5 * 60).min(30 * 60),
                    SessionType::LongBreak => app.long_secs = (app.long_secs + 5 * 60).min(60 * 60),
                }
                app.remaining = app.current_total();
                app.status = format!("{} interval: {}m", app.session.name(), app.current_total() / 60);
            }
            "-" | "_" => {
                match app.session {
                    SessionType::Work => app.work_secs = app.work_secs.saturating_sub(5 * 60).max(5 * 60),
                    SessionType::ShortBreak => app.short_secs = app.short_secs.saturating_sub(5 * 60).max(5 * 60),
                    SessionType::LongBreak => app.long_secs = app.long_secs.saturating_sub(5 * 60).max(5 * 60),
                }
                app.remaining = app.current_total();
                app.status = format!("{} interval: {}m", app.session.name(), app.current_total() / 60);
            }
            "r" | "R" => {
                app.running = false;
                app.remaining = app.current_total();
                app.status = "Timer reset.".to_string();
            }
            _ => {}
        }
        draw(&app);
    }
}
