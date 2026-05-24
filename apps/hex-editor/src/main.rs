// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1280;
const H: u32 = 760;
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
const C_EDIT: u32   = 0x2D333BFF;

const BYTES_PER_ROW: usize = 16;
const ROW_H: u32 = 18;
const CONTENT_Y: u32 = 52;
const VIS_ROWS: usize = ((H - CONTENT_Y - 30) / ROW_H) as usize; // ~37

// Column positions (in chars, each char 8px)
const COL_ADDR: u32 = 10;   // "00000000:"
const COL_HEX: u32  = 100;  // 16 hex pairs
const COL_ASCII: u32 = 660; // 16 ASCII chars

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

enum Mode {
    PathInput(String),
    HexView {
        path:      String,
        data:      Vec<u8>,
        scroll:    usize,    // top visible row (0-based)
        cursor:    usize,    // cursor row
        edit_byte: Option<usize>, // byte index being edited
        edit_buf:  String,   // 0–2 hex chars typed
        status:    String,
    },
}

fn draw_path_input(input: &str, error: &str) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Hex Editor");
    fill(0, 34, W, 1, C_BORDER);
    text(20, 52, C_DIM, "File path:");
    fill(20, 68, W - 40, 32, C_FIELD);
    border(20, 68, W - 40, 32, C_BORDER);
    let display = if input.is_empty() { "/data/" } else { input };
    let tc = if input.is_empty() { C_HINT } else { C_TITLE };
    text(30, 76, tc, display);
    if !error.is_empty() {
        text(20, 112, C_ERR, error);
    }
    text(20, H - 18, C_HINT, "Enter: open   Ctrl+C: quit");
    flush();
}

fn hex_row(addr: usize, bytes: &[u8], cursor_row: usize, row_idx: usize, edit_byte: Option<usize>, edit_buf: &str) -> String {
    let is_cursor = row_idx == cursor_row;
    let _ = is_cursor;
    // Build hex portion
    let mut hex = String::new();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 { hex.push(' '); }
        let byte_idx = addr + i;
        if edit_byte == Some(byte_idx) {
            // show edit buffer
            let disp = format!("{}{}", edit_buf, if edit_buf.len() < 2 { "_" } else { "" });
            hex.push_str(&disp[..disp.len().min(2)]);
        } else {
            hex.push_str(&format!("{:02X}", b));
        }
    }
    // Pad if < 16 bytes
    let pad = (BYTES_PER_ROW - bytes.len()) * 3;
    hex.push_str(&" ".repeat(pad));
    hex
}

fn ascii_row(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if b >= 0x20 && b < 0x7F { b as char } else { '.' }).collect()
}

fn draw_hex_view(path: &str, data: &[u8], scroll: usize, cursor: usize, edit_byte: Option<usize>, edit_buf: &str, status: &str) {
    fill(0, 0, W, H, C_BG);
    let info = format!("Hex Editor  —  {path}  ({} bytes)", data.len());
    text(20, 14, C_ACCENT, &info);
    fill(0, 34, W, 1, C_BORDER);

    // Column headers
    text(COL_ADDR, CONTENT_Y - 16, C_DIM, "ADDRESS");
    text(COL_HEX,  CONTENT_Y - 16, C_DIM, "00 01 02 03 04 05 06 07 08 09 0A 0B 0C 0D 0E 0F");
    text(COL_ASCII,CONTENT_Y - 16, C_DIM, "ASCII");
    fill(0, CONTENT_Y - 2, W, 1, C_BORDER);

    let total_rows = (data.len() + BYTES_PER_ROW - 1) / BYTES_PER_ROW;
    for vis_i in 0..VIS_ROWS {
        let row_idx = scroll + vis_i;
        if row_idx >= total_rows { break; }
        let addr = row_idx * BYTES_PER_ROW;
        let end = (addr + BYTES_PER_ROW).min(data.len());
        let bytes = &data[addr..end];
        let ry = CONTENT_Y + vis_i as u32 * ROW_H;

        let is_cursor = row_idx == cursor;
        if is_cursor {
            fill(0, ry, W, ROW_H, C_SEL);
        }
        let tc = if is_cursor { C_TITLE } else { C_DIM };

        text(COL_ADDR, ry + 2, C_HINT, &format!("{addr:08X}"));
        let hex = hex_row(addr, bytes, cursor, row_idx, edit_byte, edit_buf);
        text(COL_HEX, ry + 2, tc, &hex);
        let ascii = ascii_row(bytes);
        text(COL_ASCII, ry + 2, C_DIM, &ascii);
    }

    fill(0, H - 30, W, 1, C_BORDER);
    if !status.is_empty() {
        let sc = if status.starts_with("Error") || status.starts_with("Saved") { C_OK } else { C_DIM };
        let sc = if status.starts_with("Error") { C_ERR } else { sc };
        text(20, H - 18, sc, status);
    } else {
        let edit_hint = if edit_byte.is_some() {
            "type 2 hex digits   Esc: cancel"
        } else {
            "↑↓: scroll   u/d: page   e: edit byte   Ctrl+W: save   Ctrl+C: quit"
        };
        text(20, H - 18, C_HINT, edit_hint);
    }
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut mode = Mode::PathInput(String::new());
    let mut path_error = String::new();

    draw_path_input("", "");

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match &mut mode {
            Mode::PathInput(input) => {
                match raw.as_str() {
                    "\x03" => {
                        fill(0, 0, W, H, 0x0D1117FF);
                        flush();
                        std::process::exit(0);
                    }
                    "\x7f" => {
                        input.pop();
                        path_error.clear();
                        draw_path_input(input, "");
                    }
                    "" => {
                        let path = input.clone();
                        match std::fs::read(&path) {
                            Ok(data) => {
                                let data_len = data.len();
                                mode = Mode::HexView {
                                    path: path.clone(),
                                    data,
                                    scroll: 0,
                                    cursor: 0,
                                    edit_byte: None,
                                    edit_buf: String::new(),
                                    status: format!("{data_len} bytes loaded"),
                                };
                                if let Mode::HexView { path, data, scroll, cursor, edit_byte, edit_buf, status } = &mode {
                                    draw_hex_view(path, data, *scroll, *cursor, *edit_byte, edit_buf, status);
                                }
                            }
                            Err(e) => {
                                path_error = format!("Error: {e}");
                                draw_path_input(input, &path_error);
                            }
                        }
                    }
                    ch if ch.len() == 1 => {
                        let c = ch.chars().next().unwrap();
                        if c.is_ascii_graphic() || c == ' ' || c == '/' {
                            input.push(c);
                            path_error.clear();
                            draw_path_input(input, "");
                        }
                    }
                    _ => {}
                }
            }
            Mode::HexView { path, data, scroll, cursor, edit_byte, edit_buf, status } => {
                let total_rows = (data.len() + BYTES_PER_ROW - 1) / BYTES_PER_ROW;

                // Edit mode
                if let Some(byte_idx) = *edit_byte {
                    match raw.as_str() {
                        "\x1b" => {
                            *edit_byte = None;
                            edit_buf.clear();
                            status.clear();
                        }
                        ch if ch.len() == 1 && ch.chars().next().map(|c| c.is_ascii_hexdigit()).unwrap_or(false) => {
                            let c = ch.chars().next().unwrap().to_ascii_uppercase();
                            edit_buf.push(c);
                            if edit_buf.len() == 2 {
                                let val = u8::from_str_radix(edit_buf.as_str(), 16).unwrap_or(0);
                                if byte_idx < data.len() { data[byte_idx] = val; }
                                let next = byte_idx + 1;
                                if next < data.len() {
                                    *edit_byte = Some(next);
                                } else {
                                    *edit_byte = None;
                                }
                                edit_buf.clear();
                                *status = format!("byte {:08X} set to {:02X}", byte_idx, val);
                            }
                        }
                        _ => {}
                    }
                    draw_hex_view(path, data, *scroll, *cursor, *edit_byte, edit_buf, status);
                    continue;
                }

                match raw.as_str() {
                    "\x03" => {
                        // Return to path input
                        let old_path = path.clone();
                        mode = Mode::PathInput(old_path);
                        path_error.clear();
                        if let Mode::PathInput(inp) = &mode {
                            draw_path_input(inp, "");
                        }
                    }
                    "\x17" => {
                        // Ctrl+W: save
                        match std::fs::write(path.as_str(), data.as_slice()) {
                            Ok(()) => *status = format!("Saved {} bytes to {path}", data.len()),
                            Err(e) => *status = format!("Error saving: {e}"),
                        }
                        draw_hex_view(path, data, *scroll, *cursor, *edit_byte, edit_buf, status);
                    }
                    "\x1b[A" => {
                        if *cursor > 0 { *cursor -= 1; }
                        if *cursor < *scroll { *scroll = *cursor; }
                        status.clear();
                        draw_hex_view(path, data, *scroll, *cursor, *edit_byte, edit_buf, status);
                    }
                    "\x1b[B" => {
                        if *cursor + 1 < total_rows { *cursor += 1; }
                        if *cursor >= *scroll + VIS_ROWS { *scroll = *cursor + 1 - VIS_ROWS; }
                        status.clear();
                        draw_hex_view(path, data, *scroll, *cursor, *edit_byte, edit_buf, status);
                    }
                    "u" => {
                        // Page up (16 rows)
                        *cursor = cursor.saturating_sub(16);
                        *scroll = scroll.saturating_sub(16);
                        status.clear();
                        draw_hex_view(path, data, *scroll, *cursor, *edit_byte, edit_buf, status);
                    }
                    "d" => {
                        // Page down (16 rows)
                        let new_cursor = (*cursor + 16).min(total_rows.saturating_sub(1));
                        *cursor = new_cursor;
                        if *cursor >= *scroll + VIS_ROWS {
                            *scroll = (*cursor + 1).saturating_sub(VIS_ROWS);
                        }
                        status.clear();
                        draw_hex_view(path, data, *scroll, *cursor, *edit_byte, edit_buf, status);
                    }
                    "e" | "" => {
                        // Enter edit mode for first byte of cursor row
                        let byte_idx = *cursor * BYTES_PER_ROW;
                        if byte_idx < data.len() {
                            *edit_byte = Some(byte_idx);
                            edit_buf.clear();
                            *status = format!("Editing byte {:08X}", byte_idx);
                        }
                        draw_hex_view(path, data, *scroll, *cursor, *edit_byte, edit_buf, status);
                    }
                    _ => {}
                }
            }
        }
    }
}
