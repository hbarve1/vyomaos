use std::io::{self, BufRead, Write};

const W: u32 = 600;
const H: u32 = 520;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_ERR: u32    = 0xFF7B72FF;
const C_OK: u32     = 0x3FB950FF;
const C_BTN: u32    = 0x21262DFF;
const C_BTN_OP: u32 = 0x1B3A5CFF;

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

fn fmt_f64(v: f64) -> String {
    if v.is_nan() { return "NaN".to_string(); }
    if v.is_infinite() { return if v > 0.0 { "Inf" } else { "-Inf" }.to_string(); }
    if v == v.trunc() && v.abs() < 1e12 {
        format!("{}", v as i64)
    } else {
        // Up to 8 sig figs
        let s = format!("{:.8}", v);
        // Trim trailing zeros after decimal
        let s = s.trim_end_matches('0').trim_end_matches('.');
        s.to_string()
    }
}

fn evaluate(expr: &str, ans: f64, deg_mode: bool) -> Result<f64, String> {
    let expr = expr.trim().replace("ans", &fmt_f64(ans));
    if expr.is_empty() { return Err("empty".to_string()); }

    // Simple tokenized evaluator: handles +,-,*,/,^ and unary functions
    // Strategy: parse as a sequence of terms joined by +/-
    parse_add_sub(expr.trim(), deg_mode)
}

fn to_rad(v: f64, deg: bool) -> f64 {
    if deg { v * std::f64::consts::PI / 180.0 } else { v }
}

fn parse_add_sub(s: &str, deg: bool) -> Result<f64, String> {
    let s = s.trim();
    let mut result = 0f64;
    let mut sign   = 1f64;
    let mut i      = 0usize;

    // Handle leading sign
    let bytes = s.as_bytes();
    if i < bytes.len() && bytes[i] == b'-' { sign = -1.0; i += 1; }
    else if i < bytes.len() && bytes[i] == b'+' { i += 1; }

    while i <= s.len() {
        // Find next + or - (not inside parens)
        let seg_start = i;
        let mut depth = 0i32;
        while i < s.len() {
            match bytes[i] {
                b'(' => { depth += 1; i += 1; }
                b')' => { depth -= 1; i += 1; if depth < 0 { return Err("unmatched )".into()); } }
                b'+' | b'-' if depth == 0 && i > seg_start => { break; }
                _ => { i += 1; }
            }
        }
        let seg = s[seg_start..i].trim();
        if !seg.is_empty() {
            let val = parse_mul_div(seg, deg)?;
            result += sign * val;
        }
        if i < s.len() {
            sign = if bytes[i] == b'-' { -1.0 } else { 1.0 };
            i += 1;
        } else {
            break;
        }
    }
    Ok(result)
}

fn parse_mul_div(s: &str, deg: bool) -> Result<f64, String> {
    let s = s.trim();
    let bytes = s.as_bytes();
    let mut i = 0usize;
    let mut parts: Vec<(char, &str)> = Vec::new();
    let mut seg_start = 0usize;
    let mut depth = 0i32;

    while i < s.len() {
        match bytes[i] {
            b'(' => { depth += 1; i += 1; }
            b')' => { depth -= 1; i += 1; }
            b'*' | b'/' | b'^' if depth == 0 => {
                let seg = s[seg_start..i].trim();
                let op = bytes[i] as char;
                parts.push((op, seg));
                seg_start = i + 1;
                i += 1;
            }
            _ => { i += 1; }
        }
    }
    parts.push((' ', s[seg_start..].trim()));

    if parts.is_empty() { return Err("empty segment".into()); }

    let mut result = parse_unary(parts[0].1, deg)?;
    let mut k = 1;
    for (op, seg) in &parts[1..] {
        let val = parse_unary(seg, deg)?;
        result = match op {
            '*' => result * val,
            '/' => if val == 0.0 { return Err("div/0".into()); } else { result / val },
            '^' => result.powf(val),
            _ => val,
        };
        k += 1;
        let _ = k;
    }
    Ok(result)
}

fn parse_unary(s: &str, deg: bool) -> Result<f64, String> {
    let s = s.trim();
    if s.is_empty() { return Err("empty".into()); }
    let bytes = s.as_bytes();

    // Parenthesized sub-expression
    if bytes[0] == b'(' && bytes[s.len() - 1] == b')' {
        return parse_add_sub(&s[1..s.len()-1], deg);
    }

    // Unary minus
    if bytes[0] == b'-' {
        return Ok(-parse_unary(&s[1..], deg)?);
    }

    // Named functions: sin, cos, tan, sqrt, log, abs
    let funcs = [
        ("sin(",  |v: f64, d: bool| f64::sin(to_rad(v, d))),
        ("cos(",  |v: f64, d: bool| f64::cos(to_rad(v, d))),
        ("tan(",  |v: f64, d: bool| f64::tan(to_rad(v, d))),
        ("sqrt(", |v: f64, _: bool| v.sqrt()),
        ("log(",  |v: f64, _: bool| v.log10()),
        ("ln(",   |v: f64, _: bool| v.ln()),
        ("abs(",  |v: f64, _: bool| v.abs()),
    ];
    for (prefix, func) in &funcs {
        if s.to_ascii_lowercase().starts_with(prefix) && s.ends_with(')') {
            let inner = &s[prefix.len()..s.len()-1];
            let val = parse_add_sub(inner, deg)?;
            return Ok(func(val, deg));
        }
    }

    // Numeric literal
    s.parse::<f64>().map_err(|_| format!("bad token: {s}"))
}

struct State {
    expr:     String,
    result:   String,
    ans:      f64,
    deg_mode: bool,
    error:    bool,
}

impl State {
    fn new() -> Self {
        State { expr: String::new(), result: "0".to_string(), ans: 0.0, deg_mode: true, error: false }
    }
}

fn draw_button(x: u32, y: u32, w: u32, h: u32, label: &str, color: u32, text_color: u32) {
    fill(x, y, w, h, color);
    border(x, y, w, h, C_BORDER);
    let lx = x + (w.saturating_sub(label.len() as u32 * 8)) / 2;
    let ly = y + (h - 14) / 2;
    text(lx, ly, text_color, label);
}

fn draw(s: &State) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Scientific Calculator");
    fill(0, 34, W, 1, C_BORDER);

    // Display area
    fill(20, 44, W - 40, 60, 0x161B22FF);
    border(20, 44, W - 40, 60, C_BORDER);

    // Expression line
    let expr_disp: String = s.expr.chars().rev().take(48).collect::<String>().chars().rev().collect();
    text(28, 50, C_DIM, &expr_disp);

    // Result line
    let rc = if s.error { C_ERR } else { C_TITLE };
    text(28, 72, rc, &s.result);

    // Mode indicator
    let mode_str = if s.deg_mode { "DEG" } else { "RAD" };
    let mx = W - 20 - mode_str.len() as u32 * 8 - 4;
    text(mx, 50, C_ACCENT, mode_str);
    let ans_str = format!("ANS={}", fmt_f64(s.ans));
    text(mx.saturating_sub(ans_str.len() as u32 * 8 + 8), 50, C_HINT, &ans_str);

    // Button grid
    let bw = 80u32;
    let bh = 40u32;
    let gap = 4u32;
    let grid_x = 20u32;
    let grid_y = 116u32;

    let rows: &[&[(&str, u32, u32)]] = &[
        &[("sin(", C_BTN_OP, C_ACCENT), ("cos(", C_BTN_OP, C_ACCENT), ("tan(", C_BTN_OP, C_ACCENT), ("sqrt(", C_BTN_OP, C_ACCENT), ("log(", C_BTN_OP, C_ACCENT), ("ln(", C_BTN_OP, C_ACCENT)],
        &[("7",   C_BTN, C_TITLE),    ("8",   C_BTN, C_TITLE),    ("9",   C_BTN, C_TITLE),    ("/",   C_BTN_OP, C_ACCENT), ("(",   C_BTN, C_DIM),    (")",   C_BTN, C_DIM)],
        &[("4",   C_BTN, C_TITLE),    ("5",   C_BTN, C_TITLE),    ("6",   C_BTN, C_TITLE),    ("*",   C_BTN_OP, C_ACCENT), ("^",   C_BTN_OP, C_ACCENT), ("abs(", C_BTN_OP, C_ACCENT)],
        &[("1",   C_BTN, C_TITLE),    ("2",   C_BTN, C_TITLE),    ("3",   C_BTN, C_TITLE),    ("-",   C_BTN_OP, C_ACCENT), ("ans", C_BTN, C_DIM),   ("DEG", C_BTN_OP, C_OK)],
        &[("0",   C_BTN, C_TITLE),    (".",   C_BTN, C_TITLE),    ("+",   C_BTN_OP, C_ACCENT), ("=",  0x0D4A2CFF, C_OK), ("CLR", 0x5C1A1AFF, C_ERR), ("⌫",   C_BTN, C_DIM)],
    ];

    for (row_i, row) in rows.iter().enumerate() {
        for (col_i, &(lbl, bg, tc)) in row.iter().enumerate() {
            let bx = grid_x + col_i as u32 * (bw + gap);
            let by = grid_y + row_i as u32 * (bh + gap);
            draw_button(bx, by, bw, bh, lbl, bg, tc);
        }
    }

    fill(0, H - 30, W, 1, C_BORDER);
    text(20, H - 18, C_HINT, "type expr + Enter=eval  Esc=clr  ⌫=del  d=DEG/RAD  Ctrl+C=quit");
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut s = State::new();

    draw(&s);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => {
                fill(0, 0, W, H, C_BG);
                flush();
                std::process::exit(0);
            }
            "\x1b" => {
                s.expr.clear();
                s.result = "0".to_string();
                s.error  = false;
            }
            "\x7f" => {
                s.expr.pop();
                s.error = false;
            }
            "" => {
                match evaluate(&s.expr, s.ans, s.deg_mode) {
                    Ok(v) => {
                        s.ans    = v;
                        s.result = fmt_f64(v);
                        s.error  = false;
                    }
                    Err(e) => {
                        s.result = format!("Error: {e}");
                        s.error  = true;
                    }
                }
            }
            "d" | "D" => {
                s.deg_mode = !s.deg_mode;
            }
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if c.is_ascii_graphic() || c == ' ' {
                    s.expr.push(c);
                    s.error = false;
                }
            }
            _ => {}
        }

        draw(&s);
    }
}
