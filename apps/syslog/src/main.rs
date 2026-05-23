use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 800;
const HEADER_H: u32 = 48;
const FILTER_H: u32 = 28;
const STATUS_H: u32 = 28;
const LINE_H: u32 = 16;
const CHAR_W: u32 = 8;
const RING: usize = 200;

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

#[derive(Clone, Copy, PartialEq)]
enum Level { Debug, Info, Warn, Error, Fatal }

impl Level {
    fn name(self) -> &'static str {
        match self { Level::Debug=>"DEBUG", Level::Info=>"INFO ", Level::Warn=>"WARN ", Level::Error=>"ERROR", Level::Fatal=>"FATAL" }
    }
    fn color(self) -> u32 {
        match self { Level::Debug=>C_HINT, Level::Info=>C_TEXT, Level::Warn=>C_YELLOW, Level::Error=>C_RED, Level::Fatal=>C_RED }
    }
    fn from_seed(s: u64) -> Self {
        match s % 16 {
            0..=3  => Level::Debug,
            4..=8  => Level::Info,
            9..=11 => Level::Warn,
            12..=14=> Level::Error,
            _      => Level::Fatal,
        }
    }
    fn idx(self) -> usize {
        match self { Level::Debug=>0, Level::Info=>1, Level::Warn=>2, Level::Error=>3, Level::Fatal=>4 }
    }
}

const TAGS: &[&str] = &["kernel", "supervisor", "display", "ipc", "fs", "net", "app"];

const MESSAGES: &[&str] = &[
    "context switch completed in 42µs",
    "app started: shell (pid=12)",
    "framebuffer: 1440×900 @32bpp mapped",
    "IPC route: @supervisor → wasmtime",
    "9P mount: /data at blkdev0",
    "TCP connection established to 10.0.0.2:443",
    "WASM module loaded: 8.2KB in 1.1ms",
    "keyboard event dispatched to focused app",
    "display flush: 16.7ms (60 FPS)",
    "memory: 42MB used / 256MB total",
    "watchdog: app heartbeat OK",
    "seccomp filter applied to pid=15",
    "pkg install: hello-world@0.1.0",
    "session save: 3 windows serialized",
    "font cache hit ratio: 98.4%",
    "OTA check: up to date (v1.0.0)",
    "wasmtime: JIT compile 2.3ms",
    "signal SIGTERM → pid=8 (shell)",
    "virtio-gpu: mode change 1280×800",
    "IPC buffer overflow dropped 2 msgs",
    "FATAL: app panic in wasm trap",
    "restarting app: crash-reporter",
    "boot sequence complete in 0.54s",
    "entropy pool initialized",
];

#[derive(Clone)]
struct Entry {
    level: Level,
    tag:   &'static str,
    msg:   &'static str,
    tick:  u64,
}

struct Logger {
    entries:  Vec<Entry>,
    scroll:   usize,
    paused:   bool,
    filters:  [bool; 5], // Debug Info Warn Error Fatal
    tick:     u64,
    seed:     u64,
}

impl Logger {
    fn new() -> Self {
        let mut l = Logger {
            entries: Vec::with_capacity(RING),
            scroll: 0,
            paused: false,
            filters: [true; 5],
            tick: 0,
            seed: 0xA091C_12345678ABu64,
        };
        // Pre-populate with 20 entries
        for _ in 0..20 { l.generate(); }
        l
    }

    fn generate(&mut self) {
        self.seed = lcg(self.seed);
        let level = Level::from_seed(self.seed);
        self.seed = lcg(self.seed);
        let tag = TAGS[(self.seed as usize) % TAGS.len()];
        self.seed = lcg(self.seed);
        let msg = MESSAGES[(self.seed as usize) % MESSAGES.len()];
        let entry = Entry { level, tag, msg, tick: self.tick };
        if self.entries.len() >= RING { self.entries.remove(0); }
        self.entries.push(entry);
    }

    fn tick(&mut self) {
        if self.paused { return; }
        self.tick += 1;
        self.seed = lcg(self.seed);
        let n = (self.seed % 3 + 1) as usize;
        for _ in 0..n { self.generate(); }
        if self.scroll == 0 { /* snap to bottom */ }
    }

    fn filtered(&self) -> Vec<&Entry> {
        self.entries.iter().filter(|e| self.filters[e.level.idx()]).collect()
    }

    fn export(&self) -> Result<(), String> {
        let mut out = String::new();
        for e in &self.entries {
            out.push_str(&format!("[{:6}] [{}] [{}] {}\n", e.tick, e.level.name(), e.tag, e.msg));
        }
        std::fs::write("/data/syslog.txt", out).map_err(|e| e.to_string())
    }
}

const CONTENT_Y: u32 = HEADER_H + FILTER_H;
const CONTENT_H: u32 = H - HEADER_H - FILTER_H - STATUS_H;

fn draw(log: &Logger) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "System Logger");
    let ps = if log.paused { " [PAUSED]" } else { "" };
    text(200, 16, if log.paused { C_RED } else { C_GREEN }, if log.paused { "PAUSED" } else { "LIVE  " });
    text(310, 16, C_HINT, "P:pause  R:resume  C:clear  E:export  F1-F5:filter  ↑↓/PgUp/Dn:scroll");

    // Filter bar
    fill(0, HEADER_H, W, FILTER_H, 0x161B22FF);
    fill(0, HEADER_H + FILTER_H - 1, W, 1, C_BORDER);
    let level_labels = [("F1:DEBUG",Level::Debug),("F2:INFO",Level::Info),("F3:WARN",Level::Warn),("F4:ERROR",Level::Error),("F5:FATAL",Level::Fatal)];
    let filtered = log.filtered();
    let total_shown = filtered.len();

    for (i, (lbl, lvl)) in level_labels.iter().enumerate() {
        let en = log.filters[i];
        let col = if en { lvl.color() } else { 0x2A2F36FF };
        let cnt = log.entries.iter().filter(|e| e.level == *lvl).count();
        let label = format!("{} ({})", lbl, cnt);
        let lx = 8 + i as u32 * 200;
        text(lx, HEADER_H + 7, col, &label);
    }

    // Log entries
    fill(0, CONTENT_Y, W, CONTENT_H, C_BG);
    let lines_vis = (CONTENT_H / LINE_H) as usize;
    let scroll = log.scroll.min(total_shown.saturating_sub(lines_vis));
    let start = total_shown.saturating_sub(lines_vis + scroll);
    let end = total_shown.saturating_sub(scroll);

    for (vi, entry) in filtered[start..end].iter().enumerate() {
        let ly = CONTENT_Y + vi as u32 * LINE_H;
        let col = entry.level.color();

        // Level badge
        fill(8, ly + 2, 40, LINE_H - 4, col);
        text(10, ly + 2, C_BG, entry.level.name());

        // Tick
        text(54, ly + 2, C_HINT, &format!("{:6}", entry.tick));

        // Tag
        text(110, ly + 2, C_SEL, &format!("[{}]", entry.tag));

        // Message
        let msg_x = 178u32;
        let max_msg = (W - msg_x - 8) as usize / CHAR_W as usize;
        let md = if entry.msg.len() > max_msg { &entry.msg[..max_msg] } else { entry.msg };
        text(msg_x, ly + 2, col, md);

        // Fatal highlight
        if entry.level == Level::Fatal {
            fill(0, ly, W, LINE_H, 0x2B0D0DFF);
            fill(8, ly + 2, 40, LINE_H - 4, C_RED);
            text(10, ly + 2, C_BG, "FATAL");
            text(54, ly + 2, C_HINT, &format!("{:6}", entry.tick));
            text(110, ly + 2, C_SEL, &format!("[{}]", entry.tag));
            text(msg_x, ly + 2, C_RED, md);
        }
    }

    // Scroll indicator
    if total_shown > lines_vis {
        let track_h = CONTENT_H;
        let thumb_h = (lines_vis as u32 * track_h / total_shown as u32).max(4);
        let thumb_y = CONTENT_Y + (scroll as u32 * (track_h - thumb_h)) / (total_shown as u32 - lines_vis as u32).max(1);
        fill(W - 6, CONTENT_Y, 6, track_h, C_HEADER);
        fill(W - 6, thumb_y, 6, thumb_h, C_HINT);
    }

    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &format!("{} total  {} shown  scroll:{}/{}  tick:{}  ring:{}/{}",
        log.entries.len(), total_shown, scroll, total_shown.saturating_sub(lines_vis), log.tick, log.entries.len(), RING));
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut log = Logger::new();

    println!("@supervisor: raise syslog");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&log);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            log.tick();
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&log);
            continue;
        }

        let filtered_len = log.filtered().len();
        let vis = (CONTENT_H / LINE_H) as usize;

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "p" | "P" => { log.paused = true; }
            "r" | "R" => { log.paused = false; log.scroll = 0; }
            "c" | "C" => { log.entries.clear(); log.scroll = 0; }
            "e" | "E" => {
                match log.export() {
                    Ok(()) => {}
                    Err(_)  => {}
                }
            }
            // Filters: \x1b[11~ = F1, \x1b[12~ = F2 … but VYOMA likely sends plain keys
            // Support both \x1b[11~ and digit shortcuts
            "\x1b[11~" | "\x1bOP" => { log.filters[0] = !log.filters[0]; }
            "\x1b[12~" | "\x1bOQ" => { log.filters[1] = !log.filters[1]; }
            "\x1b[13~" | "\x1bOR" => { log.filters[2] = !log.filters[2]; }
            "\x1b[14~" | "\x1bOS" => { log.filters[3] = !log.filters[3]; }
            "\x1b[15~"            => { log.filters[4] = !log.filters[4]; }
            // Fallback: number keys 1-5 toggle filters
            "1" => { log.filters[0] = !log.filters[0]; }
            "2" => { log.filters[1] = !log.filters[1]; }
            "3" => { log.filters[2] = !log.filters[2]; }
            "4" => { log.filters[3] = !log.filters[3]; }
            "5" => { log.filters[4] = !log.filters[4]; }
            // Scroll
            "\x1b[A" => { log.scroll = (log.scroll + 1).min(filtered_len.saturating_sub(vis)); }
            "\x1b[B" => { log.scroll = log.scroll.saturating_sub(1); }
            "\x1b[5~" => { log.scroll = (log.scroll + vis).min(filtered_len.saturating_sub(vis)); }
            "\x1b[6~" => { log.scroll = log.scroll.saturating_sub(vis); }
            "\x1b[H"  => { log.scroll = filtered_len.saturating_sub(vis); } // Home = oldest
            "\x1b[F"  => { log.scroll = 0; }                                 // End = newest
            _ => {}
        }
        draw(&log);
    }
}
