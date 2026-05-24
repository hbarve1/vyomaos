// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 760;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x1F4068FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ERR: u32    = 0xFF7B72FF;

const HEADER_Y: u32   = 52;
const ROW_H: u32      = 18;
const COL_W: u32      = 160; // fixed column width in pixels (20 chars × 8px)
const VIS_ROWS: usize = ((H - HEADER_Y - ROW_H - 36) / ROW_H) as usize;
const VIS_COLS: usize = (W / COL_W) as usize; // ~8 columns visible

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur    = String::new();
    let mut in_q   = false;
    let mut chars  = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if !in_q => in_q = true,
            '"' if in_q  => {
                if chars.peek() == Some(&'"') { chars.next(); cur.push('"'); }
                else { in_q = false; }
            }
            ',' if !in_q => { fields.push(cur.trim().to_string()); cur = String::new(); }
            other        => cur.push(other),
        }
    }
    fields.push(cur.trim().to_string());
    fields
}

fn load_csv(path: &str) -> Result<Vec<Vec<String>>, String> {
    let s = std::fs::read_to_string(path).map_err(|e| format!("{e}"))?;
    Ok(s.lines().filter(|l| !l.trim().is_empty()).map(parse_csv_line).collect())
}

fn list_csv() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/data") else { return Vec::new() };
    let mut files: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| format!("/data/{}", e.file_name().to_string_lossy()))
        .filter(|n| n.ends_with(".csv"))
        .collect();
    files.sort();
    files
}

fn draw_list(files: &[String], cursor: usize) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "CSV Viewer  —  /data/*.csv");
    fill(0, 34, W, 1, C_BORDER);
    if files.is_empty() {
        text(20, 60, C_DIM, "No .csv files found in /data.");
    } else {
        for (i, f) in files.iter().enumerate() {
            let ty = 48 + i as u32 * 20;
            let tc = if i == cursor { C_ACCENT } else { C_DIM };
            text(20, ty, tc, f);
        }
        text(20, H - 18, C_HINT, "↑↓: select   Enter: open   Ctrl+C: quit");
    }
    flush();
}

fn draw_table(files: &[String], idx: usize, rows: &[Vec<String>], row_scroll: usize, col_scroll: usize) {
    fill(0, 0, W, H, C_BG);
    let fname = files.get(idx).map(|s| s.as_str()).unwrap_or("(unknown)");
    let data_rows = rows.len().saturating_sub(1);
    let ncols = rows.first().map(|r| r.len()).unwrap_or(0);
    let hdr = format!("{fname}  ({data_rows} rows × {ncols} cols)  [{}/{}]", idx + 1, files.len());
    text(20, 14, C_ACCENT, &hdr);
    fill(0, 34, W, 1, C_BORDER);

    if rows.is_empty() {
        text(20, 60, C_DIM, "Empty file.");
        flush();
        return;
    }

    let headers = &rows[0];
    // Draw header row
    fill(0, HEADER_Y, W, ROW_H, 0x161B22FF);
    for (ci, col) in (col_scroll..).take(VIS_COLS).enumerate() {
        let cx = 20 + ci as u32 * COL_W;
        if cx + COL_W > W { break; }
        let label = headers.get(col).map(|s| s.as_str()).unwrap_or("");
        let disp: String = label.chars().take(19).collect();
        text(cx, HEADER_Y + 3, C_ACCENT, &disp);
        // Column divider
        fill(cx + COL_W - 1, HEADER_Y, 1, ROW_H, C_BORDER);
    }
    fill(0, HEADER_Y + ROW_H, W, 1, C_BORDER);

    // Data rows
    let data_start = 1usize;
    let end = (data_start + row_scroll + VIS_ROWS).min(rows.len());
    for (vis_i, row_i) in (data_start + row_scroll..end).enumerate() {
        let ry = HEADER_Y + ROW_H + 2 + vis_i as u32 * ROW_H;
        if vis_i % 2 == 0 { fill(0, ry, W, ROW_H, 0x0D1117FF); }
        else              { fill(0, ry, W, ROW_H, 0x10161DFF); }
        let row = &rows[row_i];
        for (ci, col) in (col_scroll..).take(VIS_COLS).enumerate() {
            let cx = 20 + ci as u32 * COL_W;
            if cx + COL_W > W { break; }
            let val = row.get(col).map(|s| s.as_str()).unwrap_or("");
            let disp: String = val.chars().take(19).collect();
            text(cx, ry + 2, C_DIM, &disp);
        }
    }

    fill(0, H - 30, W, 1, C_BORDER);
    text(20, H - 18, C_HINT,
        &format!("↑↓: scroll rows   ←→: scroll cols   row {}/{data_rows}  col {}/{ncols}   q: back   Ctrl+C: quit",
            row_scroll + 1, col_scroll + 1));
    flush();
}

enum View {
    List { cursor: usize },
    Table { idx: usize, rows: Vec<Vec<String>>, row_scroll: usize, col_scroll: usize },
}

fn main() {
    let stdin = io::stdin();
    let files = list_csv();

    let mut view = if files.len() == 1 {
        match load_csv(&files[0]) {
            Ok(rows) => View::Table { idx: 0, rows, row_scroll: 0, col_scroll: 0 },
            Err(e)   => { eprintln!("{e}"); View::List { cursor: 0 } }
        }
    } else {
        View::List { cursor: 0 }
    };

    match &view {
        View::List { cursor } => draw_list(&files, *cursor),
        View::Table { idx, rows, row_scroll, col_scroll } =>
            draw_table(&files, *idx, rows, *row_scroll, *col_scroll),
    }

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match &mut view {
            View::List { cursor } => match raw.as_str() {
                "\x03" => { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
                "\x1b[A" => { if *cursor > 0 { *cursor -= 1; } draw_list(&files, *cursor); }
                "\x1b[B" => { if *cursor + 1 < files.len() { *cursor += 1; } draw_list(&files, *cursor); }
                "" => {
                    let idx = *cursor;
                    if let Some(path) = files.get(idx) {
                        match load_csv(path) {
                            Ok(rows) => {
                                view = View::Table { idx, rows, row_scroll: 0, col_scroll: 0 };
                                if let View::Table { idx, rows, row_scroll, col_scroll } = &view {
                                    draw_table(&files, *idx, rows, *row_scroll, *col_scroll);
                                }
                            }
                            Err(e) => { eprintln!("load error: {e}"); }
                        }
                    }
                }
                _ => {}
            },
            View::Table { idx, rows, row_scroll, col_scroll } => {
                let data_rows = rows.len().saturating_sub(1);
                let ncols = rows.first().map(|r| r.len()).unwrap_or(0);
                match raw.as_str() {
                    "\x03" | "q" => {
                        if files.len() <= 1 { fill(0,0,W,H,C_BG); flush(); std::process::exit(0); }
                        let cur = *idx;
                        view = View::List { cursor: cur };
                        if let View::List { cursor } = &view { draw_list(&files, *cursor); }
                    }
                    "\x1b[A" => {
                        if *row_scroll > 0 { *row_scroll -= 1; }
                        draw_table(&files, *idx, rows, *row_scroll, *col_scroll);
                    }
                    "\x1b[B" => {
                        if *row_scroll + VIS_ROWS < data_rows { *row_scroll += 1; }
                        draw_table(&files, *idx, rows, *row_scroll, *col_scroll);
                    }
                    "\x1b[D" => {
                        if *col_scroll > 0 { *col_scroll -= 1; }
                        draw_table(&files, *idx, rows, *row_scroll, *col_scroll);
                    }
                    "\x1b[C" => {
                        if *col_scroll + VIS_COLS < ncols { *col_scroll += 1; }
                        draw_table(&files, *idx, rows, *row_scroll, *col_scroll);
                    }
                    _ => {}
                }
            }
        }
    }
}
