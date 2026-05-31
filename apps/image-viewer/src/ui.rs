// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Drawing and rendering functions for the image viewer UI.

use std::io::{self, Write};

use crate::{
    basename, file_size, fmt_size, png_dimensions, App, Mode,
    CONTENT_H, CONTENT_W, CONTENT_Y, H, LINE_H, W,
};

// --- Colors ---
const C_BG: u32 = 0x1E1E2EFF;
const C_TITLE_BG: u32 = 0x181825FF;
const C_STATUS_BG: u32 = 0x181825FF;
const C_TEXT: u32 = 0xCDD6F4FF;
const C_DIM: u32 = 0x6C7086FF;
const C_ACCENT: u32 = 0x89B4FAFF;
const C_SELECT: u32 = 0x313244FF;
const C_ERR: u32 = 0xF38BA8FF;
const C_BORDER: u32 = 0x45475AFF;
const C_CHECKER_A: u32 = 0x313244FF;
const C_CHECKER_B: u32 = 0x45475AFF;

// --- Drawing primitives ---
pub fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}

fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}

fn text_sm(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},s,{s}");
}

fn draw_img(x: u32, y: u32, w: u32, h: u32, path: &str) {
    println!("VYOMA_DRAW:draw_image:{x},{y},{w},{h},{path}");
}

pub fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

pub fn clear() {
    fill(0, 0, W, H, C_BG);
}

// --- Chrome ---
fn draw_title_bar(title: &str) {
    fill(0, 0, W, TITLE_H, C_TITLE_BG);
    text(12, 8, C_ACCENT, title);
    fill(0, TITLE_H, W, 1, C_BORDER);
}

const TITLE_H: u32 = crate::TITLE_H;

fn draw_status_bar(status: &str) {
    let sy = H - crate::STATUS_H;
    fill(0, sy - 1, W, 1, C_BORDER);
    fill(0, sy, W, crate::STATUS_H, C_STATUS_BG);
    text_sm(12, sy + 8, C_DIM, status);
}

fn draw_checkerboard(x: u32, y: u32, w: u32, h: u32) {
    let tile = 16u32;
    let cols = (w + tile - 1) / tile;
    let rows = (h + tile - 1) / tile;
    for row in 0..rows {
        for col in 0..cols {
            let c = if (row + col) % 2 == 0 { C_CHECKER_A } else { C_CHECKER_B };
            let tx = x + col * tile;
            let ty = y + row * tile;
            let tw = tile.min(x + w - tx);
            let th = tile.min(y + h - ty);
            fill(tx, ty, tw, th, c);
        }
    }
}

// --- View drawing ---
pub fn draw_browser(app: &App) {
    let (cursor, scroll) = match app.mode {
        Mode::Browser { cursor, scroll } => (cursor, scroll),
        _ => return,
    };

    fill(0, 0, W, H, C_BG);
    draw_title_bar("Image Viewer - /data/*.png");

    let files = &app.files;
    if files.is_empty() {
        text(20, CONTENT_Y + 20, C_DIM, "No PNG files found in /data/");
        text(20, CONTENT_Y + 44, C_DIM, "Copy PNG files to the data/ directory on the host.");
        draw_status_bar("No images available");
        flush();
        return;
    }

    let max_visible = ((CONTENT_H - 10) / LINE_H) as usize;

    for (i, f) in files.iter().enumerate().skip(scroll).take(max_visible) {
        let row = (i - scroll) as u32;
        let ty = CONTENT_Y + 6 + row * LINE_H;

        if i == cursor {
            fill(8, ty - 2, W - 16, LINE_H, C_SELECT);
        }

        let name = basename(f);
        let dims = match png_dimensions(f) {
            Some((w, h)) => format!("  {w}x{h}"),
            None => String::new(),
        };
        let size = fmt_size(file_size(f));
        let label = format!("{name}  ({size}{dims})");

        let c = if i == cursor { C_ACCENT } else { C_TEXT };
        text(16, ty, c, &label);
    }

    let status = format!(
        "{}/{} files  |  Up/Down: select  Enter: open  Ctrl+C: quit",
        cursor + 1,
        files.len()
    );
    draw_status_bar(&status);
    flush();
}

pub fn draw_viewer(app: &App) {
    let st = match &app.mode {
        Mode::Viewer(st) => st,
        _ => return,
    };

    fill(0, 0, W, H, C_BG);

    // Title bar
    let name = basename(&st.path);
    let idx_info = match app.current_index() {
        Some(i) => format!(" [{}/{}]", i + 1, app.files.len()),
        None => String::new(),
    };
    let title = if st.img_w > 0 {
        format!(
            "{name}  {w}x{h}  {zoom}{idx}",
            w = st.img_w,
            h = st.img_h,
            zoom = st.zoom_label(),
            idx = idx_info
        )
    } else {
        format!("{name}{idx_info}")
    };
    draw_title_bar(&title);

    if !st.error.is_empty() {
        text(20, CONTENT_Y + 40, C_ERR, &st.error);
    } else {
        let (dw, dh) = st.display_size();
        if dw > 0 && dh > 0 {
            // Center image in content area, offset by pan
            let cx = (CONTENT_W as i32 - dw as i32) / 2 + st.pan_x;
            let cy = (CONTENT_H as i32 - dh as i32) / 2 + st.pan_y;
            let ix = cx.max(0) as u32;
            let iy = (CONTENT_Y as i32 + cy).max(CONTENT_Y as i32) as u32;

            // Checkerboard background for transparency
            draw_checkerboard(ix, iy, dw.min(CONTENT_W), dh.min(CONTENT_H));

            // Render the image via supervisor
            draw_img(ix, iy, dw, dh, &st.path);
        }
    }

    // Status bar
    let size_str = fmt_size(file_size(&st.path));
    let dim_str = if st.img_w > 0 {
        format!("{}x{}", st.img_w, st.img_h)
    } else {
        "?".to_string()
    };
    let status = format!(
        "{size_str}  {dim_str}  |  Left/Right: prev/next  +/-: zoom  F: fit  Esc: back"
    );
    draw_status_bar(&status);
    flush();
}

pub fn redraw(app: &App) {
    match &app.mode {
        Mode::Browser { .. } => draw_browser(app),
        Mode::Viewer(_) => draw_viewer(app),
    }
}
