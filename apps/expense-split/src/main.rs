use std::io::{self, BufRead, Write};

const W: u32 = 800;
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

const MAX_PEOPLE: usize = 6;
const MAX_EXPENSES: usize = 20;

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

#[derive(Clone)]
struct Expense {
    payer:  usize,
    amount: i64, // in cents
    desc:   [u8; 20],
    dlen:   usize,
}

impl Expense {
    fn desc_str(&self) -> &str {
        std::str::from_utf8(&self.desc[..self.dlen]).unwrap_or("?")
    }
}

#[derive(PartialEq)]
enum View { People, Expenses, Settle }

#[derive(PartialEq)]
enum InputMode { None, AddPerson, AddExpense }

struct App {
    people:     [[u8; 12]; MAX_PEOPLE],
    plen:       [usize; MAX_PEOPLE],
    npeople:    usize,
    expenses:   Vec<Expense>,
    sel:        usize,
    view:       View,
    mode:       InputMode,
    // form
    form_buf:   [u8; 20],
    form_len:   usize,
    form_field: usize, // 0=name/payer, 1=amount, 2=desc
    form_payer: usize,
    form_amt:   [u8; 10],
    form_alen:  usize,
    form_desc:  [u8; 20],
    form_dlen:  usize,
}

impl App {
    fn new() -> Self {
        App {
            people:     [[0; 12]; MAX_PEOPLE],
            plen:       [0; MAX_PEOPLE],
            npeople:    0,
            expenses:   Vec::new(),
            sel:        0,
            view:       View::People,
            mode:       InputMode::None,
            form_buf:   [0; 20],
            form_len:   0,
            form_field: 0,
            form_payer: 0,
            form_amt:   [0; 10],
            form_alen:  0,
            form_desc:  [0; 20],
            form_dlen:  0,
        }
    }

    fn person_name(&self, i: usize) -> &str {
        std::str::from_utf8(&self.people[i][..self.plen[i]]).unwrap_or("?")
    }

    fn add_person_confirm(&mut self) {
        if self.npeople >= MAX_PEOPLE || self.form_len == 0 { return; }
        self.people[self.npeople][..self.form_len].copy_from_slice(&self.form_buf[..self.form_len]);
        self.plen[self.npeople] = self.form_len;
        self.npeople += 1;
        self.mode = InputMode::None;
        self.form_len = 0;
    }

    fn add_expense_confirm(&mut self) {
        let amt_str = std::str::from_utf8(&self.form_amt[..self.form_alen]).unwrap_or("0");
        let cents: i64 = amt_str.parse::<i64>().unwrap_or(0) * 100;
        if cents <= 0 { return; }
        self.expenses.push(Expense {
            payer: self.form_payer,
            amount: cents,
            desc: self.form_desc,
            dlen: self.form_dlen,
        });
        self.mode = InputMode::None;
        self.form_field = 0;
        self.form_alen = 0;
        self.form_dlen = 0;
    }

    fn settlements(&self) -> Vec<(usize, usize, i64)> {
        if self.npeople == 0 { return vec![]; }
        let total: i64 = self.expenses.iter().map(|e| e.amount).sum();
        let share = if self.npeople > 0 { total / self.npeople as i64 } else { 0 };
        let mut net = [0i64; MAX_PEOPLE];
        for e in &self.expenses { net[e.payer] += e.amount; }
        for i in 0..self.npeople { net[i] -= share; }

        let mut creditors: Vec<(usize, i64)> = (0..self.npeople).filter(|&i| net[i] > 0).map(|i| (i, net[i])).collect();
        let mut debtors:   Vec<(usize, i64)> = (0..self.npeople).filter(|&i| net[i] < 0).map(|i| (i, -net[i])).collect();
        let mut txns = vec![];

        let mut ci = 0; let mut di = 0;
        while ci < creditors.len() && di < debtors.len() {
            let pay = creditors[ci].1.min(debtors[di].1);
            txns.push((debtors[di].0, creditors[ci].0, pay));
            creditors[ci].1 -= pay;
            debtors[di].1  -= pay;
            if creditors[ci].1 == 0 { ci += 1; }
            if debtors[di].1 == 0   { di += 1; }
        }
        txns
    }
}

const COL1: u32 = 20;
const COL2: u32 = 420;
const ROW_H: u32 = 28;
const START_Y: u32 = HEADER_H + 60;

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Expense Split");
    text(200, 16, C_HINT, "p:add person  e:add expense  d:delete  Tab:view  ↑↓:nav");

    // Tab bar
    let tabs = [("People", &View::People), ("Expenses", &View::Expenses), ("Settle", &View::Settle)];
    for (i, (label, v)) in tabs.iter().enumerate() {
        let tx = COL1 + i as u32 * 120;
        let active = std::mem::discriminant(&app.view) == std::mem::discriminant(v);
        let tc = if active { C_SEL } else { C_HINT };
        fill(tx, HEADER_H + 8, 110, 32, if active { C_SEL_BG } else { C_CARD });
        border(tx, HEADER_H + 8, 110, 32, if active { C_SEL } else { C_BORDER });
        text(tx + 20, HEADER_H + 16, tc, label);
    }

    match app.view {
        View::People => {
            text(COL1, START_Y - 20, C_HINT, &format!("{}/{} people", app.npeople, MAX_PEOPLE));
            for i in 0..app.npeople {
                let ry = START_Y + i as u32 * ROW_H;
                let total: i64 = app.expenses.iter().filter(|e| e.payer == i).map(|e| e.amount).sum();
                text(COL1, ry, C_TEXT, &format!("{}. {}", i + 1, app.person_name(i)));
                text(COL1 + 200, ry, C_GREEN, &format!("Paid: ${}", total / 100));
            }
            if app.npeople == 0 {
                text(COL1, START_Y + 20, C_HINT, "No people yet. Press 'p' to add.");
            }
        }
        View::Expenses => {
            let total: i64 = app.expenses.iter().map(|e| e.amount).sum();
            text(COL1, START_Y - 20, C_HINT, &format!("Total: ${}  ({} expenses)", total / 100, app.expenses.len()));
            for (i, exp) in app.expenses.iter().enumerate() {
                let ry = START_Y + i as u32 * ROW_H;
                let is_sel = i == app.sel;
                fill(COL1, ry, W - COL1 * 2, ROW_H - 2, if is_sel { C_SEL_BG } else { C_BG });
                let pname = if exp.payer < app.npeople { app.person_name(exp.payer) } else { "?" };
                text(COL1 + 4, ry + 4, C_TEXT, &format!("{} paid ${} — {}", pname, exp.amount / 100, exp.desc_str()));
            }
            if app.expenses.is_empty() {
                text(COL1, START_Y + 20, C_HINT, "No expenses yet. Press 'e' to add.");
            }
        }
        View::Settle => {
            let txns = app.settlements();
            text(COL1, START_Y - 20, C_HINT, &format!("{} transactions to settle", txns.len()));
            for (i, &(from, to, amt)) in txns.iter().enumerate() {
                let ry = START_Y + i as u32 * ROW_H;
                let fn_ = if from < app.npeople { app.person_name(from) } else { "?" };
                let tn  = if to   < app.npeople { app.person_name(to)   } else { "?" };
                text(COL1, ry, C_TEXT, &format!("{} → {} : ${}", fn_, tn, amt / 100));
            }
            if txns.is_empty() {
                text(COL1, START_Y + 20, C_GREEN, if app.npeople > 0 { "All settled up!" } else { "Add people and expenses first." });
            }
        }
    }

    // Input overlay
    if app.mode != InputMode::None {
        let fx = (W - 440) / 2;
        let fy = (H - 220) / 2;
        fill(fx, fy, 440, 220, C_CARD);
        border(fx, fy, 440, 220, C_SEL);

        match app.mode {
            InputMode::AddPerson => {
                text(fx + 140, fy + 12, C_SEL, "Add Person");
                text(fx + 20, fy + 56, C_HINT, "Name:");
                fill(fx + 20, fy + 76, 400, 28, C_SEL_BG);
                border(fx + 20, fy + 76, 400, 28, C_SEL);
                let nm = std::str::from_utf8(&app.form_buf[..app.form_len]).unwrap_or("");
                text(fx + 28, fy + 82, C_TEXT, &format!("{}_", nm));
                text(fx + 100, fy + 180, C_HINT, "Enter=confirm  Esc=cancel");
            }
            InputMode::AddExpense => {
                text(fx + 140, fy + 12, C_SEL, "Add Expense");
                let fields = ["Payer (1-N)", "Amount ($)", "Description"];
                for (i, &label) in fields.iter().enumerate() {
                    let ly = fy + 52 + i as u32 * 48;
                    let active = i == app.form_field;
                    text(fx + 20, ly, if active { C_SEL } else { C_HINT }, label);
                    fill(fx + 20, ly + 18, 400, 22, if active { C_SEL_BG } else { C_BG });
                    border(fx + 20, ly + 18, 400, 22, if active { C_SEL } else { C_BORDER });
                    let val = match i {
                        0 => {
                            let pn = if app.form_payer < app.npeople { app.person_name(app.form_payer) } else { "?" };
                            format!("{} ({})", app.form_payer + 1, pn)
                        }
                        1 => {
                            let s = std::str::from_utf8(&app.form_amt[..app.form_alen]).unwrap_or("");
                            if active { format!("{}_", s) } else { s.to_string() }
                        }
                        2 => {
                            let s = std::str::from_utf8(&app.form_desc[..app.form_dlen]).unwrap_or("");
                            if active { format!("{}_", s) } else { s.to_string() }
                        }
                        _ => String::new(),
                    };
                    text(fx + 28, ly + 22, C_TEXT, &val);
                }
                text(fx + 60, fy + 192, C_HINT, "Tab/Enter:next  Enter on last=save  Esc=cancel");
            }
            _ => {}
        }
    }

    flush();
}

fn handle_expense_form(app: &mut App, raw: &str) {
    match raw {
        "\x1b" => { app.mode = InputMode::None; }
        "\r" | "" => {
            if app.form_field < 2 { app.form_field += 1; }
            else { app.add_expense_confirm(); }
        }
        "\t" => { app.form_field = (app.form_field + 1) % 3; }
        "\x7f" => match app.form_field {
            1 => { if app.form_alen > 0 { app.form_alen -= 1; } }
            2 => { if app.form_dlen > 0 { app.form_dlen -= 1; } }
            _ => {}
        },
        "\x1b[C" => if app.form_field == 0 { app.form_payer = (app.form_payer + 1) % app.npeople.max(1); },
        "\x1b[D" => if app.form_field == 0 { app.form_payer = (app.form_payer + app.npeople.max(1) - 1) % app.npeople.max(1); },
        s if s.len() == 1 => {
            let b = s.as_bytes()[0];
            match app.form_field {
                1 => if b >= b'0' && b <= b'9' && app.form_alen < 8 { app.form_amt[app.form_alen] = b; app.form_alen += 1; }
                2 => if b >= 0x20 && b < 0x7F && app.form_dlen < 20 { app.form_desc[app.form_dlen] = b; app.form_dlen += 1; }
                _ => {}
            }
        }
        _ => {}
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise expense-split");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        if app.mode == InputMode::AddPerson {
            match raw.as_str() {
                "\x1b" | "\x03" => { if raw == "\x03" { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); } app.mode = InputMode::None; app.form_len = 0; }
                "\r" | "" => { app.add_person_confirm(); }
                "\x7f" => { if app.form_len > 0 { app.form_len -= 1; } }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7F && app.form_len < 12 { app.form_buf[app.form_len] = b; app.form_len += 1; }
                }
                _ => {}
            }
        } else if app.mode == InputMode::AddExpense {
            if raw == "\x03" { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
            handle_expense_form(&mut app, &raw);
        } else {
            match raw.as_str() {
                "\x1b" | "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
                "\x1b[A" => { if app.sel > 0 { app.sel -= 1; } }
                "\x1b[B" => { if app.sel + 1 < app.expenses.len() { app.sel += 1; } }
                "\t" => {
                    app.view = match app.view {
                        View::People   => View::Expenses,
                        View::Expenses => View::Settle,
                        View::Settle   => View::People,
                    };
                }
                "p" | "P" => { if app.npeople < MAX_PEOPLE { app.mode = InputMode::AddPerson; app.form_len = 0; } }
                "e" | "E" => { if app.npeople > 0 && app.expenses.len() < MAX_EXPENSES { app.mode = InputMode::AddExpense; app.form_field = 0; app.form_payer = 0; app.form_alen = 0; app.form_dlen = 0; } }
                "d" | "D" => {
                    if !app.expenses.is_empty() && app.sel < app.expenses.len() {
                        app.expenses.remove(app.sel);
                        if app.sel >= app.expenses.len() && app.sel > 0 { app.sel -= 1; }
                    }
                }
                _ => {}
            }
        }
        draw(&app);
    }
}
