use std::io::{self, BufRead, Write};

const W: u32 = 760;
const H: u32 = 680;
const HEADER_H: u32 = 36;
const INPUT_Y: u32  = HEADER_H + 8;
const INPUT_H: u32  = 36;
const QR_Y: u32     = INPUT_Y + INPUT_H + 16;
const MODULE: u32   = 12; // pixels per QR module
const QR_SIZE: usize = 21; // Version 1 = 21×21

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x161B22FF;
const C_BORDER: u32 = 0x30363DFF;
const C_SEL: u32    = 0x58A6FFFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_DIM: u32    = 0x8B949EFF;
const C_HINT: u32   = 0x6E7681FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_QR_BLACK: u32 = 0x000000FF;
const C_QR_WHITE: u32 = 0xFFFFFFFF;

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

// Minimal QR Version 1 generator
// Returns a 21×21 bool grid: true=dark module
fn generate_qr(input: &str) -> [[bool; QR_SIZE]; QR_SIZE] {
    let mut grid = [[false; QR_SIZE]; QR_SIZE];

    // Finder patterns (top-left, top-right, bottom-left)
    let draw_finder = |grid: &mut [[bool; QR_SIZE]; QR_SIZE], r: usize, c: usize| {
        for dr in 0..7 {
            for dc in 0..7 {
                let on = dr == 0 || dr == 6 || dc == 0 || dc == 6
                    || (dr >= 2 && dr <= 4 && dc >= 2 && dc <= 4);
                if r + dr < QR_SIZE && c + dc < QR_SIZE {
                    grid[r + dr][c + dc] = on;
                }
            }
        }
    };
    draw_finder(&mut grid, 0, 0);
    draw_finder(&mut grid, 0, 14);
    draw_finder(&mut grid, 14, 0);

    // Timing patterns
    for i in 8..13 {
        grid[6][i] = i % 2 == 0;
        grid[i][6] = i % 2 == 0;
    }

    // Dark module
    grid[13][8] = true;

    // Encode input bytes into data modules (simplified: place LSB-first bytes)
    let bytes: Vec<u8> = input.bytes().take(17).collect();
    // Byte mode indicator: 0100 (4 bits), length (8 bits), then data
    let mut bits: Vec<bool> = Vec::new();
    bits.extend_from_slice(&[false, true, false, false]); // 0b0100
    let len = bytes.len().min(17);
    for b in (0..8).rev() { bits.push((len >> b) & 1 == 1); }
    for byte in &bytes {
        for b in (0..8).rev() { bits.push((byte >> b) & 1 == 1); }
    }
    // Terminator
    for _ in 0..4 { bits.push(false); }

    // Data module positions (simplified — place in non-reserved area)
    let reserved = |r: usize, c: usize| -> bool {
        (r < 9 && c < 9) || (r < 9 && c >= 13) || (r >= 13 && c < 9)
        || r == 6 || c == 6
    };

    let mut bit_idx = 0usize;
    let mut col_dir_up = true;
    let mut col = 20i32;
    while col >= 0 && bit_idx < bits.len() {
        if col == 6 { col -= 1; continue; }
        let row_range: Vec<i32> = if col_dir_up {
            (0..21).rev().collect()
        } else {
            (0..21).collect()
        };
        for row in row_range {
            for dc in [0i32, -1] {
                let c = (col + dc) as usize;
                let r = row as usize;
                if !reserved(r, c) && bit_idx < bits.len() {
                    grid[r][c] = bits[bit_idx];
                    bit_idx += 1;
                }
            }
        }
        col -= 2;
        col_dir_up = !col_dir_up;
    }

    grid
}

fn draw(input: &str, qr: Option<&[[bool; QR_SIZE]; QR_SIZE]>, generated: bool) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    text(16, 10, C_TEXT, "QR Code Viewer");
    text(W - 240, 10, C_HINT, "Enter: generate  c: copy  Esc: close");

    // Input field
    fill(16, INPUT_Y, W - 32, INPUT_H, 0x21262DFF);
    border(16, INPUT_Y, W - 32, INPUT_H, C_BORDER);
    let prompt = format!("{}_{}", input, "");
    let display: String = format!("  {}_", input);
    let tc = if input.is_empty() { C_HINT } else { C_TEXT };
    let disp = if input.is_empty() { "  Enter URL or text…" } else { &display };
    text(24, INPUT_Y + 10, tc, disp);

    let char_count = format!("{}/17 chars", input.len().min(17));
    text(W - 100, INPUT_Y + 10, C_HINT, &char_count);

    // QR code
    let qr_x = (W - QR_SIZE as u32 * MODULE) / 2;
    if let Some(grid) = qr {
        // White border
        fill(qr_x - 4, QR_Y - 4, QR_SIZE as u32 * MODULE + 8, QR_SIZE as u32 * MODULE + 8, C_QR_WHITE);
        for r in 0..QR_SIZE {
            for c in 0..QR_SIZE {
                let color = if grid[r][c] { C_QR_BLACK } else { C_QR_WHITE };
                let mx = qr_x + c as u32 * MODULE;
                let my = QR_Y + r as u32 * MODULE;
                fill(mx, my, MODULE, MODULE, color);
            }
        }

        if generated {
            let label: String = input.chars().take(40).collect();
            let lw = label.len() as u32 * 8;
            text((W - lw) / 2, QR_Y + QR_SIZE as u32 * MODULE + 16, C_DIM, &label);
            text(W / 2 - 60, QR_Y + QR_SIZE as u32 * MODULE + 36, C_GREEN, "c: copy to clipboard");
        }
    } else {
        // Placeholder
        border(qr_x, QR_Y, QR_SIZE as u32 * MODULE, QR_SIZE as u32 * MODULE, C_BORDER);
        text(qr_x + 40, QR_Y + QR_SIZE as u32 * MODULE / 2 - 8, C_HINT, "Type text and press Enter");
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut input = String::new();
    let mut qr_grid: Option<[[bool; QR_SIZE]; QR_SIZE]> = None;
    let mut generated = false;

    println!("@supervisor: raise qr-viewer");
    let _ = io::stdout().flush();

    draw(&input, None, false);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x7f" => {
                input.pop();
                generated = false;
                qr_grid = None;
                draw(&input, qr_grid.as_ref(), false);
            }
            "" => {
                if !input.is_empty() {
                    let grid = generate_qr(&input);
                    qr_grid = Some(grid);
                    generated = true;
                }
                draw(&input, qr_grid.as_ref(), generated);
            }
            "c" | "C" => {
                if generated {
                    println!("@supervisor: clipboard-set {input}");
                    let _ = io::stdout().flush();
                }
            }
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if (c.is_ascii_graphic() || c == ' ') && input.len() < 17 {
                    input.push(c);
                    generated = false;
                    qr_grid = None;
                }
                draw(&input, qr_grid.as_ref(), false);
            }
            _ => {}
        }
    }
}
