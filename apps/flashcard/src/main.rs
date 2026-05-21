use std::io::{self, BufRead, Write};

const W: u32 = 800;
const H: u32 = 600;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_RED: u32     = 0xFF7B72FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD_BG: u32 = 0x161B22FF;
const C_CARD_FRONT: u32 = 0x21262DFF;

const CARD_X: u32 = 60;
const CARD_Y: u32 = 110;
const CARD_W: u32 = W - 120;
const CARD_H: u32 = 340;

const CARDS: [(&str, &str); 20] = [
    ("What does CPU stand for?", "Central Processing Unit"),
    ("What is the time complexity of binary search?", "O(log n)"),
    ("What does HTTP stand for?", "HyperText Transfer Protocol"),
    ("In Rust, what keyword creates an immutable binding?", "let (bindings are immutable by default)"),
    ("What is a closure?", "An anonymous function that captures its environment"),
    ("What does DNS stand for?", "Domain Name System"),
    ("What is the purpose of a Makefile?", "To automate build tasks by defining rules and dependencies"),
    ("What is a race condition?", "When program behavior depends on the sequence of uncontrollable events"),
    ("What does WASM stand for?", "WebAssembly"),
    ("What is a stack overflow?", "When the call stack exceeds its limit, usually due to infinite recursion"),
    ("What is idempotency?", "An operation that produces the same result no matter how many times it runs"),
    ("What does POSIX stand for?", "Portable Operating System Interface"),
    ("What is a semaphore?", "A synchronization primitive used to control access to shared resources"),
    ("What is the difference between TCP and UDP?", "TCP guarantees delivery and order; UDP is faster but unreliable"),
    ("What is a garbage collector?", "A runtime system that automatically recycles unused memory"),
    ("What does SOLID stand for (first letter)?", "Single Responsibility Principle"),
    ("What is memoization?", "Caching function results to avoid redundant computation"),
    ("What is the kernel?", "The core of an operating system managing hardware and resources"),
    ("What is a mutex?", "A mutual exclusion lock allowing only one thread at a time"),
    ("What is endianness?", "The byte order used to represent multi-byte values in memory"),
];

fn lcg(seed: u64) -> u64 {
    seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

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

fn draw_wrapped(x: u32, y: u32, max_w: u32, rgba: u32, s: &str) -> u32 {
    let chars_per_line = (max_w / 9).max(1) as usize;
    let mut ly = y;
    let words: Vec<&str> = s.split_whitespace().collect();
    let mut line = String::new();
    for word in &words {
        if line.len() + word.len() + 1 > chars_per_line {
            if !line.is_empty() {
                text(x, ly, rgba, &line);
                ly += 22;
                line.clear();
            }
        }
        if !line.is_empty() { line.push(' '); }
        line.push_str(word);
    }
    if !line.is_empty() { text(x, ly, rgba, &line); ly += 22; }
    ly
}

struct Game {
    order:    [usize; 20],
    pos:      usize,
    flipped:  bool,
    correct:  u32,
    wrong:    u32,
    answered: bool,
    seed:     u64,
}

impl Game {
    fn new(seed: u64) -> Self {
        let mut g = Game {
            order: [0usize; 20],
            pos: 0,
            flipped: false,
            correct: 0,
            wrong: 0,
            answered: false,
            seed,
        };
        for i in 0..20 { g.order[i] = i; }
        g.shuffle();
        g
    }

    fn shuffle(&mut self) {
        for i in (1..20).rev() {
            self.seed = lcg(self.seed);
            let j = (self.seed >> 33) as usize % (i + 1);
            self.order.swap(i, j);
        }
    }

    fn card(&self) -> usize { self.order[self.pos] }

    fn flip(&mut self) {
        self.flipped = !self.flipped;
    }

    fn next(&mut self) {
        if self.pos + 1 < 20 { self.pos += 1; self.flipped = false; self.answered = false; }
    }

    fn prev(&mut self) {
        if self.pos > 0 { self.pos -= 1; self.flipped = false; self.answered = false; }
    }

    fn mark(&mut self, correct: bool) {
        if !self.flipped || self.answered { return; }
        if correct { self.correct += 1; } else { self.wrong += 1; }
        self.answered = true;
    }
}

fn draw(g: &Game) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Flashcards");
    text(160, 16, C_HINT, &format!("{}/{}", g.pos + 1, 20));
    text(240, 16, C_GREEN, &format!("Correct: {}", g.correct));
    text(380, 16, C_RED, &format!("Wrong: {}", g.wrong));
    text(500, 16, C_HINT, "Enter:flip  ←→:nav  y/n:mark  R:shuffle");

    // Card
    let card_bg = if g.flipped { C_CARD_FRONT } else { C_CARD_BG };
    fill(CARD_X, CARD_Y, CARD_W, CARD_H, card_bg);
    border(CARD_X, CARD_Y, CARD_W, CARD_H, if g.flipped { C_SEL } else { C_BORDER });

    let ci = g.card();
    if g.flipped {
        text(CARD_X + 20, CARD_Y + 20, C_HINT, "ANSWER");
        fill(CARD_X + 20, CARD_Y + 40, CARD_W - 40, 2, C_BORDER);
        draw_wrapped(CARD_X + 20, CARD_Y + 56, CARD_W - 40, C_GREEN, CARDS[ci].1);
    } else {
        text(CARD_X + 20, CARD_Y + 20, C_HINT, "QUESTION");
        fill(CARD_X + 20, CARD_Y + 40, CARD_W - 40, 2, C_BORDER);
        draw_wrapped(CARD_X + 20, CARD_Y + 56, CARD_W - 40, C_TEXT, CARDS[ci].0);
    }

    // Hint at bottom
    let hint_y = CARD_Y + CARD_H + 20;
    if !g.flipped {
        text(CARD_X, hint_y, C_HINT, "Press Enter to reveal answer");
    } else if !g.answered {
        text(CARD_X, hint_y, C_HINT, "Press 'y' if correct, 'n' if wrong");
    } else {
        text(CARD_X, hint_y, C_GREEN, "Marked! Press → for next card");
    }

    // Progress bar
    let bar_y = H - 24;
    fill(CARD_X, bar_y, CARD_W, 8, C_BORDER);
    let done_w = CARD_W * (g.pos + 1) as u32 / 20;
    fill(CARD_X, bar_y, done_w, 8, C_GREEN);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut seed: u64 = 0xCAFEBABE12345678;
    let mut game = Game::new(seed);

    println!("@supervisor: raise flashcard");
    let _ = io::stdout().flush();
    draw(&game);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            "\r" | "" => { game.flip(); }
            "\x1b[C" => { game.next(); }
            "\x1b[D" => { game.prev(); }
            "y" | "Y" => { game.mark(true); }
            "n" | "N" => { game.mark(false); }
            "r" | "R" => { seed = lcg(seed); game = Game::new(seed); }
            _ => {}
        }
        draw(&game);
    }
}
