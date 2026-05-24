// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;

const COLS: usize = 10;
const ROWS: usize = 20;
const CELL_W: u32 = 100;
const CELL_H: u32 = 22;
const ROW_NUM_W: u32 = 36;
const COL_HDR_H: u32 = 24;

const CONTENT_Y: u32 = HEADER_H + COL_HDR_H;
const CHAR_W: u32 = 8;

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
const C_CELL: u32   = 0x0D1117FF;
const C_CELL_ALT: u32 = 0x131920FF;

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

// Cell address helpers
fn col_letter(c: usize) -> char { (b'A' + c as u8) as char }

fn cell_addr(r: usize, c: usize) -> String { format!("{}{}", col_letter(c), r + 1) }

// Parse "A1" → (row, col), 0-indexed
fn parse_addr(s: &str) -> Option<(usize, usize)> {
    let s = s.trim();
    if s.is_empty() { return None; }
    let col_char = s.chars().next()?;
    if !col_char.is_ascii_alphabetic() { return None; }
    let c = (col_char.to_ascii_uppercase() as u8 - b'A') as usize;
    let row_str = &s[1..];
    let r: usize = row_str.parse::<usize>().ok()?.checked_sub(1)?;
    if r < ROWS && c < COLS { Some((r, c)) } else { None }
}

struct Sheet {
    cells: [[String; COLS]; ROWS],
    cursor_r: usize,
    cursor_c: usize,
    editing: bool,
    edit_buf: String,
}

impl Sheet {
    fn new() -> Self {
        Sheet {
            cells: std::array::from_fn(|_| std::array::from_fn(|_| String::new())),
            cursor_r: 0,
            cursor_c: 0,
            editing: false,
            edit_buf: String::new(),
        }
    }

    fn raw(&self, r: usize, c: usize) -> &str { &self.cells[r][c] }

    fn eval(&self, r: usize, c: usize) -> String {
        let raw = &self.cells[r][c];
        if !raw.starts_with('=') { return raw.clone(); }
        let expr = raw[1..].trim();
        self.eval_expr(expr)
    }

    fn eval_expr(&self, expr: &str) -> String {
        let up = expr.to_uppercase();

        // =SUM(A1:B3) or =AVG(A1:B3)
        if let Some(rest) = up.strip_prefix("SUM(").and_then(|s| s.strip_suffix(')')) {
            let nums = self.range_values(rest);
            if nums.is_empty() { return "0".to_string(); }
            return format!("{}", nums.iter().sum::<i64>());
        }
        if let Some(rest) = up.strip_prefix("AVG(").and_then(|s| s.strip_suffix(')')) {
            let nums = self.range_values(rest);
            if nums.is_empty() { return "0".to_string(); }
            let sum: i64 = nums.iter().sum();
            return format!("{}", sum / nums.len() as i64);
        }

        // =A1 cell reference
        if let Some((r, c)) = parse_addr(expr) {
            return self.eval(r, c);
        }

        // plain number or text
        expr.to_string()
    }

    fn range_values(&self, range: &str) -> Vec<i64> {
        // "A1:C3" or "A1" single cell
        let parts: Vec<&str> = range.split(':').collect();
        if parts.len() == 2 {
            let start = parse_addr(parts[0]);
            let end   = parse_addr(parts[1]);
            if let (Some((r0, c0)), Some((r1, c1))) = (start, end) {
                let mut out = Vec::new();
                for r in r0..=r1.min(ROWS-1) {
                    for c in c0..=c1.min(COLS-1) {
                        let v = self.eval(r, c);
                        if let Ok(n) = v.trim().parse::<i64>() { out.push(n); }
                    }
                }
                return out;
            }
        } else if parts.len() == 1 {
            if let Some((r, c)) = parse_addr(parts[0]) {
                let v = self.eval(r, c);
                if let Ok(n) = v.trim().parse::<i64>() { return vec![n]; }
            }
        }
        Vec::new()
    }

    fn begin_edit(&mut self) {
        self.editing = true;
        self.edit_buf = self.cells[self.cursor_r][self.cursor_c].clone();
    }

    fn commit_edit(&mut self) {
        self.cells[self.cursor_r][self.cursor_c] = self.edit_buf.clone();
        self.editing = false;
        self.edit_buf.clear();
    }

    fn cancel_edit(&mut self) {
        self.editing = false;
        self.edit_buf.clear();
    }

    fn save_csv(&self) -> Result<(), String> {
        let mut out = String::new();
        // Header row
        out.push_str(&(0..COLS).map(|c| col_letter(c).to_string()).collect::<Vec<_>>().join(","));
        out.push('\n');
        for r in 0..ROWS {
            let row: Vec<String> = (0..COLS).map(|c| {
                let v = self.eval(r, c);
                if v.contains(',') || v.contains('"') || v.contains('\n') {
                    format!("\"{}\"", v.replace('"', "\"\""))
                } else {
                    v
                }
            }).collect();
            out.push_str(&row.join(","));
            out.push('\n');
        }
        std::fs::write("/data/spreadsheet.csv", out).map_err(|e| e.to_string())
    }

    fn load_csv(&mut self) -> Result<(), String> {
        let content = std::fs::read_to_string("/data/spreadsheet.csv").map_err(|e| e.to_string())?;
        let mut lines = content.lines();
        lines.next(); // skip header
        for (r, line) in lines.enumerate().take(ROWS) {
            let fields = parse_csv_row(line);
            for (c, val) in fields.into_iter().enumerate().take(COLS) {
                self.cells[r][c] = val;
            }
        }
        Ok(())
    }
}

fn parse_csv_row(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if in_quotes => {
                if chars.peek() == Some(&'"') { chars.next(); cur.push('"'); }
                else { in_quotes = false; }
            }
            '"' => { in_quotes = true; }
            ',' if !in_quotes => { fields.push(cur.clone()); cur.clear(); }
            c => { cur.push(c); }
        }
    }
    fields.push(cur);
    fields
}

fn cell_display(sheet: &Sheet, r: usize, c: usize) -> String {
    let v = sheet.eval(r, c);
    let max_chars = (CELL_W as usize - 4) / CHAR_W as usize;
    if v.len() > max_chars { v[..max_chars].to_string() } else { v }
}

fn draw(sheet: &Sheet, status_msg: &str) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Spreadsheet");
    text(180, 16, C_HINT, "10 columns × 20 rows");
    text(440, 16, C_HINT, "Arrows:nav  Enter:edit  =SUM/AVG  Ctrl+W:save  Ctrl+L:load");

    // Column headers
    let hdr_y = HEADER_H;
    fill(0, hdr_y, W, COL_HDR_H, C_HEADER);
    fill(ROW_NUM_W, hdr_y, 1, COL_HDR_H, C_BORDER);
    for c in 0..COLS {
        let cx = ROW_NUM_W + c as u32 * CELL_W;
        fill(cx + CELL_W - 1, hdr_y, 1, COL_HDR_H, C_BORDER);
        let label = col_letter(c).to_string();
        let tx = cx + (CELL_W - CHAR_W) / 2;
        text(tx, hdr_y + 5, C_HINT, &label);
    }

    // Grid
    for r in 0..ROWS {
        let ry = CONTENT_Y + r as u32 * CELL_H;
        if ry + CELL_H > H - STATUS_H { break; }

        // Row number
        let row_bg = if r % 2 == 0 { C_CELL } else { C_CELL_ALT };
        fill(0, ry, ROW_NUM_W, CELL_H, row_bg);
        fill(ROW_NUM_W - 1, ry, 1, CELL_H, C_BORDER);
        text(4, ry + 4, C_HINT, &format!("{:2}", r + 1));

        for c in 0..COLS {
            let cx = ROW_NUM_W + c as u32 * CELL_W;
            let is_cursor = r == sheet.cursor_r && c == sheet.cursor_c;

            let cell_bg = if is_cursor { 0x1C2D4EFF } else { row_bg };
            fill(cx, ry, CELL_W, CELL_H, cell_bg);
            fill(cx + CELL_W - 1, ry, 1, CELL_H, C_BORDER);
            fill(cx, ry + CELL_H - 1, CELL_W, 1, C_BORDER);

            if is_cursor {
                border(cx, ry, CELL_W, CELL_H, C_SEL);
            }

            let display = if is_cursor && sheet.editing {
                let s = &sheet.edit_buf;
                let max_c = (CELL_W as usize - 8) / CHAR_W as usize;
                if s.len() > max_c { &s[s.len()-max_c..] } else { s.as_str() }.to_string()
            } else {
                cell_display(sheet, r, c)
            };

            let tcol = if sheet.cells[r][c].starts_with('=') { C_ORANGE } else { C_TEXT };
            text(cx + 3, ry + 4, tcol, &display);

            // Cursor blink in edit mode
            if is_cursor && sheet.editing {
                let ex = cx + 3 + display.len() as u32 * CHAR_W;
                fill(ex, ry + 3, 2, CELL_H - 6, C_SEL);
            }
        }
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);

    let addr = cell_addr(sheet.cursor_r, sheet.cursor_c);
    let raw_val = sheet.raw(sheet.cursor_r, sheet.cursor_c);
    let eval_val = sheet.eval(sheet.cursor_r, sheet.cursor_c);

    let cell_info = if sheet.editing {
        format!("{}  editing: {}", addr, sheet.edit_buf)
    } else if raw_val != eval_val.as_str() {
        format!("{}  raw: {}  →  {}", addr, raw_val, eval_val)
    } else {
        format!("{}  {}", addr, raw_val)
    };

    let status_line = if status_msg.is_empty() { cell_info } else { format!("{}  |  {}", cell_info, status_msg) };
    let max_s = (W as usize - 16) / CHAR_W as usize;
    let s = if status_line.len() > max_s { &status_line[..max_s] } else { &status_line };
    text(8, sb_y + 6, C_HINT, s);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut sheet = Sheet::new();
    let mut status_msg = String::new();

    println!("@supervisor: raise spreadsheet");
    let _ = io::stdout().flush();
    draw(&sheet, &status_msg);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        status_msg.clear();

        if sheet.editing {
            match raw.as_str() {
                "\x03" | "\x1b" => { sheet.cancel_edit(); }
                "\x7f" => { sheet.edit_buf.pop(); }
                "" => {
                    sheet.commit_edit();
                    // Move down after Enter
                    if sheet.cursor_r + 1 < ROWS { sheet.cursor_r += 1; }
                }
                "\t" => {
                    sheet.commit_edit();
                    if sheet.cursor_c + 1 < COLS { sheet.cursor_c += 1; }
                    else if sheet.cursor_r + 1 < ROWS { sheet.cursor_c = 0; sheet.cursor_r += 1; }
                }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { sheet.edit_buf.push(b as char); }
                }
                _ => {}
            }
        } else {
            match raw.as_str() {
                "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
                "\x1b[A" => { sheet.cursor_r = sheet.cursor_r.saturating_sub(1); }
                "\x1b[B" => { if sheet.cursor_r + 1 < ROWS { sheet.cursor_r += 1; } }
                "\x1b[C" => { if sheet.cursor_c + 1 < COLS { sheet.cursor_c += 1; } }
                "\x1b[D" => { sheet.cursor_c = sheet.cursor_c.saturating_sub(1); }
                "\x1b[H" => { sheet.cursor_r = 0; sheet.cursor_c = 0; }
                "\x1b[F" => { sheet.cursor_r = ROWS - 1; sheet.cursor_c = COLS - 1; }
                "\x7f" => {
                    sheet.cells[sheet.cursor_r][sheet.cursor_c].clear();
                }
                "" => { sheet.begin_edit(); }
                "\t" => {
                    if sheet.cursor_c + 1 < COLS { sheet.cursor_c += 1; }
                    else if sheet.cursor_r + 1 < ROWS { sheet.cursor_c = 0; sheet.cursor_r += 1; }
                }
                "\x17" => { // Ctrl+W save
                    match sheet.save_csv() {
                        Ok(())  => status_msg = "Saved to /data/spreadsheet.csv".to_string(),
                        Err(e)  => status_msg = format!("Save failed: {}", e),
                    }
                }
                "\x0c" => { // Ctrl+L load
                    match sheet.load_csv() {
                        Ok(())  => status_msg = "Loaded from /data/spreadsheet.csv".to_string(),
                        Err(e)  => status_msg = format!("Load failed: {}", e),
                    }
                }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f {
                        sheet.begin_edit();
                        sheet.edit_buf.clear();
                        sheet.edit_buf.push(b as char);
                    }
                }
                _ => {}
            }
        }
        draw(&sheet, &status_msg);
    }
}
