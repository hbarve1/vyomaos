// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 800;
const COLS: usize = 20;
const ROWS: usize = 15;
const CELL_W: u32 = 64;
const CELL_H: u32 = 22;
const ROW_NUM_W: u32 = 36;
const COL_HDR_H: u32 = 22;
const HEADER_H: u32 = 48;
const FORMULA_H: u32 = 26;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;

const COL_HDR_Y: u32 = HEADER_H + FORMULA_H;
const GRID_Y: u32 = COL_HDR_Y + COL_HDR_H;
const GRID_X: u32 = ROW_NUM_W;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_SEL_BG: u32 = 0x1C2D4EFF;
const C_RED: u32    = 0xFF7B72FF;

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

fn parse_cell_ref(s: &str) -> Option<(usize, usize)> {
    let s = s.trim();
    let col_end = s.find(|c: char| c.is_ascii_digit())?;
    if col_end == 0 || col_end > 1 { return None; }
    let col_c = s.chars().next()?.to_ascii_uppercase();
    if col_c < 'A' || col_c > 'T' { return None; }
    let col = (col_c as usize) - ('A' as usize);
    let row = s[col_end..].parse::<usize>().ok()?.wrapping_sub(1);
    if row < ROWS { Some((row, col)) } else { None }
}

struct Sheet {
    cells:    Vec<Vec<String>>,
    cur_r:    usize,
    cur_c:    usize,
    editing:  bool,
    edit_buf: String,
    status:   String,
}

impl Sheet {
    fn new() -> Self {
        let mut cells = vec![vec![String::new(); COLS]; ROWS];
        cells[0][0] = "Item".into();
        cells[0][1] = "Price".into();
        cells[0][2] = "Qty".into();
        cells[0][3] = "Total".into();
        for (i, (name, price, qty)) in [("Apples","120","10"),("Oranges","80","5"),
                                         ("Bananas","60","8"),("Grapes","150","3")].iter().enumerate() {
            let r = i + 1;
            cells[r][0] = name.to_string();
            cells[r][1] = price.to_string();
            cells[r][2] = qty.to_string();
            cells[r][3] = format!("=B{}*C{}", r+1, r+1);
        }
        cells[5][0] = "Subtotal".into();
        cells[5][1] = "=SUM(B2:B5)".into();
        cells[5][2] = "=COUNT(C2:C5)".into();
        cells[5][3] = "=SUM(D2:D5)".into();
        cells[7][0] = "Avg price".into();
        cells[7][1] = "=AVG(B2:B5)".into();
        cells[8][0] = "Min price".into();
        cells[8][1] = "=MIN(B2:B5)".into();
        cells[9][0] = "Max price".into();
        cells[9][1] = "=MAX(B2:B5)".into();
        Sheet { cells, cur_r: 0, cur_c: 0, editing: false, edit_buf: String::new(), status: String::new() }
    }

    fn col_letter(c: usize) -> char { (b'A' + c as u8) as char }
    fn cell_name(r: usize, c: usize) -> String { format!("{}{}", Self::col_letter(c), r + 1) }

    fn eval_cell(&self, r: usize, c: usize) -> String {
        let mut v = Vec::new();
        self.eval(r, c, &mut v)
    }

    fn eval(&self, r: usize, c: usize, vis: &mut Vec<(usize,usize)>) -> String {
        if vis.contains(&(r, c)) { return "#CIRC".to_string(); }
        let raw = self.cells[r][c].clone();
        if !raw.starts_with('=') { return raw; }
        vis.push((r, c));
        let result = self.eval_expr(raw[1..].trim(), vis);
        vis.pop();
        result
    }

    fn range_vals(&self, range: &str, vis: &mut Vec<(usize,usize)>) -> Vec<i64> {
        let parts: Vec<&str> = range.trim().split(':').collect();
        if parts.len() == 1 {
            if let Some((r, c)) = parse_cell_ref(parts[0]) {
                if let Ok(v) = self.eval(r, c, vis).parse::<i64>() { return vec![v]; }
            }
            return vec![];
        }
        let (Some((r1,c1)), Some((r2,c2))) = (parse_cell_ref(parts[0]), parse_cell_ref(parts[1])) else {
            return vec![];
        };
        let mut vals = vec![];
        for row in r1.min(r2)..=r1.max(r2) {
            for col in c1.min(c2)..=c1.max(c2) {
                if let Ok(v) = self.eval(row, col, vis).parse::<i64>() { vals.push(v); }
            }
        }
        vals
    }

    fn eval_expr(&self, expr: &str, vis: &mut Vec<(usize,usize)>) -> String {
        let expr = expr.trim();
        if expr.is_empty() { return String::new(); }

        // Function calls
        for func in &["SUM","AVG","MIN","MAX","COUNT"] {
            let pfx = format!("{}(", func);
            if let Some(rest) = expr.strip_prefix(pfx.as_str()) {
                if let Some(inner) = rest.strip_suffix(')') {
                    let vals = self.range_vals(inner, vis);
                    let r: i64 = match *func {
                        "SUM"   => vals.iter().sum(),
                        "AVG"   => if vals.is_empty() { 0 } else { vals.iter().sum::<i64>() / vals.len() as i64 },
                        "MIN"   => vals.iter().copied().min().unwrap_or(0),
                        "MAX"   => vals.iter().copied().max().unwrap_or(0),
                        _       => vals.len() as i64,
                    };
                    return r.to_string();
                }
            }
        }

        let bytes = expr.as_bytes();

        // Find rightmost + or - at depth 0 (gives left-assoc)
        let mut depth = 0i32;
        let mut last_pm: Option<usize> = None;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'+' | b'-' if depth == 0 && i > 0 => { last_pm = Some(i); }
                _ => {}
            }
        }
        if let Some(pos) = last_pm {
            let l = self.eval_expr(&expr[..pos], vis);
            let r = self.eval_expr(&expr[pos+1..], vis);
            return match (l.parse::<i64>(), r.parse::<i64>()) {
                (Ok(lv), Ok(rv)) => (if bytes[pos] == b'+' { lv+rv } else { lv-rv }).to_string(),
                _ => "#ERR".to_string(),
            };
        }

        // Find rightmost * or /
        let mut last_md: Option<usize> = None;
        depth = 0;
        for (i, &b) in bytes.iter().enumerate() {
            match b {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b'*' | b'/' if depth == 0 => { last_md = Some(i); }
                _ => {}
            }
        }
        if let Some(pos) = last_md {
            let l = self.eval_expr(&expr[..pos], vis);
            let r = self.eval_expr(&expr[pos+1..], vis);
            return match (l.parse::<i64>(), r.parse::<i64>()) {
                (Ok(lv), Ok(rv)) => if bytes[pos] == b'*' {
                    (lv * rv).to_string()
                } else if rv != 0 { (lv / rv).to_string() } else { "#DIV0".to_string() },
                _ => "#ERR".to_string(),
            };
        }

        // Cell reference
        if let Some((r, c)) = parse_cell_ref(expr) {
            return self.eval(r, c, vis);
        }

        // Number literal
        if expr.parse::<i64>().is_ok() { return expr.to_string(); }
        "#ERR".to_string()
    }

    fn save(&self) -> Result<(), String> {
        let mut out = String::new();
        for row in &self.cells {
            for (ci, cell) in row.iter().enumerate() {
                if ci > 0 { out.push(','); }
                if cell.contains(',') || cell.contains('"') {
                    out.push('"');
                    for ch in cell.chars() { if ch == '"' { out.push('"'); } out.push(ch); }
                    out.push('"');
                } else { out.push_str(cell); }
            }
            out.push('\n');
        }
        std::fs::write("/data/spreadsheet2.csv", out).map_err(|e| e.to_string())
    }

    fn load(&mut self) -> Result<(), String> {
        let s = std::fs::read_to_string("/data/spreadsheet2.csv").map_err(|e| e.to_string())?;
        for (ri, line) in s.lines().take(ROWS).enumerate() {
            let mut ci = 0usize;
            let mut in_q = false;
            let mut fld = String::new();
            for ch in line.chars() {
                if in_q { if ch == '"' { in_q = false; } else { fld.push(ch); } }
                else if ch == '"' { in_q = true; }
                else if ch == ',' {
                    if ci < COLS { self.cells[ri][ci] = fld.clone(); }
                    ci += 1; fld.clear();
                } else { fld.push(ch); }
            }
            if ci < COLS { self.cells[ri][ci] = fld; }
        }
        Ok(())
    }
}

fn draw(s: &Sheet) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Spreadsheet v2");
    text(240, 16, C_HINT, "↑↓←→:nav  type:edit  Enter/Tab:confirm  Del:clear  Ctrl+W:save  Ctrl+L:load");

    // Formula bar
    fill(0, HEADER_H, W, FORMULA_H, C_CARD);
    fill(0, HEADER_H + FORMULA_H - 1, W, 1, C_BORDER);
    let cn = Sheet::cell_name(s.cur_r, s.cur_c);
    text(8, HEADER_H + 6, C_SEL, &format!("{}:", cn));
    let bar_content = if s.editing { format!("{}_", s.edit_buf) } else { s.cells[s.cur_r][s.cur_c].clone() };
    text(8 + (cn.len() as u32 + 2) * CHAR_W, HEADER_H + 6, C_TEXT, &bar_content);

    // Column headers
    fill(0, COL_HDR_Y, ROW_NUM_W, COL_HDR_H, C_HEADER);
    for c in 0..COLS {
        let cx = GRID_X + c as u32 * CELL_W;
        let sel = c == s.cur_c;
        fill(cx, COL_HDR_Y, CELL_W - 1, COL_HDR_H, if sel { C_SEL_BG } else { C_HEADER });
        fill(cx + CELL_W - 1, COL_HDR_Y, 1, COL_HDR_H, C_BORDER);
        let lbl = Sheet::col_letter(c).to_string();
        text(cx + (CELL_W - CHAR_W) / 2, COL_HDR_Y + 5, if sel { C_SEL } else { C_HINT }, &lbl);
    }
    fill(0, COL_HDR_Y + COL_HDR_H - 1, W, 1, C_BORDER);

    // Rows
    for r in 0..ROWS {
        let ry = GRID_Y + r as u32 * CELL_H;
        let is_cur_row = r == s.cur_r;
        fill(0, ry, ROW_NUM_W, CELL_H, if is_cur_row { C_SEL_BG } else { C_HEADER });
        text(4, ry + 5, if is_cur_row { C_SEL } else { C_HINT }, &format!("{:3}", r+1));
        fill(ROW_NUM_W - 1, ry, 1, CELL_H, C_BORDER);

        for c in 0..COLS {
            let cx = GRID_X + c as u32 * CELL_W;
            let is_sel = r == s.cur_r && c == s.cur_c;
            fill(cx, ry, CELL_W - 1, CELL_H, if is_sel { C_SEL_BG } else { C_BG });
            fill(cx + CELL_W - 1, ry, 1, CELL_H, C_BORDER);

            let disp = if is_sel && s.editing {
                s.edit_buf.clone()
            } else {
                s.eval_cell(r, c)
            };

            if !disp.is_empty() {
                let max_c = (CELL_W - 4) as usize / CHAR_W as usize;
                let t = if disp.len() > max_c { &disp[..max_c] } else { &disp };
                let is_num = disp.parse::<i64>().is_ok();
                let is_err = disp.starts_with('#');
                let is_formula = s.cells[r][c].starts_with('=');
                let col = if is_sel { C_TEXT }
                          else if is_err { C_RED }
                          else if is_formula { C_GREEN }
                          else if is_num { C_SEL }
                          else { C_TEXT };
                let tx = if (is_num || (is_formula && !is_err)) && !s.editing {
                    cx + CELL_W - 2 - t.len() as u32 * CHAR_W
                } else {
                    cx + 2
                };
                text(tx, ry + 5, col, t);
            }
            if is_sel { border(cx, ry, CELL_W - 1, CELL_H, C_SEL); }
        }
        fill(0, ry + CELL_H - 1, W, 1, C_BORDER);
    }

    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    if !s.status.is_empty() {
        text(8, sb_y + 6, C_HINT, &s.status);
    } else {
        let m = if s.editing { "EDIT" } else { "NORMAL" };
        let mc = if s.editing { C_ORANGE } else { C_HINT };
        text(8, sb_y + 6, mc, m);
        text(72, sb_y + 6, C_HINT, &format!("  {} rows × {} cols  |  formulas: =SUM =AVG =MIN =MAX =COUNT =cell_ref arithmetic", ROWS, COLS));
    }
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut sheet = Sheet::new();

    println!("@supervisor: raise spreadsheet2");
    let _ = io::stdout().flush();
    draw(&sheet);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }
        sheet.status.clear();

        if sheet.editing {
            match raw.as_str() {
                "\x1b" => { sheet.editing = false; sheet.edit_buf.clear(); }
                "" => {
                    sheet.cells[sheet.cur_r][sheet.cur_c] = sheet.edit_buf.clone();
                    sheet.edit_buf.clear(); sheet.editing = false;
                    if sheet.cur_r + 1 < ROWS { sheet.cur_r += 1; }
                }
                "\t" => {
                    sheet.cells[sheet.cur_r][sheet.cur_c] = sheet.edit_buf.clone();
                    sheet.edit_buf.clear(); sheet.editing = false;
                    if sheet.cur_c + 1 < COLS { sheet.cur_c += 1; }
                }
                "\x7f" => { sheet.edit_buf.pop(); }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f { sheet.edit_buf.push(b as char); }
                }
                _ => {}
            }
        } else {
            match raw.as_str() {
                "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
                "\x1b[A" => { if sheet.cur_r > 0 { sheet.cur_r -= 1; } }
                "\x1b[B" => { if sheet.cur_r + 1 < ROWS { sheet.cur_r += 1; } }
                "\x1b[C" => { if sheet.cur_c + 1 < COLS { sheet.cur_c += 1; } }
                "\x1b[D" => { if sheet.cur_c > 0 { sheet.cur_c -= 1; } }
                "\x7f" => { sheet.cells[sheet.cur_r][sheet.cur_c].clear(); }
                "\x17" => {
                    match sheet.save() {
                        Ok(()) => sheet.status = "Saved /data/spreadsheet2.csv".to_string(),
                        Err(e) => sheet.status = format!("Save error: {}", e),
                    }
                }
                "\x0c" => {
                    match sheet.load() {
                        Ok(()) => sheet.status = "Loaded /data/spreadsheet2.csv".to_string(),
                        Err(e) => sheet.status = format!("Load error: {}", e),
                    }
                }
                s if s.len() == 1 => {
                    let b = s.as_bytes()[0];
                    if b >= 0x20 && b < 0x7f {
                        sheet.editing = true;
                        sheet.edit_buf = (b as char).to_string();
                    }
                }
                _ => {}
            }
        }
        draw(&sheet);
    }
}
