// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1280;
const H: u32 = 800;
const HEADER_H: u32 = 40;
const STATUS_H: u32 = 28;
const FILE_W: u32 = 200;
const PREVIEW_X: u32 = FILE_W + 1;
const PREVIEW_W: u32 = W - FILE_W - 1;
const PREVIEW_Y: u32 = HEADER_H;
const PREVIEW_H: u32 = H - HEADER_H - STATUS_H;
const LINE_H: u32 = 18;
const CHAR_W: u32 = 8;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
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

#[derive(Clone)]
struct Image {
    w:    usize,
    h:    usize,
    data: Vec<u8>, // RGB triples
}

impl Image {
    fn new(w: usize, h: usize, r: u8, g: u8, b: u8) -> Self {
        let mut data = Vec::with_capacity(w * h * 3);
        for _ in 0..w * h {
            data.push(r);
            data.push(g);
            data.push(b);
        }
        Image { w, h, data }
    }

    fn pixel(&self, x: usize, y: usize) -> (u8, u8, u8) {
        let i = (y * self.w + x) * 3;
        (self.data[i], self.data[i + 1], self.data[i + 2])
    }

    fn set_pixel(&mut self, x: usize, y: usize, r: u8, g: u8, b: u8) {
        let i = (y * self.w + x) * 3;
        self.data[i] = r;
        self.data[i + 1] = g;
        self.data[i + 2] = b;
    }

    fn crop(&self, cx: usize, cy: usize, cw: usize, ch: usize) -> Self {
        let cw = cw.min(self.w.saturating_sub(cx));
        let ch = ch.min(self.h.saturating_sub(cy));
        if cw == 0 || ch == 0 { return self.clone(); }
        let mut out = Image::new(cw, ch, 0, 0, 0);
        for y in 0..ch {
            for x in 0..cw {
                let (r, g, b) = self.pixel(cx + x, cy + y);
                out.set_pixel(x, y, r, g, b);
            }
        }
        out
    }

    fn resize(&self, nw: usize, nh: usize) -> Self {
        if nw == 0 || nh == 0 { return self.clone(); }
        let mut out = Image::new(nw, nh, 0, 0, 0);
        for y in 0..nh {
            for x in 0..nw {
                let sx = x * self.w / nw;
                let sy = y * self.h / nh;
                let (r, g, b) = self.pixel(sx.min(self.w - 1), sy.min(self.h - 1));
                out.set_pixel(x, y, r, g, b);
            }
        }
        out
    }

    fn rotate90(&self) -> Self {
        let mut out = Image::new(self.h, self.w, 0, 0, 0);
        for y in 0..self.h {
            for x in 0..self.w {
                let (r, g, b) = self.pixel(x, y);
                out.set_pixel(self.h - 1 - y, x, r, g, b);
            }
        }
        out
    }

    fn brightness(&mut self, delta: i32) {
        for v in self.data.iter_mut() {
            *v = (*v as i32 + delta).clamp(0, 255) as u8;
        }
    }

    fn contrast(&mut self, delta: i32) {
        let factor = (259 * (delta + 255)) / (255 * (259 - delta));
        for v in self.data.iter_mut() {
            *v = (factor * (*v as i32 - 128) / 256 + 128).clamp(0, 255) as u8;
        }
    }

    fn save_ppm(&self, path: &str) -> Result<(), String> {
        let mut out: Vec<u8> = Vec::new();
        let header = format!("P6\n{} {}\n255\n", self.w, self.h);
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(&self.data);
        std::fs::write(path, out).map_err(|e| e.to_string())
    }

    fn load_ppm(path: &str) -> Result<Self, String> {
        let raw = std::fs::read(path).map_err(|e| e.to_string())?;
        let mut pos = 0usize;

        let skip_comment_ws = |data: &[u8], p: &mut usize| {
            while *p < data.len() {
                if data[*p] == b'#' {
                    while *p < data.len() && data[*p] != b'\n' { *p += 1; }
                } else if data[*p].is_ascii_whitespace() {
                    *p += 1;
                } else {
                    break;
                }
            }
        };

        let read_token = |data: &[u8], p: &mut usize| -> String {
            let mut tok = String::new();
            while *p < data.len() && !data[*p].is_ascii_whitespace() {
                tok.push(data[*p] as char);
                *p += 1;
            }
            tok
        };

        skip_comment_ws(&raw, &mut pos);
        let magic = read_token(&raw, &mut pos);
        if magic != "P6" { return Err("not P6".to_string()); }
        skip_comment_ws(&raw, &mut pos);
        let ws = read_token(&raw, &mut pos).parse::<usize>().map_err(|e| e.to_string())?;
        skip_comment_ws(&raw, &mut pos);
        let hs = read_token(&raw, &mut pos).parse::<usize>().map_err(|e| e.to_string())?;
        skip_comment_ws(&raw, &mut pos);
        let _maxval = read_token(&raw, &mut pos);
        if pos < raw.len() && raw[pos].is_ascii_whitespace() { pos += 1; }
        let expected = ws * hs * 3;
        if raw.len() - pos < expected {
            return Err(format!("truncated: need {} got {}", expected, raw.len() - pos));
        }
        let data = raw[pos..pos + expected].to_vec();
        Ok(Image { w: ws, h: hs, data })
    }
}

fn make_demo_image() -> Image {
    let w = 120usize;
    let h = 80usize;
    let mut img = Image::new(w, h, 0, 0, 0);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w) as u8;
            let g = (y * 255 / h) as u8;
            let b = ((x + y) * 128 / (w + h)) as u8;
            img.set_pixel(x, y, r, g, b);
        }
    }
    img
}

enum Mode {
    Browse,
    CropInput,
    ResizeInput,
}

struct App {
    files:   Vec<String>,
    sel:     usize,
    image:   Option<Image>,
    mode:    Mode,
    input:   String,
    status:  String,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            files: vec![
                "demo.ppm".to_string(),
                "screenshot.ppm".to_string(),
                "filtered.ppm".to_string(),
                "edited.ppm".to_string(),
            ],
            sel: 0,
            image: Some(make_demo_image()),
            mode: Mode::Browse,
            input: String::new(),
            status: String::from("C=crop  R=resize  T=rotate90  B=brightness+  N=contrast+  S=save  ↑↓=file  L=load"),
        };
        // Try to scan /data for .ppm files
        if let Ok(entries) = std::fs::read_dir("/data") {
            let mut ppm_files: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".ppm") { Some(name) } else { None }
                })
                .collect();
            ppm_files.sort();
            if !ppm_files.is_empty() {
                a.files = ppm_files;
            }
        }
        a
    }

    fn load_selected(&mut self) {
        let path = format!("/data/{}", self.files[self.sel]);
        match Image::load_ppm(&path) {
            Ok(img) => {
                self.status = format!("Loaded {} ({}×{})", self.files[self.sel], img.w, img.h);
                self.image = Some(img);
            }
            Err(e) => {
                self.status = format!("Load error: {}", e);
                self.image = Some(make_demo_image());
            }
        }
    }
}

fn draw_preview(img: &Image) {
    // Scale to fit PREVIEW_W × PREVIEW_H using block rendering
    let max_w = PREVIEW_W as usize;
    let max_h = PREVIEW_H as usize;
    if img.w == 0 || img.h == 0 { return; }

    // Compute block size to fit
    let bx = (max_w / img.w).max(1).min(4);
    let by = (max_h / img.h).max(1).min(4);
    let block = bx.min(by);
    let render_w = img.w.min(max_w / block);
    let render_h = img.h.min(max_h / block);

    // Clear preview area
    fill(PREVIEW_X, PREVIEW_Y, PREVIEW_W, PREVIEW_H, 0x080C10FF);

    let ox = PREVIEW_X + (PREVIEW_W - (render_w * block) as u32) / 2;
    let oy = PREVIEW_Y + (PREVIEW_H - (render_h * block) as u32) / 2;

    for y in 0..render_h {
        let mut x = 0usize;
        while x < render_w {
            let (r0, g0, b0) = img.pixel(x, y);
            // Run-length encode same color
            let mut run = 1usize;
            while x + run < render_w {
                let (r1, g1, b1) = img.pixel(x + run, y);
                if r0 != r1 || g0 != g1 || b0 != b1 { break; }
                run += 1;
            }
            let rgba = ((r0 as u32) << 24) | ((g0 as u32) << 16) | ((b0 as u32) << 8) | 0xFF;
            fill(
                ox + (x * block) as u32,
                oy + (y * block) as u32,
                (run * block) as u32,
                block as u32,
                rgba,
            );
            x += run;
        }
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Photo Editor");
    if let Some(img) = &app.image {
        text(160, 12, C_HINT, &format!("{}×{} px", img.w, img.h));
    }

    // File list panel
    fill(0, HEADER_H, FILE_W, H - HEADER_H, C_CARD);
    fill(FILE_W, HEADER_H, 1, H - HEADER_H, C_BORDER);
    text(8, HEADER_H + 6, C_HINT, "Files");
    fill(0, HEADER_H + LINE_H + 2, FILE_W, 1, C_BORDER);

    let list_y0 = HEADER_H + LINE_H + 4;
    for (i, fname) in app.files.iter().enumerate() {
        let fy = list_y0 + i as u32 * LINE_H;
        if fy + LINE_H > H - STATUS_H { break; }
        if i == app.sel {
            fill(0, fy, FILE_W, LINE_H, 0x1C2D4EFF);
            text(6, fy + 2, C_SEL, &truncate(fname, (FILE_W / CHAR_W) as usize - 2));
        } else {
            let col = if fname.ends_with(".ppm") { C_TEXT } else { C_HINT };
            text(6, fy + 2, col, &truncate(fname, (FILE_W / CHAR_W) as usize - 2));
        }
    }

    // Preview area
    if let Some(img) = &app.image {
        draw_preview(img);
    } else {
        fill(PREVIEW_X, PREVIEW_Y, PREVIEW_W, PREVIEW_H, 0x080C10FF);
        text(PREVIEW_X + PREVIEW_W / 2 - 60, PREVIEW_Y + PREVIEW_H / 2, C_HINT, "No image loaded");
    }

    // Input overlay for crop/resize
    match &app.mode {
        Mode::CropInput => {
            let iy = H / 2 - 20;
            fill(PREVIEW_X + 40, iy - 4, PREVIEW_W - 80, 36, C_HEADER);
            border(PREVIEW_X + 40, iy - 4, PREVIEW_W - 80, 36, C_SEL);
            text(PREVIEW_X + 48, iy + 4, C_TEXT, &format!("Crop x,y,w,h: {}_", app.input));
        }
        Mode::ResizeInput => {
            let iy = H / 2 - 20;
            fill(PREVIEW_X + 40, iy - 4, PREVIEW_W - 80, 36, C_HEADER);
            border(PREVIEW_X + 40, iy - 4, PREVIEW_W - 80, 36, C_SEL);
            text(PREVIEW_X + 48, iy + 4, C_TEXT, &format!("Resize w,h: {}_", app.input));
        }
        _ => {}
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max - 1]) }
}

fn parse_pair(s: &str) -> Option<(usize, usize)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() < 2 { return None; }
    let a = parts[0].trim().parse::<usize>().ok()?;
    let b = parts[1].trim().parse::<usize>().ok()?;
    Some((a, b))
}

fn parse_quad(s: &str) -> Option<(usize, usize, usize, usize)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() < 4 { return None; }
    let a = parts[0].trim().parse::<usize>().ok()?;
    let b = parts[1].trim().parse::<usize>().ok()?;
    let c = parts[2].trim().parse::<usize>().ok()?;
    let d = parts[3].trim().parse::<usize>().ok()?;
    Some((a, b, c, d))
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise photo-editor");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        match &app.mode {
            Mode::CropInput => {
                match raw.as_str() {
                    "\x03" | "\x1b" => {
                        app.mode = Mode::Browse;
                        app.input.clear();
                        app.status = "Crop cancelled.".to_string();
                    }
                    "\x7f" => { app.input.pop(); }
                    "" => {
                        let inp = app.input.clone();
                        app.input.clear();
                        app.mode = Mode::Browse;
                        if let Some(img) = &app.image {
                            if let Some((cx, cy, cw, ch)) = parse_quad(&inp) {
                                let cropped = img.crop(cx, cy, cw, ch);
                                app.status = format!("Cropped to {}×{}", cropped.w, cropped.h);
                                app.image = Some(cropped);
                            } else {
                                app.status = "Crop: invalid input (need x,y,w,h)".to_string();
                            }
                        }
                    }
                    s if s.len() == 1 => { app.input.push_str(s); }
                    _ => {}
                }
                draw(&app);
                continue;
            }
            Mode::ResizeInput => {
                match raw.as_str() {
                    "\x03" | "\x1b" => {
                        app.mode = Mode::Browse;
                        app.input.clear();
                        app.status = "Resize cancelled.".to_string();
                    }
                    "\x7f" => { app.input.pop(); }
                    "" => {
                        let inp = app.input.clone();
                        app.input.clear();
                        app.mode = Mode::Browse;
                        if let Some(img) = &app.image {
                            if let Some((nw, nh)) = parse_pair(&inp) {
                                let resized = img.resize(nw, nh);
                                app.status = format!("Resized to {}×{}", resized.w, resized.h);
                                app.image = Some(resized);
                            } else {
                                app.status = "Resize: invalid input (need w,h)".to_string();
                            }
                        }
                    }
                    s if s.len() == 1 => { app.input.push_str(s); }
                    _ => {}
                }
                draw(&app);
                continue;
            }
            _ => {}
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                if app.sel > 0 { app.sel -= 1; }
            }
            "\x1b[B" => {
                if app.sel + 1 < app.files.len() { app.sel += 1; }
            }
            "l" | "L" | "" => {
                app.load_selected();
            }
            "c" | "C" => {
                if app.image.is_some() {
                    app.mode = Mode::CropInput;
                    app.input.clear();
                    app.status = "Enter crop: x,y,w,h".to_string();
                }
            }
            "r" | "R" => {
                if app.image.is_some() {
                    app.mode = Mode::ResizeInput;
                    app.input.clear();
                    app.status = "Enter resize: w,h".to_string();
                }
            }
            "t" | "T" => {
                if let Some(img) = app.image.take() {
                    let rotated = img.rotate90();
                    app.status = format!("Rotated 90° → {}×{}", rotated.w, rotated.h);
                    app.image = Some(rotated);
                }
            }
            "b" | "B" => {
                if let Some(img) = app.image.as_mut() {
                    img.brightness(10);
                    app.status = "Brightness +10".to_string();
                }
            }
            "v" | "V" => {
                if let Some(img) = app.image.as_mut() {
                    img.brightness(-10);
                    app.status = "Brightness -10".to_string();
                }
            }
            "n" | "N" => {
                if let Some(img) = app.image.as_mut() {
                    img.contrast(10);
                    app.status = "Contrast +10".to_string();
                }
            }
            "m" | "M" => {
                if let Some(img) = app.image.as_mut() {
                    img.contrast(-10);
                    app.status = "Contrast -10".to_string();
                }
            }
            "s" | "S" => {
                if let Some(img) = &app.image {
                    match img.save_ppm("/data/edited.ppm") {
                        Ok(()) => app.status = format!("Saved /data/edited.ppm ({}×{})", img.w, img.h),
                        Err(e) => app.status = format!("Save error: {}", e),
                    }
                }
            }
            _ => {}
        }
        draw(&app);
    }
}
