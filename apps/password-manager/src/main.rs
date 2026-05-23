use std::io::{self, BufRead, Write};

const W: u32 = 960;
const H: u32 = 640;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x1F4068FF;
const C_OK: u32     = 0x3FB950FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ERR: u32    = 0xFF7B72FF;
const C_FIELD: u32  = 0x21262DFF;

const VAULT_PATH: &str = "/data/vault.enc";

const LIST_Y: u32  = 52;
const ROW_H: u32   = 22;
const VIS_ROWS: usize = ((H - LIST_Y - 36) / ROW_H) as usize;

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

// XOR cipher — each byte XOR'd against repeating key bytes
fn xor_cipher(data: &[u8], key: &[u8]) -> Vec<u8> {
    if key.is_empty() { return data.to_vec(); }
    data.iter().enumerate().map(|(i, &b)| b ^ key[i % key.len()]).collect()
}

#[derive(Clone, Default)]
struct Entry {
    site: String,
    user: String,
    pass: String,
}

fn serialize(entries: &[Entry]) -> String {
    entries.iter().map(|e| {
        format!("{}|{}|{}", e.site.replace('|', "\\|"), e.user.replace('|', "\\|"), e.pass.replace('|', "\\|"))
    }).collect::<Vec<_>>().join("\n")
}

fn deserialize(raw: &str) -> Vec<Entry> {
    raw.lines().filter(|l| !l.is_empty()).map(|l| {
        let parts: Vec<&str> = l.splitn(3, '|').collect();
        Entry {
            site: parts.first().unwrap_or(&"").replace("\\|", "|"),
            user: parts.get(1).unwrap_or(&"").replace("\\|", "|"),
            pass: parts.get(2).unwrap_or(&"").replace("\\|", "|"),
        }
    }).collect()
}

fn load_vault(master_key: &str) -> Vec<Entry> {
    let Ok(data) = std::fs::read(VAULT_PATH) else { return Vec::new() };
    let decrypted = xor_cipher(&data, master_key.as_bytes());
    let s = String::from_utf8_lossy(&decrypted);
    deserialize(&s)
}

fn save_vault(entries: &[Entry], master_key: &str) -> Result<(), String> {
    let plain = serialize(entries);
    let encrypted = xor_cipher(plain.as_bytes(), master_key.as_bytes());
    std::fs::write(VAULT_PATH, &encrypted).map_err(|e| format!("{e}"))
}

fn mask(s: &str) -> String {
    "*".repeat(s.len())
}

enum Mode {
    Lock { input: String, error: String },
    List { scroll: usize, cursor: usize, status: String },
    Add  { field: usize, values: [String; 3] },
    // Confirm delete for safety
    ConfirmDelete { target_idx: usize },
}

fn draw_lock(input: &str, error: &str) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Password Manager");
    fill(0, 34, W, 1, C_BORDER);
    text(20, 60, C_DIM, "Enter master password:");
    fill(20, 80, W - 40, 32, C_FIELD);
    border(20, 80, W - 40, 32, C_BORDER);
    let display = if input.is_empty() { "••••••••" } else { &mask(input) };
    let tc = if input.is_empty() { C_HINT } else { C_TITLE };
    text(30, 88, tc, display);
    if !error.is_empty() {
        text(20, 124, C_ERR, error);
    }
    text(20, H - 18, C_HINT, "Enter: unlock   Ctrl+C: quit");
    flush();
}

fn draw_list(entries: &[Entry], scroll: usize, cursor: usize, status: &str) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Password Manager");
    fill(0, 34, W, 1, C_BORDER);

    // Column headers
    text(20,  LIST_Y - 16, C_DIM, "SITE");
    text(300, LIST_Y - 16, C_DIM, "USERNAME");
    text(600, LIST_Y - 16, C_DIM, "PASSWORD");
    fill(0, LIST_Y - 2, W, 1, C_BORDER);

    if entries.is_empty() {
        text(20, LIST_Y + 10, C_HINT, "No entries yet. Press 'a' to add.");
    } else {
        let end = (scroll + VIS_ROWS).min(entries.len());
        for (vis_i, e) in entries[scroll..end].iter().enumerate() {
            let ry = LIST_Y + vis_i as u32 * ROW_H;
            let is_sel = (scroll + vis_i) == cursor;
            if is_sel { fill(0, ry, W, ROW_H, C_SEL); }
            let tc = if is_sel { C_TITLE } else { C_DIM };

            let site_disp: String = e.site.chars().take(28).collect();
            let user_disp: String = e.user.chars().take(24).collect();
            let pass_disp = mask(&e.pass);
            let pass_disp: String = pass_disp.chars().take(20).collect();

            text(20,  ry + 4, tc, &site_disp);
            text(300, ry + 4, tc, &user_disp);
            text(600, ry + 4, C_HINT, &pass_disp);
        }
    }

    fill(0, H - 30, W, 1, C_BORDER);
    if !status.is_empty() {
        let sc = if status.starts_with("Error") { C_ERR } else { C_OK };
        text(20, H - 18, sc, status);
    } else {
        text(20, H - 18, C_HINT, "↑↓: select   a: add   d: delete   c: copy pass   Ctrl+W: save   Ctrl+C: quit");
    }
    flush();
}

fn draw_add(field: usize, values: &[String; 3]) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Password Manager  —  Add Entry");
    fill(0, 34, W, 1, C_BORDER);

    let labels = ["Site / URL", "Username", "Password"];
    for (i, lbl) in labels.iter().enumerate() {
        let fy = 60 + i as u32 * 80;
        let is_active = i == field;
        let lc = if is_active { C_ACCENT } else { C_DIM };
        text(20, fy, lc, lbl);
        fill(20, fy + 18, W - 40, 32, C_FIELD);
        border(20, fy + 18, W - 40, 32, if is_active { C_ACCENT } else { C_BORDER });
        let val = if i == 2 { mask(&values[i]) } else { values[i].clone() };
        let display = if val.is_empty() && is_active { "_".to_string() } else { val };
        text(30, fy + 26, if is_active { C_TITLE } else { C_HINT }, &display);
    }

    text(20, H - 18, C_HINT, "Tab/Enter: next field   Esc: cancel   Enter on last: save");
    flush();
}

fn draw_confirm_delete(entry: &Entry) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Password Manager  —  Delete Entry");
    fill(0, 34, W, 1, C_BORDER);
    text(20, 80, C_ERR, "Delete this entry?");
    text(20, 110, C_DIM, &format!("Site: {}", entry.site));
    text(20, 132, C_DIM, &format!("User: {}", entry.user));
    text(20, H - 18, C_HINT, "y: confirm delete   Esc/n: cancel");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut mode = Mode::Lock { input: String::new(), error: String::new() };
    let mut entries: Vec<Entry> = Vec::new();
    let mut master_key = String::new();

    draw_lock("", "");

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match &mut mode {
            Mode::Lock { input, error } => {
                match raw.as_str() {
                    "\x03" => {
                        fill(0, 0, W, H, C_BG);
                        flush();
                        std::process::exit(0);
                    }
                    "\x7f" => {
                        input.pop();
                        error.clear();
                        let inp_clone = input.clone();
                        draw_lock(&inp_clone, "");
                    }
                    "" => {
                        let key = input.clone();
                        master_key = key.clone();
                        entries = load_vault(&key);
                        mode = Mode::List { scroll: 0, cursor: 0, status: format!("{} entries loaded", entries.len()) };
                        if let Mode::List { scroll, cursor, status } = &mode {
                            draw_list(&entries, *scroll, *cursor, status);
                        }
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' {
                            input.push(c);
                            error.clear();
                            let inp_clone = input.clone();
                            draw_lock(&inp_clone, "");
                        }
                    }
                    _ => {}
                }
            }

            Mode::List { scroll, cursor, status } => {
                let total = entries.len();
                match raw.as_str() {
                    "\x03" => {
                        fill(0, 0, W, H, C_BG);
                        flush();
                        std::process::exit(0);
                    }
                    "\x17" => {
                        match save_vault(&entries, &master_key) {
                            Ok(()) => *status = format!("Saved {} entries", entries.len()),
                            Err(e) => *status = format!("Error: {e}"),
                        }
                        draw_list(&entries, *scroll, *cursor, status);
                    }
                    "\x1b[A" => {
                        if *cursor > 0 { *cursor -= 1; }
                        if *cursor < *scroll { *scroll = *cursor; }
                        status.clear();
                        draw_list(&entries, *scroll, *cursor, status);
                    }
                    "\x1b[B" => {
                        if total > 0 && *cursor + 1 < total { *cursor += 1; }
                        if *cursor >= *scroll + VIS_ROWS { *scroll = *cursor + 1 - VIS_ROWS; }
                        status.clear();
                        draw_list(&entries, *scroll, *cursor, status);
                    }
                    "a" => {
                        mode = Mode::Add { field: 0, values: [String::new(), String::new(), String::new()] };
                        if let Mode::Add { field, values } = &mode {
                            draw_add(*field, values);
                        }
                    }
                    "d" => {
                        if *cursor < entries.len() {
                            let idx = *cursor;
                            mode = Mode::ConfirmDelete { target_idx: idx };
                            if let Mode::ConfirmDelete { target_idx } = &mode {
                                draw_confirm_delete(&entries[*target_idx]);
                            }
                        }
                    }
                    "c" => {
                        if *cursor < entries.len() {
                            let pass = entries[*cursor].pass.clone();
                            println!("@supervisor: clipboard-set {pass}");
                            *status = "Password copied to clipboard".to_string();
                            draw_list(&entries, *scroll, *cursor, status);
                        }
                    }
                    _ => {}
                }
            }

            Mode::Add { field, values } => {
                match raw.as_str() {
                    "\x1b" => {
                        mode = Mode::List { scroll: 0, cursor: 0, status: String::new() };
                        if let Mode::List { scroll, cursor, status } = &mode {
                            draw_list(&entries, *scroll, *cursor, status);
                        }
                    }
                    "\t" | "" => {
                        if *field < 2 {
                            *field += 1;
                            let f = *field;
                            draw_add(f, values);
                        } else {
                            // Save entry
                            let e = Entry {
                                site: values[0].clone(),
                                user: values[1].clone(),
                                pass: values[2].clone(),
                            };
                            entries.push(e);
                            let len = entries.len();
                            mode = Mode::List { scroll: 0, cursor: len.saturating_sub(1), status: "Entry added".to_string() };
                            if let Mode::List { scroll, cursor, status } = &mode {
                                draw_list(&entries, *scroll, *cursor, status);
                            }
                        }
                    }
                    "\x7f" => {
                        values[*field].pop();
                        let f = *field;
                        draw_add(f, values);
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' {
                            values[*field].push(c);
                            let f = *field;
                            draw_add(f, values);
                        }
                    }
                    _ => {}
                }
            }

            Mode::ConfirmDelete { target_idx } => {
                let idx = *target_idx;
                match raw.as_str() {
                    "y" | "Y" => {
                        if idx < entries.len() { entries.remove(idx); }
                        let cur = idx.saturating_sub(1).min(entries.len().saturating_sub(1));
                        mode = Mode::List { scroll: 0, cursor: cur, status: "Entry deleted".to_string() };
                        if let Mode::List { scroll, cursor, status } = &mode {
                            draw_list(&entries, *scroll, *cursor, status);
                        }
                    }
                    "n" | "\x1b" | "\x03" => {
                        mode = Mode::List { scroll: 0, cursor: idx, status: String::new() };
                        if let Mode::List { scroll, cursor, status } = &mode {
                            draw_list(&entries, *scroll, *cursor, status);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
