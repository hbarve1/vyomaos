// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_PURPLE: u32 = 0xBC8CFFFF;
const C_GREEN: u32  = 0x3FB950FF;
const C_RED: u32    = 0xFF7B72FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

struct TarotCard {
    name:     &'static str,
    symbol:   &'static str,
    upright:  &'static str,
    reversed: &'static str,
}

const CARDS: &[TarotCard] = &[
    TarotCard { name: "The Fool",         symbol: "0",    upright: "New beginnings, innocence, spontaneity, free spirit", reversed: "Recklessness, taken advantage of, inconsideration" },
    TarotCard { name: "The Magician",     symbol: "I",    upright: "Manifestation, resourcefulness, power, inspired action", reversed: "Poor planning, untapped talents, manipulation" },
    TarotCard { name: "High Priestess",   symbol: "II",   upright: "Intuition, sacred knowledge, divine feminine, subconscious", reversed: "Secrets, disconnected from intuition, withdrawal" },
    TarotCard { name: "The Empress",      symbol: "III",  upright: "Femininity, beauty, nature, nurturing, abundance", reversed: "Creative block, dependence on others, smothering" },
    TarotCard { name: "The Emperor",      symbol: "IV",   upright: "Authority, establishment, structure, a father figure", reversed: "Domination, excessive control, rigidity, stubbornness" },
    TarotCard { name: "The Hierophant",   symbol: "V",    upright: "Spiritual wisdom, religious beliefs, conformity, tradition", reversed: "Personal beliefs, freedom, challenging the status quo" },
    TarotCard { name: "The Lovers",       symbol: "VI",   upright: "Love, harmony, relationships, values alignment, choices", reversed: "Self-love, disharmony, imbalance, misaligned values" },
    TarotCard { name: "The Chariot",      symbol: "VII",  upright: "Control, willpower, success, action, determination", reversed: "Self-discipline, opposition, lack of direction" },
    TarotCard { name: "Strength",         symbol: "VIII", upright: "Inner strength, bravery, compassion, focus, patience", reversed: "Self doubt, weakness, insecurity, low energy" },
    TarotCard { name: "The Hermit",       symbol: "IX",   upright: "Soul-searching, introspection, inner guidance, solitude", reversed: "Isolation, loneliness, withdrawal, lost your way" },
    TarotCard { name: "Wheel of Fortune", symbol: "X",    upright: "Good luck, karma, life cycles, destiny, a turning point", reversed: "Bad luck, resistance to change, breaking cycles" },
    TarotCard { name: "Justice",          symbol: "XI",   upright: "Justice, fairness, truth, cause and effect, law", reversed: "Unfairness, lack of accountability, dishonesty" },
    TarotCard { name: "The Hanged Man",   symbol: "XII",  upright: "Pause, surrender, letting go, new perspectives", reversed: "Delays, resistance, stalling, indecision" },
    TarotCard { name: "Death",            symbol: "XIII", upright: "Endings, change, transformation, transition", reversed: "Resistance to change, personal transformation, inner purging" },
    TarotCard { name: "Temperance",       symbol: "XIV",  upright: "Balance, moderation, patience, purpose, meaning", reversed: "Imbalance, excess, self-healing, realignment" },
    TarotCard { name: "The Devil",        symbol: "XV",   upright: "Shadow self, attachment, addiction, restriction", reversed: "Releasing limiting beliefs, exploring dark thoughts, detachment" },
    TarotCard { name: "The Tower",        symbol: "XVI",  upright: "Sudden change, upheaval, chaos, revelation, awakening", reversed: "Personal transformation, fear of change, averting disaster" },
    TarotCard { name: "The Star",         symbol: "XVII", upright: "Hope, faith, purpose, renewal, spirituality", reversed: "Lack of faith, despair, self-trust, disconnection" },
    TarotCard { name: "The Moon",         symbol: "XVIII",upright: "Illusion, fear, the unconscious, intuition, confusion", reversed: "Release of fear, repressed emotion, inner confusion" },
    TarotCard { name: "The Sun",          symbol: "XIX",  upright: "Positivity, fun, warmth, success, vitality", reversed: "Inner child, feeling down, overly optimistic" },
    TarotCard { name: "Judgement",        symbol: "XX",   upright: "Judgement, rebirth, inner calling, absolution", reversed: "Self-doubt, inner critic, ignoring the call, indecisiveness" },
    TarotCard { name: "The World",        symbol: "XXI",  upright: "Completion, integration, accomplishment, travel", reversed: "Seeking closure, short-cuts, delays, incomplete goals" },
];

const POSITIONS: &[&str] = &["Past", "Present", "Future"];

struct Spread {
    cards:    [usize; 3],
    reversed: [bool; 3],
}

struct App {
    seed:   u64,
    spread: Spread,
    sel:    usize,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            seed: 0xC0DE_CAFE_DEAD_BEEFu64,
            spread: Spread { cards: [0, 7, 14], reversed: [false, false, false] },
            sel: 0,
        };
        a.new_reading();
        a
    }

    fn new_reading(&mut self) {
        let mut picked = [22usize; 3];
        for i in 0..3 {
            loop {
                self.seed = lcg(self.seed);
                let c = (self.seed >> 32) as usize % CARDS.len();
                if !picked[..i].contains(&c) {
                    picked[i] = c;
                    break;
                }
            }
            self.seed = lcg(self.seed);
            self.spread.reversed[i] = (self.seed >> 63) == 1;
        }
        self.spread.cards = picked;
        self.sel = 0;
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Tarot Card Reader");
        text(210, 8, C_HINT, "Left/Right:select  N:new reading  Q:quit");

        let card_w = 200i32;
        let card_h = 320i32;
        let spacing = 40i32;
        let total = 3 * card_w + 2 * spacing;
        let start_x = (W - total) / 2;
        let card_y = 52i32;

        for i in 0..3usize {
            let cx = start_x + i as i32 * (card_w + spacing);
            let ci = self.spread.cards[i];
            let rev = self.spread.reversed[i];
            let card = &CARDS[ci];
            let selected = i == self.sel;

            let bg = if selected { 0x1A1A3AFF } else { C_CARD };
            fill(cx, card_y, card_w, card_h, bg);
            let bc = if selected { C_SEL } else { C_BORDER };
            border(cx, card_y, card_w, card_h, bc);

            fill(cx + 8, card_y + 8, card_w - 16, card_h - 16, 0x0A0A14FF);
            border(cx + 8, card_y + 8, card_w - 16, card_h - 16, C_BORDER);

            let sym_x = cx + (card_w - card.symbol.len() as i32 * 8) / 2;
            text(sym_x, card_y + 24, C_PURPLE, card.symbol);

            let pos_tc = [C_ORANGE, C_SEL, C_GREEN][i];
            let pl = POSITIONS[i].len() as i32;
            text(cx + (card_w - pl * 8) / 2, card_y + 44, pos_tc, POSITIONS[i]);

            let (ori_c, ori_str) = if rev { (C_RED, "Reversed") } else { (C_GREEN, "Upright ") };
            text(cx + 12, card_y + 64, ori_c, ori_str);

            // Decorative dots
            for di in 0..5i32 {
                fill(cx + card_w / 2 - 20 + di * 10, card_y + 92, 4, 4, C_PURPLE);
            }
            fill(cx + card_w / 2 - 2, card_y + 104, 4, 4, C_PURPLE);

            // Card name
            let name_x = cx + (card_w - card.name.len() as i32 * 8).max(4) / 2;
            text(name_x.max(cx + 4), card_y + 128, C_TEXT, card.name);

            // First keyword snippet
            let meaning = if rev { card.reversed } else { card.upright };
            let kw: &str = &meaning[..meaning.len().min(24)];
            text(cx + 8, card_y + card_h - 40, C_HINT, kw);

            // Bottom accent line
            fill(cx + 16, card_y + card_h - 20, card_w - 32, 2, pos_tc);
        }

        // Detail panel for selected card
        let si = self.spread.cards[self.sel];
        let srev = self.spread.reversed[self.sel];
        let scard = &CARDS[si];
        let meaning = if srev { scard.reversed } else { scard.upright };

        let panel_y = card_y + card_h + 16;
        let panel_h = H - panel_y - 28;
        fill(20, panel_y, W - 40, panel_h, C_CARD);
        border(20, panel_y, W - 40, panel_h, C_BORDER);

        let pos_tc = [C_ORANGE, C_SEL, C_GREEN][self.sel];
        text(32, panel_y + 10, pos_tc, &format!("{} — {}", POSITIONS[self.sel], scard.name));
        let ori_c = if srev { C_RED } else { C_GREEN };
        let ori_str = if srev { "(Reversed)" } else { "(Upright)" };
        let name_end_x = 32 + scard.name.len() as i32 * 8 + 80;
        text(name_end_x, panel_y + 10, ori_c, ori_str);

        // Word-wrap meaning
        let mut mx = 32i32;
        let mut my = panel_y + 30;
        for word in meaning.split_whitespace() {
            let wl = word.len() as i32 * 8;
            if mx + wl > W - 44 { mx = 32; my += 18; }
            text(mx, my, C_TEXT, word);
            mx += wl + 8;
        }

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!("Card {}/3: {}  |  N=new reading", self.sel + 1, scard.name));
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[D" => { if self.sel > 0 { self.sel -= 1; } }
            "\x1b[C" => { if self.sel < 2 { self.sel += 1; } }
            "n" | "N" => self.new_reading(),
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
