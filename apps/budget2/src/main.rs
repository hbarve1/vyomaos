// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1000;
const H: u32 = 700;
const HEADER_H: u32 = 40;
const MONTH_BAR_H: u32 = 28;
const CONTENT_Y: u32 = HEADER_H + MONTH_BAR_H;
const LEFT_W: u32 = 480;
const RIGHT_X: u32 = LEFT_W + 8;
const RIGHT_W: u32 = W - RIGHT_X - 8;
const CONTENT_H: u32 = H - CONTENT_Y - 32;
const STATUS_H: u32 = 28;
const LINE_H: u32 = 20;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
const C_RED: u32    = 0xFF7B72FF;
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

const MONTHS: &[&str] = &[
    "Jan", "Feb", "Mar", "Apr", "May", "Jun",
    "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const CATS: &[&str] = &[
    "Housing", "Food", "Transport", "Utilities",
    "Health", "Entertainment", "Savings", "Other",
];

// Default monthly budget targets per category (USD)
const BUDGETS: &[u32] = &[1500, 600, 300, 200, 200, 150, 500, 150];

#[derive(Clone)]
struct MonthData {
    income:   u32,
    expenses: [u32; 8],
}

impl MonthData {
    fn new(month_idx: usize) -> Self {
        // Slightly varied defaults to make demo interesting
        let base_income = 4500u32;
        let variance = (month_idx as u32 * 37 + 100) % 400;
        let income = base_income + variance;
        let mut expenses = [0u32; 8];
        for (i, &budget) in BUDGETS.iter().enumerate() {
            let var = (month_idx as u32 * (i as u32 + 7) * 13 + 50) % (budget / 4 + 1);
            expenses[i] = if (month_idx + i) % 3 == 0 {
                budget + var       // occasionally over-budget
            } else {
                budget.saturating_sub(var)
            };
        }
        MonthData { income, expenses }
    }
    fn total_expenses(&self) -> u32 { self.expenses.iter().sum() }
    fn savings(&self) -> i32 { self.income as i32 - self.total_expenses() as i32 }
}

#[derive(PartialEq)]
enum Mode { Normal, AddIncome, AddExpense }

struct App {
    months:     Vec<MonthData>,
    cur_month:  usize,
    sel_cat:    usize,
    mode:       Mode,
    input_buf:  String,
    status:     String,
}

impl App {
    fn new() -> Self {
        let months: Vec<MonthData> = (0..12).map(MonthData::new).collect();
        App {
            months,
            cur_month: 4, // May (0-indexed)
            sel_cat: 0,
            mode: Mode::Normal,
            input_buf: String::new(),
            status: String::from("Tab=month  ↑↓=category  +/-=$50  A=income  E=expense  R=reset  Ctrl+C=quit"),
        }
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Budget Planner");
    let md = &app.months[app.cur_month];
    text(200, 12, C_GREEN, &format!("Income: ${}", md.income));
    text(360, 12, C_RED,   &format!("Expenses: ${}", md.total_expenses()));
    let sav = md.savings();
    let sav_col = if sav >= 0 { C_GREEN } else { C_RED };
    let sav_str = if sav >= 0 { format!("Savings: +${}", sav) } else { format!("Deficit: -${}", -sav) };
    text(560, 12, sav_col, &sav_str);

    // Month selector bar
    fill(0, HEADER_H, W, MONTH_BAR_H, C_CARD);
    fill(0, HEADER_H + MONTH_BAR_H - 1, W, 1, C_BORDER);
    let mut mx = 8u32;
    for (i, &mn) in MONTHS.iter().enumerate() {
        let active = i == app.cur_month;
        let col = if active { C_SEL } else { C_HINT };
        if active {
            fill(mx - 2, HEADER_H + 2, mn.len() as u32 * 8 + 8, 22, C_HEADER);
            border(mx - 2, HEADER_H + 2, mn.len() as u32 * 8 + 8, 22, C_SEL);
        }
        text(mx, HEADER_H + 7, col, mn);
        mx += mn.len() as u32 * 8 + 14;
    }

    // Left: category list with bar chart
    fill(0, CONTENT_Y, LEFT_W, CONTENT_H, C_BG);
    text(8, CONTENT_Y + 6, C_HINT, "Category           Spent     Budget    Bar");
    fill(0, CONTENT_Y + 22, LEFT_W, 1, C_BORDER);

    let bar_max_w = 120u32;
    for (i, &cat) in CATS.iter().enumerate() {
        let vy = CONTENT_Y + 28 + i as u32 * LINE_H * 2;
        let is_sel = i == app.sel_cat;
        let spent = md.expenses[i];
        let budget = BUDGETS[i];
        let over = spent > budget;
        let col = if over { C_RED } else { C_TEXT };
        let sel_col = if is_sel { C_SEL } else { col };

        if is_sel {
            fill(0, vy - 2, LEFT_W, LINE_H * 2, C_CARD);
        }

        text(8, vy, sel_col, &format!("{:<14}", cat));
        text(120, vy, col, &format!("${:<8}", spent));
        text(200, vy, C_HINT, &format!("${:<8}", budget));

        // Bar
        let bar_w = if budget > 0 { (spent * bar_max_w / budget).min(bar_max_w + 20) } else { 0 };
        let budget_bar = bar_max_w.min(bar_max_w);
        fill(288, vy + 2, budget_bar, 10, C_BORDER);
        fill(288, vy + 2, bar_w.min(bar_max_w), 10, if over { C_RED } else { C_GREEN });
        if over {
            fill(288 + bar_max_w, vy + 2, (bar_w - bar_max_w).min(20), 10, 0xFF000040);
        }
        // Pct label
        let pct = if budget > 0 { spent * 100 / budget } else { 0 };
        let pct_col = if pct > 100 { C_RED } else if pct > 80 { C_YELLOW } else { C_HINT };
        text(416, vy + 2, pct_col, &format!("{}%", pct));
    }

    // Divider
    fill(LEFT_W, CONTENT_Y, 1, CONTENT_H, C_BORDER);

    // Right: summary panel
    fill(RIGHT_X, CONTENT_Y, RIGHT_W, CONTENT_H, C_CARD);
    border(RIGHT_X, CONTENT_Y, RIGHT_W, CONTENT_H, C_BORDER);

    text(RIGHT_X + 8, CONTENT_Y + 8, C_HINT, &format!("{} Summary", MONTHS[app.cur_month]));
    fill(RIGHT_X + 8, CONTENT_Y + 26, RIGHT_W - 16, 1, C_BORDER);

    let mut ry = CONTENT_Y + 34;

    // Income
    text(RIGHT_X + 8, ry, C_HINT, "Income:");
    text(RIGHT_X + 100, ry, C_GREEN, &format!("${}", md.income));
    ry += LINE_H;

    // Total expenses
    text(RIGHT_X + 8, ry, C_HINT, "Expenses:");
    text(RIGHT_X + 100, ry, C_RED, &format!("${}", md.total_expenses()));
    ry += LINE_H;

    // Net savings
    fill(RIGHT_X + 8, ry, RIGHT_W - 16, 1, C_BORDER);
    ry += 6;
    text(RIGHT_X + 8, ry, C_HINT, "Net:");
    let sav2 = md.savings();
    let sc2 = if sav2 >= 0 { C_GREEN } else { C_RED };
    let ss2 = if sav2 >= 0 { format!("+${}", sav2) } else { format!("-${}", -sav2) };
    text(RIGHT_X + 100, ry, sc2, &ss2);
    ry += LINE_H + 8;

    // Savings goal progress bar
    let goal = 500u32;
    text(RIGHT_X + 8, ry, C_HINT, "Savings goal: $500/mo");
    ry += LINE_H;
    let bar_total = RIGHT_W - 20;
    fill(RIGHT_X + 8, ry, bar_total, 12, C_BORDER);
    let sav_clamped = sav2.max(0) as u32;
    let goal_bar = (sav_clamped * bar_total / goal.max(1)).min(bar_total);
    fill(RIGHT_X + 8, ry, goal_bar, 12, if sav2 >= goal as i32 { C_GREEN } else { C_YELLOW });
    ry += 20;
    let goal_pct = sav_clamped * 100 / goal.max(1);
    text(RIGHT_X + 8, ry, C_HINT, &format!("{}% of goal", goal_pct));
    ry += LINE_H + 12;

    // 12-month overview mini-bars
    text(RIGHT_X + 8, ry, C_HINT, "Year overview:");
    ry += LINE_H;
    let bar_h_mini = 40u32;
    let col_w = (RIGHT_W - 16) / 12;
    let max_expense = app.months.iter().map(|m| m.total_expenses()).max().unwrap_or(1);
    for (i, m) in app.months.iter().enumerate() {
        let bx = RIGHT_X + 8 + i as u32 * col_w;
        let exp_h = (m.total_expenses() * bar_h_mini / max_expense).max(1);
        let inc_h = (m.income * bar_h_mini / max_expense).max(1);
        // income bar (background)
        fill(bx, ry + bar_h_mini - inc_h, col_w.saturating_sub(2), inc_h, C_BORDER);
        // expense bar
        let exp_col = if m.total_expenses() > m.income { C_RED } else { C_GREEN };
        fill(bx, ry + bar_h_mini - exp_h, col_w.saturating_sub(2), exp_h, exp_col);
        // current month indicator
        if i == app.cur_month {
            fill(bx, ry + bar_h_mini + 2, col_w.saturating_sub(2), 2, C_SEL);
        }
    }
    ry += bar_h_mini + 8;
    text(RIGHT_X + 8, ry, C_HINT, "Jan        Jun        Dec");

    // Input overlay
    if app.mode != Mode::Normal {
        let prompt = if app.mode == Mode::AddIncome { "Add income ($): " } else { "Add expense ($): " };
        let oy = H / 2 - 30;
        fill(RIGHT_X + 8, oy, RIGHT_W - 16, 56, C_HEADER);
        border(RIGHT_X + 8, oy, RIGHT_W - 16, 56, C_SEL);
        text(RIGHT_X + 16, oy + 10, C_HINT, prompt);
        text(RIGHT_X + 16, oy + 30, C_TEXT, &format!("{}_", app.input_buf));
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise budget2");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if app.mode != Mode::Normal {
            match raw.as_str() {
                "\x03" => { app.mode = Mode::Normal; app.input_buf.clear(); }
                "\x7f" => { app.input_buf.pop(); }
                "" => {
                    // Enter — parse amount
                    if let Ok(amt) = app.input_buf.trim().parse::<u32>() {
                        let md = &mut app.months[app.cur_month];
                        if app.mode == Mode::AddIncome {
                            md.income += amt;
                            app.status = format!("Income +${} added.", amt);
                        } else {
                            md.expenses[app.sel_cat] += amt;
                            app.status = format!("Expense +${} added to {}.", amt, CATS[app.sel_cat]);
                        }
                    } else {
                        app.status = "Invalid amount.".to_string();
                    }
                    app.input_buf.clear();
                    app.mode = Mode::Normal;
                }
                s if s.chars().all(|c| c.is_ascii_digit() || c == '.') => {
                    app.input_buf.push_str(s);
                }
                _ => {}
            }
            draw(&app);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

            "\t" => {
                app.cur_month = (app.cur_month + 1) % 12;
            }
            "\x1b[A" => {
                if app.sel_cat > 0 { app.sel_cat -= 1; }
            }
            "\x1b[B" => {
                if app.sel_cat + 1 < CATS.len() { app.sel_cat += 1; }
            }

            "+" | "=" => {
                app.months[app.cur_month].expenses[app.sel_cat] += 50;
                app.status = format!("{}  +$50 → ${}", CATS[app.sel_cat], app.months[app.cur_month].expenses[app.sel_cat]);
            }
            "-" => {
                let v = &mut app.months[app.cur_month].expenses[app.sel_cat];
                *v = v.saturating_sub(50);
                app.status = format!("{}  -$50 → ${}", CATS[app.sel_cat], *v);
            }

            "a" | "A" => {
                app.mode = Mode::AddIncome;
                app.input_buf.clear();
                app.status = "Enter income amount and press Enter".to_string();
            }
            "e" | "E" => {
                app.mode = Mode::AddExpense;
                app.input_buf.clear();
                app.status = format!("Adding expense to {}. Enter amount.", CATS[app.sel_cat]);
            }

            "r" | "R" => {
                app.months[app.cur_month] = MonthData::new(app.cur_month);
                app.status = format!("{} reset to defaults.", MONTHS[app.cur_month]);
            }

            _ => {}
        }

        draw(&app);
    }
}
