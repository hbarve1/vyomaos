// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_YELLOW: u32 = 0xD29922FF;

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

// Node box dimensions
const NODE_W: u32 = 130;
const NODE_H: u32 = 44;

struct Person {
    name:       &'static str,
    born:       u32,
    died:       Option<u32>,
    gen:        usize,     // 0-3
    gen_idx:    usize,     // position within generation
    parent:     Option<usize>, // index into PEOPLE
}

// 20 people across 4 generations
const PEOPLE: &[Person] = &[
    // Gen 0 — founders
    Person { name: "Henry Barton",  born: 1880, died: Some(1952), gen: 0, gen_idx: 0, parent: None },
    Person { name: "Clara Morse",   born: 1884, died: Some(1960), gen: 0, gen_idx: 1, parent: None },
    // Gen 1 — children of Henry+Clara
    Person { name: "George Barton", born: 1905, died: Some(1978), gen: 1, gen_idx: 0, parent: Some(0) },
    Person { name: "Alice Barton",  born: 1908, died: Some(1991), gen: 1, gen_idx: 1, parent: Some(0) },
    Person { name: "Edward Morse",  born: 1912, died: Some(1985), gen: 1, gen_idx: 2, parent: Some(1) },
    Person { name: "Ruth Morse",    born: 1915, died: Some(2002), gen: 1, gen_idx: 3, parent: Some(1) },
    // Gen 2 — grandchildren
    Person { name: "Thomas Barton", born: 1930, died: Some(2010), gen: 2, gen_idx: 0, parent: Some(2) },
    Person { name: "Dorothy B.",    born: 1933, died: None,        gen: 2, gen_idx: 1, parent: Some(2) },
    Person { name: "James Barton",  born: 1935, died: Some(2018), gen: 2, gen_idx: 2, parent: Some(3) },
    Person { name: "Margaret B.",   born: 1938, died: None,        gen: 2, gen_idx: 3, parent: Some(3) },
    Person { name: "William Morse", born: 1937, died: Some(2015), gen: 2, gen_idx: 4, parent: Some(4) },
    Person { name: "Patricia M.",   born: 1940, died: None,        gen: 2, gen_idx: 5, parent: Some(4) },
    Person { name: "Robert Morse",  born: 1942, died: Some(2019), gen: 2, gen_idx: 6, parent: Some(5) },
    Person { name: "Helen Morse",   born: 1945, died: None,        gen: 2, gen_idx: 7, parent: Some(5) },
    // Gen 3 — great-grandchildren
    Person { name: "Lisa Barton",   born: 1958, died: None, gen: 3, gen_idx: 0, parent: Some(6) },
    Person { name: "Mark Barton",   born: 1961, died: None, gen: 3, gen_idx: 1, parent: Some(7) },
    Person { name: "Sarah B.",      born: 1964, died: None, gen: 3, gen_idx: 2, parent: Some(7) },
    Person { name: "Kevin Morse",   born: 1963, died: None, gen: 3, gen_idx: 3, parent: Some(10) },
    Person { name: "Anna Morse",    born: 1967, died: None, gen: 3, gen_idx: 4, parent: Some(11) },
    Person { name: "Paul Morse",    born: 1970, died: None, gen: 3, gen_idx: 5, parent: Some(12) },
];

// Gen sizes
const GEN_SIZES: [usize; 4] = [2, 4, 8, 6];

fn gen_count(gen: usize) -> usize { GEN_SIZES[gen] }

// Compute pixel center of a node
fn node_center(gen: usize, gen_idx: usize) -> (u32, u32) {
    let total_h = H - HEADER_H - STATUS_H;
    let gen_h = total_h / 4;
    let cy = HEADER_H + gen as u32 * gen_h + gen_h / 2;

    let n = gen_count(gen) as u32;
    let cx = (gen_idx as u32 + 1) * W / (n + 1);
    (cx, cy)
}

fn draw_node(p: &Person, cx: u32, cy: u32, selected: bool) {
    let x = cx.saturating_sub(NODE_W / 2);
    let y = cy.saturating_sub(NODE_H / 2);
    let bg = if selected { 0x1C2D4EFF } else { C_CARD };
    fill(x, y, NODE_W, NODE_H, bg);
    if selected {
        border(x - 1, y - 1, NODE_W + 2, NODE_H + 2, C_SEL);
    } else {
        border(x, y, NODE_W, NODE_H, C_BORDER);
    }

    // Name (truncated to fit)
    let max_name = (NODE_W - 8) as usize / CHAR_W as usize;
    let name_d = if p.name.len() > max_name { &p.name[..max_name] } else { p.name };
    let name_col = if selected { C_SEL } else { C_TEXT };
    text(x + 4, y + 4, name_col, name_d);

    // Years
    let years = match p.died {
        Some(d) => format!("{}-{}", p.born, d),
        None    => format!("b.{}", p.born),
    };
    let year_col = if p.died.is_none() { C_GREEN } else { C_HINT };
    text(x + 4, y + 22, year_col, &years);
}

fn draw_line(x1: u32, y1: u32, x2: u32, y2: u32) {
    // Draw vertical segment then horizontal (Manhattan routing)
    let mid_y = (y1 + y2) / 2;
    // Down from parent
    if y2 > y1 {
        fill(x1, y1, 1, mid_y - y1 + 1, C_BORDER);
        // Horizontal
        let lx = x1.min(x2);
        let rx = x1.max(x2);
        fill(lx, mid_y, rx - lx + 1, 1, C_BORDER);
        // Down to child
        fill(x2, mid_y, 1, y2 - mid_y + 1, C_BORDER);
    }
}

fn draw(selected: usize) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Genealogy Tree");
    text(220, 16, C_HINT, "The Barton-Morse Family — 4 Generations");
    text(620, 16, C_HINT, "←→:siblings  ↑↓:generations  Ctrl+C:exit");

    // Generation labels
    let gen_labels = ["Generation 0", "Generation 1", "Generation 2", "Generation 3"];
    let gen_colors = [C_ORANGE, C_YELLOW, C_GREEN, C_SEL];
    let total_h = H - HEADER_H - STATUS_H;
    let gen_h = total_h / 4;
    for g in 0..4 {
        let gy = HEADER_H + g as u32 * gen_h + 4;
        text(4, gy, gen_colors[g], gen_labels[g]);
    }

    // Draw connecting lines (parent → children)
    for (i, p) in PEOPLE.iter().enumerate() {
        if let Some(par_idx) = p.parent {
            let par = &PEOPLE[par_idx];
            let (px, py) = node_center(par.gen, par.gen_idx);
            let (cx, cy) = node_center(p.gen, p.gen_idx);
            // Line from bottom of parent to top of child
            draw_line(px, py + NODE_H / 2, cx, cy.saturating_sub(NODE_H / 2));
            let _ = i;
        }
    }

    // Draw all nodes
    for (i, p) in PEOPLE.iter().enumerate() {
        let (cx, cy) = node_center(p.gen, p.gen_idx);
        draw_node(p, cx, cy, i == selected);
    }

    // Status bar
    let p = &PEOPLE[selected];
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let died_str = match p.died {
        Some(y) => format!("died {}", y),
        None    => "living".to_string(),
    };
    let parent_str = match p.parent {
        Some(i) => format!("  |  child of: {}", PEOPLE[i].name),
        None    => String::new(),
    };
    text(8, sb_y + 6, C_HINT, &format!("{}  |  born {}  |  {}  |  Gen {}{}",
        p.name, p.born, died_str, p.gen, parent_str));
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut selected = 0usize;

    println!("@supervisor: raise genealogy");
    let _ = io::stdout().flush();
    draw(selected);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        let cur = &PEOPLE[selected];
        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[C" => {
                // Move right within same generation
                let next_idx = cur.gen_idx + 1;
                if let Some(p) = PEOPLE.iter().position(|p| p.gen == cur.gen && p.gen_idx == next_idx) {
                    selected = p;
                }
            }
            "\x1b[D" => {
                // Move left within same generation
                if cur.gen_idx > 0 {
                    let prev_idx = cur.gen_idx - 1;
                    if let Some(p) = PEOPLE.iter().position(|p| p.gen == cur.gen && p.gen_idx == prev_idx) {
                        selected = p;
                    }
                }
            }
            "\x1b[A" => {
                // Move up — go to parent
                if let Some(par) = cur.parent {
                    selected = par;
                }
            }
            "\x1b[B" => {
                // Move down — go to first child
                if let Some(c) = PEOPLE.iter().position(|p| p.parent == Some(selected)) {
                    selected = c;
                }
            }
            _ => {}
        }
        draw(selected);
    }
}
