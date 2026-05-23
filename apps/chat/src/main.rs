use std::io::{self, BufRead, Write};

const W: u32 = 1100;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const INPUT_H: u32 = 36;
const LINE_H: u32 = 18;
const CHAR_W: u32 = 8;
const CONTENT_Y: u32 = HEADER_H;
const INPUT_Y: u32 = H - INPUT_H;
const VISIBLE_LINES: usize = ((INPUT_Y - CONTENT_Y) / LINE_H) as usize;
const MAX_MESSAGES: usize = 200;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_PURPLE: u32 = 0xBC8CFFFF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(seed: u64) -> u64 {
    seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

const BOTS: &[(&str, u32, u64)] = &[
    ("Alice", C_SEL,    0xA11CE_12345678AB),
    ("Bob",   C_ORANGE, 0xB0B5_ABCDEF1234),
    ("Carol", C_YELLOW, 0xCA401_5678ABCDE),
    ("Dave",  C_GREEN,  0xDA7E_FEDCBA9876),
    ("Eve",   C_PURPLE, 0xE7E_0123456789A),
];

const BOT_PHRASES: &[&str] = &[
    "Hey, how's it going?",
    "Interesting point!",
    "I totally agree.",
    "That makes sense.",
    "Have you tried rebooting?",
    "I've been thinking about that too.",
    "Let me check that out.",
    "Nice one!",
    "What do you think about this?",
    "I'm not sure I follow.",
    "That's a great idea!",
    "Can you elaborate?",
    "Haha, true!",
    "This channel is great.",
    "Anyone here have experience with WASM?",
    "VyomaOS is impressive!",
    "I should try that.",
    "Wait, really?",
    "LOL same",
    "Working on something similar actually.",
];

struct Bot {
    name:       &'static str,
    color:      u32,
    seed:       u64,
    cooldown:   u32, // ticks until next message
}

impl Bot {
    fn tick(&mut self, tick: u64, you_said: Option<&str>) -> Option<String> {
        self.seed = lcg(self.seed ^ tick);
        if self.cooldown > 0 { self.cooldown -= 1; return None; }

        // 20% chance to respond per tick when idle
        if self.seed % 5 != 0 { return None; }

        self.seed = lcg(self.seed);
        let phrase_idx = (self.seed % BOT_PHRASES.len() as u64) as usize;
        let mut msg = BOT_PHRASES[phrase_idx].to_string();

        // 20% chance to @mention "You"
        self.seed = lcg(self.seed);
        if self.seed % 5 == 0 {
            msg = format!("@You {}", msg.to_lowercase());
        }

        // If user said something, 40% chance to quote-respond
        if let Some(said) = you_said {
            self.seed = lcg(self.seed);
            if self.seed % 5 < 2 {
                let snip = if said.len() > 20 { &said[..20] } else { said };
                msg = format!("re: \"{}\" — {}", snip, msg);
            }
        }

        self.cooldown = ((self.seed % 8) + 2) as u32; // 2-9 tick cooldown
        Some(msg)
    }
}

struct ChatApp {
    messages:   Vec<(String, String, u32)>, // (username, text, color)
    input:      String,
    scroll:     usize,
    tick:       u64,
    bots:       Vec<Bot>,
    last_user_msg: Option<String>,
    pending_bot_msgs: Vec<(String, String, u32)>,
    bot_queue_tick: u64, // tick when pending bots will post
}

impl ChatApp {
    fn new() -> Self {
        let bots = BOTS.iter().map(|&(name, color, seed)| Bot { name, color, seed, cooldown: 1 }).collect();
        let mut app = ChatApp {
            messages: Vec::new(),
            input: String::new(),
            scroll: 0,
            tick: 0,
            bots,
            last_user_msg: None,
            pending_bot_msgs: Vec::new(),
            bot_queue_tick: 0,
        };
        app.push("System", "[VyomaOS Chat — 5 bots active]", C_HINT);
        app.push("Alice", "Hello! Welcome to the chat.", C_SEL);
        app.push("Bob", "Hey, glad you're here!", C_ORANGE);
        app
    }

    fn push(&mut self, user: &str, msg: &str, color: u32) {
        if self.messages.len() >= MAX_MESSAGES { self.messages.remove(0); }
        self.messages.push((user.to_string(), msg.to_string(), color));
        self.scroll = 0; // scroll to bottom on new message
    }

    fn send_user(&mut self, msg: &str) {
        if msg.is_empty() { return; }
        self.push("You", msg, C_GREEN);
        self.last_user_msg = Some(msg.to_string());
    }

    fn on_tick(&mut self) {
        self.tick += 1;
        let you = self.last_user_msg.take();
        let you_ref = you.as_deref();

        for bot in &mut self.bots {
            if let Some(msg) = bot.tick(self.tick, you_ref) {
                self.messages.push((bot.name.to_string(), msg, bot.color));
                if self.messages.len() > MAX_MESSAGES { self.messages.remove(0); }
                self.scroll = 0;
            }
        }
    }
}

fn render_message_line(x: u32, y: u32, user: &str, msg: &str, user_color: u32) {
    let max_total = (W as usize - 16) / CHAR_W as usize;
    let user_part = format!("[{}] ", user);
    let remaining = max_total.saturating_sub(user_part.len());
    let msg_display = if msg.len() > remaining { &msg[..remaining] } else { msg };

    // Highlight @You mentions in msg
    if msg_display.contains("@You") {
        text(x, y, user_color, &user_part);
        let msg_x = x + user_part.len() as u32 * CHAR_W;
        // Simple: render whole msg in orange to highlight mention
        text(msg_x, y, C_ORANGE, msg_display);
    } else {
        text(x, y, user_color, &user_part);
        let msg_x = x + user_part.len() as u32 * CHAR_W;
        let msg_col = if user == "System" { C_HINT } else { C_TEXT };
        text(msg_x, y, msg_col, msg_display);
    }
}

fn draw(app: &ChatApp) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Chat");
    text(100, 16, C_HINT, "VyomaOS multi-user simulator");
    let users = BOTS.iter().map(|&(n, _, _)| n).collect::<Vec<_>>().join(" ");
    text(400, 16, C_HINT, &format!("Users: You  {}", users));

    let total = app.messages.len();
    let scroll_clamped = app.scroll.min(total.saturating_sub(VISIBLE_LINES));
    let start = total.saturating_sub(VISIBLE_LINES + scroll_clamped);
    let end = (start + VISIBLE_LINES).min(total);

    for (i, idx) in (start..end).enumerate() {
        let (ref user, ref msg, color) = app.messages[idx];
        let y = CONTENT_Y + i as u32 * LINE_H + 2;
        render_message_line(8, y, user, msg, color);
    }

    if app.scroll > 0 {
        text(W - 130, CONTENT_Y + 4, C_HINT, &format!("[+{} msgs]", app.scroll));
    }

    // Input line
    fill(0, INPUT_Y, W, INPUT_H, C_CARD);
    fill(0, INPUT_Y, W, 1, C_BORDER);
    let prompt = "You: ";
    text(8, INPUT_Y + 10, C_GREEN, prompt);
    let cx = 8 + (prompt.len() + app.input.len()) as u32 * CHAR_W;
    let max_input = (W as usize - 8 - prompt.len() * CHAR_W as usize) / CHAR_W as usize - 2;
    let input_display = if app.input.len() > max_input { &app.input[app.input.len()-max_input..] } else { &app.input };
    text(8 + prompt.len() as u32 * CHAR_W, INPUT_Y + 10, C_TEXT, input_display);
    fill(cx, INPUT_Y + 8, 2, LINE_H, C_SEL);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = ChatApp::new();

    println!("@supervisor: raise chat");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            app.on_tick();
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&app);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b" => {}
            "\x1b[5~" => { app.scroll += VISIBLE_LINES / 2; }
            "\x1b[6~" => { app.scroll = app.scroll.saturating_sub(VISIBLE_LINES / 2); }
            "\x1b[H" => { app.scroll = MAX_MESSAGES; }
            "\x1b[F" => { app.scroll = 0; }
            "\x7f" => { app.input.pop(); }
            "" => {
                let msg = app.input.clone();
                app.input.clear();
                app.send_user(&msg);
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x20 && b < 0x7f { app.input.push(b as char); }
            }
            _ => {}
        }
        draw(&app);
    }
}
