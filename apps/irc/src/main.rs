use std::io::{self, BufRead, Write};

const W: u32 = 1280;
const H: u32 = 760;
const HEADER_H: u32 = 36;
const TAB_H: u32 = 28;
const INPUT_H: u32 = 32;
const STATUS_H: u32 = 24;
const NICK_W: u32 = 160;
const MSG_W: u32 = W - NICK_W - 1;
const CONTENT_Y: u32 = HEADER_H + TAB_H;
const CONTENT_H: u32 = H - HEADER_H - TAB_H - INPUT_H - STATUS_H;
const LINE_H: u32 = 16;
const CHAR_W: u32 = 8;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const CHANNELS: [&str; 3] = ["#general", "#dev", "#random"];

const NICKS: [&str; 8] = ["vyoma", "alice", "bob", "carol", "dave", "eve", "frank", "grace"];

const NICK_COLORS: [u32; 8] = [
    C_ORANGE, C_GREEN, C_SEL, C_PURPLE, C_YELLOW, C_RED, C_TEXT, C_HINT,
];

const BOT_MSGS_GENERAL: &[&str] = &[
    "hey everyone!",
    "how's it going?",
    "just pushed a fix to main",
    "anyone up for a review?",
    "great progress today!",
    "the build is passing now",
    "let me know if you need help",
    "VyomaOS is getting really nice",
    "loving the new display protocol",
    "coffee break anyone?",
];

const BOT_MSGS_DEV: &[&str] = &[
    "working on the IPC broker refactor",
    "supervisor PR is up for review",
    "found a race condition in the scheduler",
    "WASM binary size is down to 8KB",
    "added new VYOMA_DRAW commands",
    "the watchdog is working correctly",
    "kernel config slimmed down further",
    "fixed the LCG seed overflow issue",
    "new app: spreadsheet2 compiles clean",
    "running integration tests now",
];

const BOT_MSGS_RANDOM: &[&str] = &[
    "anyone else think WASM is the future?",
    "just had the best coffee",
    "it's raining here",
    "working from home today",
    "weekend project: retro game in WASM",
    "VyomaOS could run on a Pi!",
    "minimal kernels are so satisfying",
    "Rust + WASM = perfect combo",
    "100 apps milestone soon!",
    "let's ship it 🚀",
];

#[derive(Clone)]
struct Message {
    nick_idx: usize,
    text: String,
    tick: u64,
}

struct App {
    channels:   [Vec<Message>; 3],
    chan:        usize,
    scroll:      [usize; 3],
    input:       String,
    status:      String,
    tick:        u64,
    seed:        u64,
    me:          usize, // nick index for "you"
}

impl App {
    fn new() -> Self {
        let mut a = App {
            channels: [Vec::new(), Vec::new(), Vec::new()],
            chan: 0,
            scroll: [0; 3],
            input: String::new(),
            status: String::from("Connected to VyomaIRC · /msg <text>  /join <#chan>  /quit"),
            tick: 0,
            seed: 0xA0C5E_12345678ABu64,
            me: 0,
        };
        // Pre-populate each channel with a few messages
        let seeds: [(usize, &str); 9] = [
            (1, "hey everyone!"),
            (2, "just pushed a fix to main"),
            (3, "anyone up for a review?"),
            (4, "working on the IPC broker refactor"),
            (5, "supervisor PR is up for review"),
            (6, "WASM binary size is down to 8KB"),
            (7, "anyone else think WASM is the future?"),
            (2, "just had the best coffee"),
            (3, "weekend project: retro game in WASM"),
        ];
        let per_channel = [
            &seeds[0..3],
            &seeds[3..6],
            &seeds[6..9],
        ];
        for (ci, msgs) in per_channel.iter().enumerate() {
            for (ni, txt) in *msgs {
                a.channels[ci].push(Message { nick_idx: *ni, text: txt.to_string(), tick: 0 });
            }
        }
        a
    }

    fn bot_tick(&mut self) {
        self.tick += 1;
        self.seed = lcg(self.seed);
        // Maybe post to one channel
        if self.seed % 3 == 0 {
            self.seed = lcg(self.seed);
            let ch = (self.seed as usize) % 3;
            self.seed = lcg(self.seed);
            let nick_idx = ((self.seed as usize) % (NICKS.len() - 1)) + 1; // skip index 0 = "you"
            self.seed = lcg(self.seed);
            let msgs: &[&str] = match ch {
                0 => BOT_MSGS_GENERAL,
                1 => BOT_MSGS_DEV,
                _ => BOT_MSGS_RANDOM,
            };
            let msg_idx = (self.seed as usize) % msgs.len();
            let msg = Message {
                nick_idx,
                text: msgs[msg_idx].to_string(),
                tick: self.tick,
            };
            if self.channels[ch].len() >= 200 {
                self.channels[ch].remove(0);
            }
            self.channels[ch].push(msg);
            // snap to bottom if not scrolled
            if self.scroll[ch] == 0 { /* stay at bottom */ }
        }
    }

    fn send_msg(&mut self, text: String) {
        let msg = Message {
            nick_idx: self.me,
            text,
            tick: self.tick,
        };
        if self.channels[self.chan].len() >= 200 {
            self.channels[self.chan].remove(0);
        }
        self.channels[self.chan].push(msg);
        self.scroll[self.chan] = 0;
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 10, C_ORANGE, "VyomaIRC");
    text(120, 10, C_TEXT, &format!("— {} — logged in as {}", CHANNELS[app.chan], NICKS[app.me]));

    // Channel tabs
    fill(0, HEADER_H, W, TAB_H, C_CARD);
    fill(0, HEADER_H + TAB_H - 1, W, 1, C_BORDER);
    for (i, ch) in CHANNELS.iter().enumerate() {
        let tx = 8 + i as u32 * 140;
        let is_active = i == app.chan;
        if is_active {
            fill(tx - 4, HEADER_H, 136, TAB_H - 1, C_HEADER);
            text(tx, HEADER_H + 7, C_SEL, ch);
        } else {
            text(tx, HEADER_H + 7, C_HINT, ch);
        }
    }

    // Nick list divider
    fill(MSG_W, CONTENT_Y, 1, CONTENT_H, C_BORDER);

    // Nick list header
    fill(MSG_W + 1, CONTENT_Y, NICK_W, LINE_H + 2, C_HEADER);
    text(MSG_W + 8, CONTENT_Y + 3, C_HINT, "Online");

    // Nick list entries
    for (i, (nick, col)) in NICKS.iter().zip(NICK_COLORS.iter()).enumerate() {
        let ny = CONTENT_Y + LINE_H + 4 + i as u32 * LINE_H;
        if ny + LINE_H > CONTENT_Y + CONTENT_H { break; }
        let col = if i == app.me { C_GREEN } else { *col };
        let prefix = if i == app.me { "● " } else { "○ " };
        text(MSG_W + 8, ny, col, &format!("{}{}", prefix, nick));
    }

    // Messages area
    let messages = &app.channels[app.chan];
    let lines_vis = (CONTENT_H / LINE_H) as usize;
    let scroll = app.scroll[app.chan].min(messages.len().saturating_sub(lines_vis));
    let start = messages.len().saturating_sub(lines_vis + scroll);
    let end = messages.len().saturating_sub(scroll);

    fill(0, CONTENT_Y, MSG_W, CONTENT_H, C_BG);

    for (vi, msg) in messages[start..end].iter().enumerate() {
        let ly = CONTENT_Y + vi as u32 * LINE_H;
        let nc = NICK_COLORS[msg.nick_idx % NICK_COLORS.len()];
        let nick = NICKS[msg.nick_idx % NICKS.len()];
        let prefix = format!("<{}> ", nick);
        let px = 8u32;
        let nick_w = prefix.len() as u32 * CHAR_W;
        text(px, ly + 2, nc, &prefix);
        let max_chars = (MSG_W - nick_w - px - 8) as usize / CHAR_W as usize;
        let txt = if msg.text.len() > max_chars { &msg.text[..max_chars] } else { &msg.text };
        let msg_col = if msg.nick_idx == app.me { C_GREEN } else { C_TEXT };
        text(px + nick_w, ly + 2, msg_col, txt);
    }

    // Input bar
    let input_y = CONTENT_Y + CONTENT_H;
    fill(0, input_y, W, INPUT_H, C_CARD);
    fill(0, input_y, W, 1, C_BORDER);
    let prompt = format!("[{}] {}> ", CHANNELS[app.chan], NICKS[app.me]);
    let px = 8u32;
    text(px, input_y + 8, C_HINT, &prompt);
    let pw = prompt.len() as u32 * CHAR_W;
    text(px + pw, input_y + 8, C_TEXT, &app.input);
    // Cursor
    let cx = px + pw + app.input.len() as u32 * CHAR_W;
    fill(cx, input_y + 6, 2, 18, C_SEL);

    // Status bar
    let sb_y = input_y + INPUT_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 4, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise irc");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            app.bot_tick();
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&app);
            continue;
        }

        let lines_vis = (CONTENT_H / LINE_H) as usize;
        let total = app.channels[app.chan].len();

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x7f" => { app.input.pop(); }
            "" => {
                // Enter: process command or send
                let input = app.input.trim().to_string();
                app.input.clear();
                if input.is_empty() { draw(&app); continue; }
                if input.starts_with("/quit") {
                    fill(0, 0, W, H, C_BG); flush(); std::process::exit(0);
                } else if let Some(rest) = input.strip_prefix("/join ") {
                    let ch = rest.trim();
                    if let Some(idx) = CHANNELS.iter().position(|c| *c == ch) {
                        app.chan = idx;
                        app.status = format!("Joined {}", CHANNELS[app.chan]);
                    } else {
                        app.status = format!("Unknown channel: {}", ch);
                    }
                } else if let Some(rest) = input.strip_prefix("/msg ") {
                    app.send_msg(rest.to_string());
                    app.status = format!("→ {} | {} msgs in {}", rest, app.channels[app.chan].len(), CHANNELS[app.chan]);
                } else if input.starts_with('/') {
                    app.status = format!("Unknown command: {}", input);
                } else {
                    app.send_msg(input.clone());
                    app.status = format!("{} msgs in {}", app.channels[app.chan].len(), CHANNELS[app.chan]);
                }
            }
            "\t" => {
                app.chan = (app.chan + 1) % CHANNELS.len();
                app.status = format!("Switched to {}", CHANNELS[app.chan]);
            }
            "\x1b[A" => {
                app.scroll[app.chan] = (app.scroll[app.chan] + 1).min(total.saturating_sub(lines_vis));
            }
            "\x1b[B" => {
                app.scroll[app.chan] = app.scroll[app.chan].saturating_sub(1);
            }
            "\x1b[5~" => {
                let pg = lines_vis / 2;
                app.scroll[app.chan] = (app.scroll[app.chan] + pg).min(total.saturating_sub(lines_vis));
            }
            "\x1b[6~" => {
                let pg = lines_vis / 2;
                app.scroll[app.chan] = app.scroll[app.chan].saturating_sub(pg);
            }
            s if s.len() == 1 && s.chars().next().map(|c| !c.is_control()).unwrap_or(false) => {
                if app.input.len() < 120 {
                    app.input.push_str(s);
                }
            }
            _ => {}
        }
        draw(&app);
    }
}
