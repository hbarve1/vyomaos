use std::io::{self, BufRead, Write};

const W: u32 = 760;
const H: u32 = 560;
const HEADER_H: u32 = 36;
const SEARCH_H: u32 = 36;
const TABS_H: u32   = 32;
const GRID_Y: u32   = HEADER_H + SEARCH_H + TABS_H;
const CELL_W: u32   = 56;
const CELL_H: u32   = 56;
const COLS: u32     = W / CELL_W;
const STATUS_H: u32 = 28;

const C_BG: u32     = 0x161B22FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_SEL_BG: u32 = 0x1F4068FF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;

const CATEGORIES: &[(&str, &[(&str, &str)])] = &[
    ("Smileys", &[
        ("😀","grinning"), ("😂","laughing"), ("🥰","love"), ("😎","cool"), ("🤔","thinking"),
        ("😅","sweat smile"), ("😭","crying"), ("🤣","rolling"), ("😍","heart eyes"), ("🥹","holding tears"),
        ("😤","triumph"), ("🙄","rolling eyes"), ("😱","screaming"), ("🤯","mind blown"), ("🥳","party"),
    ]),
    ("Objects", &[
        ("💻","laptop"), ("📱","phone"), ("⌨️","keyboard"), ("🖥️","desktop"), ("🖱️","mouse"),
        ("💾","floppy"), ("📀","disc"), ("🔧","wrench"), ("⚙️","gear"), ("🔑","key"),
        ("📖","book"), ("📝","memo"), ("✉️","envelope"), ("📦","package"), ("🗂️","folder"),
    ]),
    ("Symbols", &[
        ("✅","check"), ("❌","cross"), ("⚡","lightning"), ("🔥","fire"), ("💡","bulb"),
        ("⭐","star"), ("🌟","glowing star"), ("💎","gem"), ("🏆","trophy"), ("🎯","target"),
        ("❤️","heart"), ("💙","blue heart"), ("💚","green heart"), ("🟢","green circle"), ("🔵","blue circle"),
    ]),
    ("Nature", &[
        ("🌙","moon"), ("☀️","sun"), ("🌈","rainbow"), ("⛅","partly cloudy"), ("❄️","snowflake"),
        ("🌊","wave"), ("🌸","cherry blossom"), ("🍀","four leaf"), ("🌲","tree"), ("🌺","hibiscus"),
        ("🦊","fox"), ("🐉","dragon"), ("🦋","butterfly"), ("🐱","cat"), ("🐶","dog"),
    ]),
    ("Food", &[
        ("🍕","pizza"), ("🍔","burger"), ("🍜","noodles"), ("🍣","sushi"), ("🍎","apple"),
        ("🍊","tangerine"), ("🍇","grapes"), ("🫐","blueberries"), ("🥑","avocado"), ("🍦","ice cream"),
        ("☕","coffee"), ("🧃","juice"), ("🍺","beer"), ("🧁","cupcake"), ("🎂","birthday cake"),
    ]),
    ("Travel", &[
        ("🚀","rocket"), ("✈️","airplane"), ("🚂","train"), ("🚗","car"), ("🛸","flying saucer"),
        ("🏠","house"), ("🏙️","cityscape"), ("🗼","tokyo tower"), ("🗽","liberty"), ("🌍","earth"),
        ("🏖️","beach"), ("🏔️","mountain"), ("🗺️","map"), ("⛺","tent"), ("🚢","ship"),
    ]),
];

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

fn filtered_emojis<'a>(cat_idx: usize, query: &str) -> Vec<(&'a str, &'a str)> {
    let q = query.to_ascii_lowercase();
    let source: Vec<(&str, &str)> = if q.is_empty() {
        CATEGORIES[cat_idx].1.iter().map(|&(e, n)| (e, n)).collect()
    } else {
        CATEGORIES.iter().flat_map(|(_, items)| {
            items.iter().filter_map(|&(e, n)| {
                if n.contains(q.as_str()) || e.contains(&query) { Some((e, n)) } else { None }
            })
        }).collect()
    };
    source
}

fn draw(cat_idx: usize, cursor: usize, query: &str, copied: Option<&str>) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    text(16, 10, C_TEXT, "Emoji Picker");
    text(W - 220, 10, C_HINT, "Enter: copy  Tab: category  Esc: close");

    // Search bar
    fill(0, HEADER_H, W, SEARCH_H, 0x0D1117FF);
    fill(8, HEADER_H + 6, W - 16, SEARCH_H - 12, 0x21262DFF);
    border(8, HEADER_H + 6, W - 16, SEARCH_H - 12, C_BORDER);
    let search_text = if query.is_empty() { "  Search emoji…".to_string() } else { format!("  {}_", query) };
    let tc = if query.is_empty() { C_HINT } else { C_TEXT };
    text(16, HEADER_H + 11, tc, &search_text);

    // Category tabs
    fill(0, HEADER_H + SEARCH_H, W, TABS_H, 0x0D1117FF);
    fill(0, HEADER_H + SEARCH_H + TABS_H, W, 1, C_BORDER);
    let tab_w = W / CATEGORIES.len() as u32;
    for (i, &(name, _)) in CATEGORIES.iter().enumerate() {
        let tx = i as u32 * tab_w;
        let is_sel = i == cat_idx && query.is_empty();
        if is_sel { fill(tx, HEADER_H + SEARCH_H, tab_w, TABS_H, C_SEL_BG); }
        let tc = if is_sel { C_SEL } else { C_DIM };
        let nw = name.len() as u32 * 8;
        text(tx + (tab_w - nw.min(tab_w)) / 2, HEADER_H + SEARCH_H + 8, tc, name);
    }

    // Emoji grid
    let emojis = filtered_emojis(cat_idx, query);
    let grid_h = H - GRID_Y - STATUS_H;
    let visible_rows = (grid_h / CELL_H) as usize;
    let scroll = if cursor >= COLS as usize * visible_rows {
        (cursor / COLS as usize) - visible_rows + 1
    } else { 0 };

    for (i, &(emoji, name)) in emojis.iter().enumerate() {
        let col = i as u32 % COLS;
        let row = i as u32 / COLS;
        if (row as usize) < scroll { continue; }
        if (row as usize) >= scroll + visible_rows { break; }

        let ex = col * CELL_W;
        let ey = GRID_Y + (row as u32 - scroll as u32) * CELL_H;
        let is_sel = i == cursor;

        if is_sel {
            fill(ex, ey, CELL_W, CELL_H, C_SEL_BG);
            border(ex, ey, CELL_W, CELL_H, C_SEL);
        }

        // Emoji char
        println!("VYOMA_DRAW:draw_text:{},{},{:#010x},m,{emoji}", ex + 14, ey + 18, C_TEXT);

        // Tooltip on selected
        if is_sel {
            let nw = name.len() as u32 * 8 + 8;
            fill(ex, ey + CELL_H + 2, nw, 16, C_HEADER);
            text(ex + 4, ey + CELL_H + 4, C_HINT, name);
        }
    }

    if emojis.is_empty() {
        text(16, GRID_Y + 20, C_HINT, "No emoji match the search");
    }

    // Status bar
    fill(0, H - STATUS_H, W, STATUS_H, C_HEADER);
    fill(0, H - STATUS_H, W, 1, C_BORDER);
    if let Some(emoji) = copied {
        let msg = format!("{emoji} copied to clipboard!");
        text(16, H - STATUS_H + 8, C_GREEN, &msg);
    } else {
        let total = format!("{} emoji", emojis.len());
        text(16, H - STATUS_H + 8, C_HINT, &total);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut cat_idx = 0usize;
    let mut cursor  = 0usize;
    let mut query   = String::new();
    let mut copied: Option<String> = None;

    println!("@supervisor: raise emoji-picker");
    let _ = io::stdout().flush();

    draw(cat_idx, cursor, &query, None);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        let emojis = filtered_emojis(cat_idx, &query);
        let n = emojis.len();

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x09" => { // Tab
                if query.is_empty() {
                    cat_idx = (cat_idx + 1) % CATEGORIES.len();
                    cursor = 0;
                }
            }
            "\x7f" => {
                query.pop();
                cursor = 0;
                copied = None;
            }
            "\x1b[A" => { if cursor >= COLS as usize { cursor -= COLS as usize; } }
            "\x1b[B" => { if cursor + COLS as usize < n { cursor += COLS as usize; } }
            "\x1b[D" => { if cursor > 0 { cursor -= 1; } }
            "\x1b[C" => { if cursor + 1 < n { cursor += 1; } }
            "" => {
                if let Some(&(emoji, _)) = emojis.get(cursor) {
                    println!("@supervisor: clipboard-set {emoji}");
                    let _ = io::stdout().flush();
                    copied = Some(emoji.to_string());
                }
            }
            ch if ch.len() >= 1 && ch.chars().next().map(|c| c.is_ascii_graphic() || c == ' ').unwrap_or(false) => {
                query.push_str(ch);
                cursor = 0;
                copied = None;
            }
            _ => {}
        }

        draw(cat_idx, cursor, &query, copied.as_deref());
    }
}
