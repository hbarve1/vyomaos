// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 840;
const H: u32 = 680;
const HEADER_H: u32 = 48;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_CARD: u32    = 0x161B22FF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_RED: u32     = 0xFF7B72FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;

const CATS: [&str; 8] = ["Food", "Transport", "Housing", "Salary", "Freelance", "Shopping", "Health", "Other"];
const MAX_ENTRIES: usize = 20;

#[derive(Clone)]
struct Entry {
    income:  bool,
    cat:     usize,
    amount:  i64,
    desc:    [u8; 20],
    desc_len: usize,
}

impl Entry {
    fn desc_str(&self) -> &str {
        std::str::from_utf8(&self.desc[..self.desc_len]).unwrap_or("?")
    }
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

#[derive(PartialEq)]
enum Mode { List, Add }

struct App {
    entries: Vec<Entry>,
    sel:     usize,
    mode:    Mode,
    // Add form state
    form_field: usize, // 0=type 1=category 2=amount 3=desc
    form_income: bool,
    form_cat:    usize,
    form_amount: [u8; 12],
    form_amt_len: usize,
    form_desc:   [u8; 20],
    form_desc_len: usize,
}

impl App {
    fn new() -> Self {
        App {
            entries: Vec::new(),
            sel: 0,
            mode: Mode::List,
            form_field: 0,
            form_income: true,
            form_cat: 0,
            form_amount: [0; 12],
            form_amt_len: 0,
            form_desc: [0; 20],
            form_desc_len: 0,
        }
    }

    fn total_income(&self) -> i64 {
        self.entries.iter().filter(|e| e.income).map(|e| e.amount).sum()
    }

    fn total_expense(&self) -> i64 {
        self.entries.iter().filter(|e| !e.income).map(|e| e.amount).sum()
    }

    fn parse_amount(&self) -> i64 {
        let s = std::str::from_utf8(&self.form_amount[..self.form_amt_len]).unwrap_or("0");
        s.parse::<i64>().unwrap_or(0)
    }

    fn confirm_add(&mut self) {
        let amount = self.parse_amount();
        if amount <= 0 { return; }
        if self.entries.len() >= MAX_ENTRIES { return; }
        self.entries.push(Entry {
            income: self.form_income,
            cat: self.form_cat,
            amount,
            desc: self.form_desc,
            desc_len: self.form_desc_len,
        });
        self.sel = self.entries.len().saturating_sub(1);
        self.mode = Mode::List;
        self.reset_form();
    }

    fn reset_form(&mut self) {
        self.form_field = 0;
        self.form_income = true;
        self.form_cat = 0;
        self.form_amount = [0; 12];
        self.form_amt_len = 0;
        self.form_desc = [0; 20];
        self.form_desc_len = 0;
    }

    fn delete_sel(&mut self) {
        if self.entries.is_empty() { return; }
        self.entries.remove(self.sel);
        if self.sel >= self.entries.len() && self.sel > 0 { self.sel -= 1; }
    }

    fn handle_form_key(&mut self, raw: &str) {
        match raw {
            "\x1b" => { self.mode = Mode::List; self.reset_form(); }
            "\r" | "" => {
                if self.form_field < 3 { self.form_field += 1; }
                else { self.confirm_add(); }
            }
            "\t" => { self.form_field = (self.form_field + 1) % 4; }
            "\x7f" => {
                match self.form_field {
                    2 => { if self.form_amt_len > 0 { self.form_amt_len -= 1; } }
                    3 => { if self.form_desc_len > 0 { self.form_desc_len -= 1; } }
                    _ => {}
                }
            }
            "\x1b[C" | "\x1b[D" => {
                match self.form_field {
                    0 => { self.form_income = !self.form_income; }
                    1 => {
                        if raw == "\x1b[C" { self.form_cat = (self.form_cat + 1) % 8; }
                        else { self.form_cat = (self.form_cat + 7) % 8; }
                    }
                    _ => {}
                }
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                match self.form_field {
                    2 => {
                        if b >= b'0' && b <= b'9' && self.form_amt_len < 10 {
                            self.form_amount[self.form_amt_len] = b;
                            self.form_amt_len += 1;
                        }
                    }
                    3 => {
                        if b >= 0x20 && b < 0x7F && self.form_desc_len < 20 {
                            self.form_desc[self.form_desc_len] = b;
                            self.form_desc_len += 1;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
}

const LIST_X: u32 = 20;
const LIST_Y: u32 = 160;
const ROW_H: u32 = 28;

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Budget Tracker");
    text(200, 16, C_HINT, "a:add  d:delete  ↑↓:nav  Esc:back");

    // Summary bar
    let income = app.total_income();
    let expense = app.total_expense();
    let balance = income - expense;
    fill(LIST_X, 60, W - LIST_X * 2, 60, C_CARD);
    border(LIST_X, 60, W - LIST_X * 2, 60, C_BORDER);
    text(LIST_X + 16, 72, C_GREEN, &format!("Income: ${}", income));
    text(LIST_X + 220, 72, C_RED, &format!("Expenses: ${}", expense));
    let bal_col = if balance >= 0 { C_GREEN } else { C_RED };
    text(LIST_X + 460, 72, bal_col, &format!("Balance: ${}", balance));
    text(LIST_X + 16, 92, C_HINT, &format!("{}/{} entries", app.entries.len(), MAX_ENTRIES));

    // Column headers
    fill(LIST_X, LIST_Y - 22, W - LIST_X * 2, 22, C_CARD);
    text(LIST_X + 4, LIST_Y - 18, C_HINT, "Type");
    text(LIST_X + 100, LIST_Y - 18, C_HINT, "Category");
    text(LIST_X + 260, LIST_Y - 18, C_HINT, "Amount");
    text(LIST_X + 380, LIST_Y - 18, C_HINT, "Description");

    // Entries
    for (i, entry) in app.entries.iter().enumerate() {
        let ry = LIST_Y + i as u32 * ROW_H;
        if ry + ROW_H > H - 20 { break; }

        let bg = if i == app.sel && app.mode == Mode::List { C_SEL_BG } else { C_BG };
        fill(LIST_X, ry, W - LIST_X * 2, ROW_H - 2, bg);

        let type_col = if entry.income { C_GREEN } else { C_RED };
        let type_str = if entry.income { "Income" } else { "Expense" };
        text(LIST_X + 4, ry + 6, type_col, type_str);
        text(LIST_X + 100, ry + 6, C_TEXT, CATS[entry.cat]);
        text(LIST_X + 260, ry + 6, C_ORANGE, &format!("${}", entry.amount));
        text(LIST_X + 380, ry + 6, C_HINT, entry.desc_str());
    }

    if app.entries.is_empty() {
        text(LIST_X + 20, LIST_Y + 20, C_HINT, "No entries yet. Press 'a' to add one.");
    }

    // Add form overlay
    if app.mode == Mode::Add {
        let fx = (W - 500) / 2;
        let fy = (H - 320) / 2;
        fill(fx, fy, 500, 320, C_CARD);
        border(fx, fy, 500, 320, C_SEL);
        text(fx + 180, fy + 12, C_SEL, "Add Entry");

        let fields = ["Type", "Category", "Amount ($)", "Description"];
        for (i, &label) in fields.iter().enumerate() {
            let ly = fy + 52 + i as u32 * 56;
            let active = i == app.form_field;
            let lc = if active { C_SEL } else { C_HINT };
            text(fx + 20, ly - 2, lc, label);
            fill(fx + 20, ly + 16, 460, 28, if active { C_SEL_BG } else { C_BG });
            border(fx + 20, ly + 16, 460, 28, if active { C_SEL } else { C_BORDER });

            let val: String = match i {
                0 => if app.form_income { "Income".to_string() } else { "Expense".to_string() },
                1 => CATS[app.form_cat].to_string(),
                2 => {
                    let s = std::str::from_utf8(&app.form_amount[..app.form_amt_len]).unwrap_or("").to_string();
                    if active { s + "_" } else { s }
                }
                3 => {
                    let s = std::str::from_utf8(&app.form_desc[..app.form_desc_len]).unwrap_or("").to_string();
                    if active { s + "_" } else { s }
                }
                _ => String::new(),
            };
            let vc = if i == 0 { if app.form_income { C_GREEN } else { C_RED } } else { C_TEXT };
            text(fx + 28, ly + 22, vc, &val);
        }

        text(fx + 60, fy + 288, C_HINT, "Tab/Enter:next field  Enter on last=save  Esc=cancel");
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise budget");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        if app.mode == Mode::Add {
            if raw == "\x03" { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            app.handle_form_key(&raw);
        } else {
            match raw.as_str() {
                "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
                "\x1b[A" => { if app.sel > 0 { app.sel -= 1; } }
                "\x1b[B" => { if app.sel + 1 < app.entries.len() { app.sel += 1; } }
                "a" | "A" => { app.mode = Mode::Add; app.reset_form(); }
                "d" | "D" => { app.delete_sel(); }
                _ => {}
            }
        }
        draw(&app);
    }
}
