// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1240;
const H: u32 = 760;
const C_BG: u32     = 0x0D1117FF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_ERR: u32    = 0xFF7B72FF;

const IMG_X: u32 = 20;
const IMG_Y: u32 = 50;
const IMG_AREA_W: u32 = W - 40;
const IMG_AREA_H: u32 = H - 80;
// Display at 4×4 blocks → max 160 source cols × 100 source rows visible at once
const BLOCK: u32 = 4;
const MAX_COLS: u32 = IMG_AREA_W / BLOCK; // 300
const MAX_ROWS: u32 = IMG_AREA_H / BLOCK; // 177

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

fn list_ppm() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/data") else { return Vec::new() };
    let mut files: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| format!("/data/{}", e.file_name().to_string_lossy()))
        .filter(|n| n.ends_with(".ppm"))
        .collect();
    files.sort();
    files
}

/// Parse PPM P6 binary. Returns (width, height, flat RGB Vec<u8>).
fn load_ppm(path: &str) -> Result<(u32, u32, Vec<u8>), String> {
    let data = std::fs::read(path).map_err(|e| format!("{e}"))?;
    let mut pos = 0usize;

    let skip_ws_comments = |pos: &mut usize, data: &[u8]| {
        loop {
            while *pos < data.len() && (data[*pos] == b' ' || data[*pos] == b'\t' || data[*pos] == b'\r' || data[*pos] == b'\n') {
                *pos += 1;
            }
            if *pos < data.len() && data[*pos] == b'#' {
                while *pos < data.len() && data[*pos] != b'\n' { *pos += 1; }
            } else {
                break;
            }
        }
    };

    let read_uint = |pos: &mut usize, data: &[u8]| -> Option<u32> {
        let start = *pos;
        while *pos < data.len() && data[*pos].is_ascii_digit() { *pos += 1; }
        if *pos == start { return None; }
        std::str::from_utf8(&data[start..*pos]).ok()?.parse().ok()
    };

    if !data.starts_with(b"P6") { return Err("not a P6 PPM".into()); }
    pos += 2;
    skip_ws_comments(&mut pos, &data);
    let w = read_uint(&mut pos, &data).ok_or("no width")?;
    skip_ws_comments(&mut pos, &data);
    let h = read_uint(&mut pos, &data).ok_or("no height")?;
    skip_ws_comments(&mut pos, &data);
    let maxval = read_uint(&mut pos, &data).ok_or("no maxval")?;
    // skip exactly one whitespace byte after maxval
    if pos < data.len() { pos += 1; }

    let expected = (w * h * 3) as usize;
    if data.len() < pos + expected {
        return Err(format!("truncated: need {expected} bytes, got {}", data.len() - pos));
    }

    let mut pixels = data[pos..pos + expected].to_vec();
    // Scale if maxval != 255
    if maxval != 255 && maxval > 0 {
        for b in pixels.iter_mut() {
            *b = (*b as u32 * 255 / maxval) as u8;
        }
    }
    Ok((w, h, pixels))
}

fn render_image(pixels: &[u8], src_w: u32, src_h: u32, scroll_row: u32) {
    let disp_cols = MAX_COLS.min(src_w);
    let disp_rows = MAX_ROWS.min(src_h.saturating_sub(scroll_row));

    fill(IMG_X, IMG_Y, disp_cols * BLOCK, disp_rows * BLOCK, 0x000000FF);

    for row in 0..disp_rows {
        let src_y = (scroll_row + row) as usize;
        if src_y >= src_h as usize { break; }

        let mut run_x = 0u32;
        let mut run_rgba = 0u32;

        for col in 0..=disp_cols {
            let rgba = if col < disp_cols {
                let src_x = (col * src_w / disp_cols) as usize;
                let i = (src_y * src_w as usize + src_x) * 3;
                let r = pixels[i] as u32;
                let g = pixels[i + 1] as u32;
                let b = pixels[i + 2] as u32;
                (r << 24) | (g << 16) | (b << 8) | 0xFF
            } else {
                !run_rgba // force flush on last col
            };

            if col == 0 {
                run_x = 0;
                run_rgba = rgba;
            } else if rgba != run_rgba || col == disp_cols {
                let px = IMG_X + run_x * BLOCK;
                let py = IMG_Y + row * BLOCK;
                let pw = (col - run_x) * BLOCK;
                println!("VYOMA_DRAW:fill_rect:{px},{py},{pw},{BLOCK},{run_rgba:#010x}");
                run_x = col;
                run_rgba = rgba;
            }
        }
    }
}

fn draw_list(files: &[String], cursor: usize) {
    fill(0, 0, W, H, C_BG);
    text(20, 14, C_ACCENT, "Image Viewer  —  /data/*.ppm");
    fill(0, 34, W, 1, C_BORDER);
    if files.is_empty() {
        text(20, 60, C_DIM, "No .ppm files found in /data. Use `screenshot /data/out.ppm` to create one.");
    } else {
        for (i, f) in files.iter().enumerate() {
            let ty = 46 + i as u32 * 20;
            let tc = if i == cursor { C_ACCENT } else { C_DIM };
            text(20, ty, tc, f);
        }
        text(20, H - 18, C_HINT, "↑↓: select   Enter: open   Ctrl+C: quit");
    }
    flush();
}

fn draw_image(files: &[String], idx: usize, pixels: &[u8], src_w: u32, src_h: u32, scroll: u32, status: &str) {
    fill(0, 0, W, H, C_BG);
    let fname = files.get(idx).map(|s| s.as_str()).unwrap_or("");
    let hdr = format!("{fname}  ({src_w}×{src_h})  [{}/{}]", idx + 1, files.len());
    text(20, 14, C_ACCENT, &hdr);
    fill(0, 34, W, 1, C_BORDER);

    if !status.is_empty() {
        text(20, IMG_Y + 10, C_ERR, status);
    } else {
        render_image(pixels, src_w, src_h, scroll);
    }

    let scroll_info = format!("row {scroll}/{src_h}");
    text(20, H - 18, C_HINT,
        &format!("↑↓: scroll   ←→: prev/next   Esc/q: back   {scroll_info}"));
    flush();
}

enum View {
    List { cursor: usize },
    Image { idx: usize, pixels: Vec<u8>, w: u32, h: u32, scroll: u32, status: String },
}

fn main() {
    let stdin = io::stdin();
    let files = list_ppm();

    let mut view = if files.len() == 1 {
        // Auto-load single file
        match load_ppm(&files[0]) {
            Ok((w, h, px)) => View::Image { idx: 0, pixels: px, w, h, scroll: 0, status: String::new() },
            Err(e) => View::Image { idx: 0, pixels: Vec::new(), w: 0, h: 0, scroll: 0, status: e },
        }
    } else {
        View::List { cursor: 0 }
    };

    match &view {
        View::List { cursor } => draw_list(&files, *cursor),
        View::Image { idx, pixels, w, h, scroll, status } =>
            draw_image(&files, *idx, pixels, *w, *h, *scroll, status),
    }

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match &mut view {
            View::List { cursor } => {
                match raw.as_str() {
                    "\x03" => {
                        fill(0, 0, W, H, 0x0D1117FF);
                        flush();
                        std::process::exit(0);
                    }
                    "\x1b[A" => {
                        if *cursor > 0 { *cursor -= 1; }
                        draw_list(&files, *cursor);
                    }
                    "\x1b[B" => {
                        if *cursor + 1 < files.len() { *cursor += 1; }
                        draw_list(&files, *cursor);
                    }
                    "" => {
                        let idx = *cursor;
                        if let Some(path) = files.get(idx) {
                            let (new_view) = match load_ppm(path) {
                                Ok((w, h, px)) => View::Image { idx, pixels: px, w, h, scroll: 0, status: String::new() },
                                Err(e) => View::Image { idx, pixels: Vec::new(), w: 0, h: 0, scroll: 0, status: e },
                            };
                            view = new_view;
                            if let View::Image { idx, ref pixels, w, h, scroll, ref status } = view {
                                draw_image(&files, idx, pixels, w, h, scroll, status);
                            }
                        }
                    }
                    _ => {}
                }
            }
            View::Image { idx, pixels, w, h, scroll, status } => {
                match raw.as_str() {
                    "\x03" | "\x1b" | "q" => {
                        if files.len() <= 1 {
                            fill(0, 0, W, H, 0x0D1117FF);
                            flush();
                            std::process::exit(0);
                        }
                        let cur = *idx;
                        view = View::List { cursor: cur };
                        if let View::List { cursor } = &view {
                            draw_list(&files, *cursor);
                        }
                    }
                    "\x1b[A" => {
                        if *scroll > 0 { *scroll -= 1; }
                        draw_image(&files, *idx, pixels, *w, *h, *scroll, status);
                    }
                    "\x1b[B" => {
                        if *scroll + MAX_ROWS < *h { *scroll += 1; }
                        draw_image(&files, *idx, pixels, *w, *h, *scroll, status);
                    }
                    "\x1b[D" => {
                        // prev file
                        let new_idx = if *idx > 0 { *idx - 1 } else { files.len().saturating_sub(1) };
                        if let Some(path) = files.get(new_idx) {
                            match load_ppm(path) {
                                Ok((nw, nh, px)) => {
                                    *idx = new_idx; *pixels = px; *w = nw; *h = nh; *scroll = 0; status.clear();
                                }
                                Err(e) => {
                                    *idx = new_idx; *pixels = Vec::new(); *w = 0; *h = 0; *scroll = 0; *status = e;
                                }
                            }
                        }
                        draw_image(&files, *idx, pixels, *w, *h, *scroll, status);
                    }
                    "\x1b[C" => {
                        // next file
                        let new_idx = (*idx + 1) % files.len().max(1);
                        if let Some(path) = files.get(new_idx) {
                            match load_ppm(path) {
                                Ok((nw, nh, px)) => {
                                    *idx = new_idx; *pixels = px; *w = nw; *h = nh; *scroll = 0; status.clear();
                                }
                                Err(e) => {
                                    *idx = new_idx; *pixels = Vec::new(); *w = 0; *h = 0; *scroll = 0; *status = e;
                                }
                            }
                        }
                        draw_image(&files, *idx, pixels, *w, *h, *scroll, status);
                    }
                    _ => {}
                }
            }
        }
    }
}
