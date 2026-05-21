use std::io::{self, BufRead, Write};

const W: u32 = 1360;
const H: u32 = 840;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;

const BLOCK: u32 = 3;
const THUMB_SRC_W: usize = 100;
const THUMB_SRC_H: usize = 75;
const THUMB_W: u32 = THUMB_SRC_W as u32 * BLOCK;
const THUMB_H: u32 = THUMB_SRC_H as u32 * BLOCK;
const CAPTION_H: u32 = 18;
const THUMB_CELL_H: u32 = THUMB_H + CAPTION_H + 16;
const THUMB_CELL_W: u32 = THUMB_W + 16;
const GRID_COLS: usize = 4;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
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

struct Image {
    path:    String,
    name:    String,
    pixels:  Vec<u32>, // RGBA packed
    pix_w:   usize,
    pix_h:   usize,
}

fn skip_ws(bytes: &[u8], pos: &mut usize) {
    loop {
        while *pos < bytes.len() && matches!(bytes[*pos], b' ' | b'\t' | b'\r' | b'\n') { *pos += 1; }
        if *pos < bytes.len() && bytes[*pos] == b'#' {
            while *pos < bytes.len() && bytes[*pos] != b'\n' { *pos += 1; }
        } else { break; }
    }
}

fn parse_num(bytes: &[u8], pos: &mut usize) -> Option<usize> {
    skip_ws(bytes, pos);
    let s = *pos;
    while *pos < bytes.len() && bytes[*pos].is_ascii_digit() { *pos += 1; }
    if *pos == s { return None; }
    std::str::from_utf8(&bytes[s..*pos]).ok()?.parse().ok()
}

fn load_thumb(path: &str) -> Option<Image> {
    let data = std::fs::read(path).ok()?;
    let mut pos = 0usize;
    skip_ws(&data, &mut pos);
    if pos + 2 > data.len() || &data[pos..pos+2] != b"P6" { return None; }
    pos += 2;
    let iw = parse_num(&data, &mut pos)?;
    let ih = parse_num(&data, &mut pos)?;
    let mv = parse_num(&data, &mut pos)?;
    if pos < data.len() { pos += 1; }
    if iw == 0 || ih == 0 || mv == 0 { return None; }
    let bps = if mv > 255 { 2 } else { 1 };
    if pos + iw * ih * 3 * bps > data.len() { return None; }
    let raw = &data[pos..];

    let tw = THUMB_SRC_W.min(iw);
    let th = THUMB_SRC_H.min(ih);
    let mut pixels = Vec::with_capacity(tw * th);
    for ty in 0..th {
        for tx in 0..tw {
            let sx = tx * iw / tw;
            let sy = ty * ih / th;
            let i = (sy * iw + sx) * 3 * bps;
            let (r, g, b) = if bps == 1 {
                (raw[i] as u32, raw[i+1] as u32, raw[i+2] as u32)
            } else {
                let scale = |hi: usize, lo: usize| (raw[hi] as u32 * 256 + raw[lo] as u32) * 255 / mv as u32;
                (scale(i, i+1), scale(i+2, i+3), scale(i+4, i+5))
            };
            pixels.push((r << 24) | (g << 16) | (b << 8) | 0xFF);
        }
    }
    let name = std::path::Path::new(path)
        .file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string();
    Some(Image { path: path.to_string(), name, pixels, pix_w: tw, pix_h: th })
}

fn list_ppms() -> Vec<String> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir("/data") {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|e| e.to_str()) == Some("ppm") {
                v.push(p.to_string_lossy().to_string());
            }
        }
    }
    v.sort();
    v
}

fn draw_pixels(img: &Image, ox: u32, oy: u32) {
    for py in 0..img.pix_h {
        for px in 0..img.pix_w {
            let rgba = img.pixels[py * img.pix_w + px];
            fill(ox + px as u32 * BLOCK, oy + py as u32 * BLOCK, BLOCK, BLOCK, rgba);
        }
    }
}

fn draw_grid(images: &[Image], cursor: usize) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Image Gallery");
    text(210, 16, C_HINT, "←→↑↓:navigate  Enter:fullscreen  Ctrl+C:exit");

    if images.is_empty() {
        text(16, HEADER_H + 40, C_HINT, "No .ppm files found in /data/");
    } else {
        let avail_h = H - HEADER_H - STATUS_H;
        let rows_vis = (avail_h / (THUMB_CELL_H + 8)).max(1) as usize;
        let cursor_row = cursor / GRID_COLS;
        let scroll = if cursor_row + 1 > rows_vis { cursor_row + 1 - rows_vis } else { 0 };
        let pad_x = (W - GRID_COLS as u32 * THUMB_CELL_W) / (GRID_COLS as u32 + 1);

        for (i, img) in images.iter().enumerate() {
            let col = i % GRID_COLS;
            let row = i / GRID_COLS;
            if row < scroll { continue; }
            let disp_row = (row - scroll) as u32;
            if disp_row >= rows_vis as u32 { break; }

            let tx = pad_x + col as u32 * (THUMB_CELL_W + pad_x);
            let ty = HEADER_H + 8 + disp_row * (THUMB_CELL_H + 8);
            let selected = i == cursor;

            fill(tx, ty, THUMB_CELL_W, THUMB_CELL_H, C_CARD);
            if selected {
                border(tx - 1, ty - 1, THUMB_CELL_W + 2, THUMB_CELL_H + 2, C_SEL);
            }
            draw_pixels(img, tx + 8, ty + 8);

            let max_cap = (THUMB_CELL_W as usize - 8) / CHAR_W as usize;
            let cap = if img.name.len() > max_cap { &img.name[..max_cap] } else { &img.name };
            let cap_col = if selected { C_SEL } else { C_HINT };
            text(tx + 8, ty + THUMB_H + 12, cap_col, cap);
        }
    }

    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let n = images.len();
    text(8, sb_y + 6, C_HINT, &format!("{} images  |  selected {}/{}", n, cursor + 1, n.max(1)));
    flush();
}

fn draw_full(images: &[Image], idx: usize) {
    let img = &images[idx];
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, &img.name);
    text(16 + (img.name.len() as u32 + 2) * CHAR_W, 16, C_HINT,
         &format!("{}/{}  |  ←→:prev/next  Esc:grid", idx + 1, images.len()));

    let dw = img.pix_w as u32 * BLOCK;
    let dh = img.pix_h as u32 * BLOCK;
    let avail_h = H - HEADER_H - STATUS_H;
    let ox = (W - dw) / 2;
    let oy = HEADER_H + (avail_h - dh) / 2;
    draw_pixels(img, ox, oy);
    border(ox - 2, oy - 2, dw + 4, dh + 4, C_BORDER);

    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let max_p = (W as usize - 16) / CHAR_W as usize;
    let p = if img.path.len() > max_p { &img.path[..max_p] } else { &img.path };
    text(8, sb_y + 6, C_HINT, p);
    flush();
}

fn main() {
    let stdin = io::stdin();

    let paths = list_ppms();
    let images: Vec<Image> = paths.iter().filter_map(|p| load_thumb(p)).collect();

    let mut cursor: usize = 0;
    let mut fullscreen = false;

    println!("@supervisor: raise image-gallery");
    let _ = io::stdout().flush();
    draw_grid(&images, cursor);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        if fullscreen {
            match raw.as_str() {
                "\x03" | "\x1b" | "\x7f" => { fullscreen = false; draw_grid(&images, cursor); }
                "\x1b[D" => {
                    if cursor > 0 { cursor -= 1; }
                    draw_full(&images, cursor);
                }
                "\x1b[C" => {
                    if cursor + 1 < images.len() { cursor += 1; }
                    draw_full(&images, cursor);
                }
                _ => {}
            }
        } else {
            let n = images.len();
            match raw.as_str() {
                "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
                "\x1b[C" => { if n > 0 { cursor = (cursor + 1).min(n - 1); } }
                "\x1b[D" => { cursor = cursor.saturating_sub(1); }
                "\x1b[B" => {
                    if n > 0 { cursor = (cursor + GRID_COLS).min(n - 1); }
                }
                "\x1b[A" => {
                    cursor = cursor.saturating_sub(GRID_COLS);
                }
                "" => {
                    if n > 0 { fullscreen = true; draw_full(&images, cursor); continue; }
                }
                _ => {}
            }
            draw_grid(&images, cursor);
        }
    }
}
