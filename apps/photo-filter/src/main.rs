use std::io::{self, BufRead, Write};

const W: u32 = 1040;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const CTRL_H: u32 = 80;
const CANVAS_X: u32 = 20;
const CANVAS_Y: u32 = HEADER_H + CTRL_H;
const CELL: u32 = 4;
const MAX_PX_W: u32 = (W - 40) / CELL;  // 250 pixels wide max
const MAX_PX_H: u32 = (H - CANVAS_Y - 20) / CELL; // ~148 pixels tall max

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
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

const FILTER_NAMES: [&str; 5] = ["Original", "Greyscale", "Brightness+50", "Contrast×2", "Invert"];

fn clamp_u8(v: i32) -> u8 { v.clamp(0, 255) as u8 }

fn apply_filter(r: u8, g: u8, b: u8, filter: usize) -> (u8, u8, u8) {
    match filter {
        0 => (r, g, b),
        1 => {
            let grey = ((r as u32 * 77 + g as u32 * 150 + b as u32 * 29) >> 8) as u8;
            (grey, grey, grey)
        }
        2 => (
            clamp_u8(r as i32 + 50),
            clamp_u8(g as i32 + 50),
            clamp_u8(b as i32 + 50),
        ),
        3 => (
            clamp_u8((r as i32 - 128) * 2 + 128),
            clamp_u8((g as i32 - 128) * 2 + 128),
            clamp_u8((b as i32 - 128) * 2 + 128),
        ),
        _ => (255 - r, 255 - g, 255 - b),
    }
}

fn skip_ws_comment(data: &[u8], pos: &mut usize) {
    loop {
        while *pos < data.len()
            && (data[*pos] == b' ' || data[*pos] == b'\n' || data[*pos] == b'\r' || data[*pos] == b'\t')
        {
            *pos += 1;
        }
        if *pos < data.len() && data[*pos] == b'#' {
            while *pos < data.len() && data[*pos] != b'\n' { *pos += 1; }
        } else {
            break;
        }
    }
}

fn parse_u32_from(data: &[u8], pos: &mut usize) -> Option<u32> {
    skip_ws_comment(data, pos);
    let start = *pos;
    let mut n: u32 = 0;
    while *pos < data.len() && data[*pos].is_ascii_digit() {
        n = n * 10 + (data[*pos] - b'0') as u32;
        *pos += 1;
    }
    if *pos == start { None } else { Some(n) }
}

fn load_ppm(path: &str) -> Option<(u32, u32, Vec<(u8, u8, u8)>)> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 7 || &data[0..2] != b"P6" { return None; }
    let mut pos = 2usize;
    let img_w = parse_u32_from(&data, &mut pos)?;
    let img_h = parse_u32_from(&data, &mut pos)?;
    let maxval = parse_u32_from(&data, &mut pos)?;
    // skip one whitespace after maxval
    if pos < data.len() { pos += 1; }
    let bytes_needed = (img_w * img_h * 3) as usize;
    if pos + bytes_needed > data.len() { return None; }
    let scale = if maxval == 255 { 1u32 } else { 255 };
    let pixels: Vec<(u8, u8, u8)> = data[pos..pos + bytes_needed]
        .chunks(3)
        .map(|c| {
            let r = if maxval == 255 { c[0] } else { (c[0] as u32 * scale / maxval) as u8 };
            let g = if maxval == 255 { c[1] } else { (c[1] as u32 * scale / maxval) as u8 };
            let b = if maxval == 255 { c[2] } else { (c[2] as u32 * scale / maxval) as u8 };
            (r, g, b)
        })
        .collect();
    Some((img_w, img_h, pixels))
}

fn save_ppm(path: &str, w: u32, h: u32, pixels: &[(u8, u8, u8)]) -> bool {
    let header = format!("P6\n{} {}\n255\n", w, h);
    let mut data = header.into_bytes();
    for &(r, g, b) in pixels {
        data.push(r); data.push(g); data.push(b);
    }
    std::fs::write(path, &data).is_ok()
}

fn list_ppm_files() -> Vec<String> {
    let mut files: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/data") {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.ends_with(".ppm") {
                files.push(format!("/data/{}", name));
            }
        }
    }
    files.sort();
    files
}

struct App {
    files:    Vec<String>,
    file_idx: usize,
    img_w:    u32,
    img_h:    u32,
    pixels:   Vec<(u8, u8, u8)>,
    filter:   usize,
    status:   String,
    loaded:   bool,
}

impl App {
    fn new() -> Self {
        let files = list_ppm_files();
        let mut app = App {
            files,
            file_idx: 0,
            img_w: 0, img_h: 0,
            pixels: Vec::new(),
            filter: 0,
            status: String::new(),
            loaded: false,
        };
        app.try_load();
        app
    }

    fn try_load(&mut self) {
        if self.files.is_empty() {
            self.loaded = false;
            self.status = "No .ppm files in /data — copy PPM images there first".to_string();
            return;
        }
        let path = self.files[self.file_idx].clone();
        match load_ppm(&path) {
            Some((w, h, px)) => {
                self.img_w = w; self.img_h = h; self.pixels = px;
                self.loaded = true;
                let name = path.split('/').last().unwrap_or(&path);
                self.status = format!("{} ({}×{})", name, w, h);
            }
            None => {
                self.loaded = false;
                self.status = format!("Failed to load {}", path);
            }
        }
    }

    fn next_file(&mut self) {
        if self.files.is_empty() { return; }
        self.file_idx = (self.file_idx + 1) % self.files.len();
        self.filter = 0;
        self.try_load();
    }

    fn prev_file(&mut self) {
        if self.files.is_empty() { return; }
        self.file_idx = if self.file_idx == 0 { self.files.len() - 1 } else { self.file_idx - 1 };
        self.filter = 0;
        self.try_load();
    }

    fn save_filtered(&mut self) {
        if !self.loaded { return; }
        let filtered: Vec<(u8, u8, u8)> = self.pixels.iter()
            .map(|&(r, g, b)| apply_filter(r, g, b, self.filter))
            .collect();
        if save_ppm("/data/filtered.ppm", self.img_w, self.img_h, &filtered) {
            self.status = format!("Saved /data/filtered.ppm ({} filter)", FILTER_NAMES[self.filter]);
        } else {
            self.status = "Save failed".to_string();
        }
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Photo Filter");
    text(160, 16, C_HINT, &app.status);

    // Controls row: filter buttons + navigation hint
    let ctrl_y = HEADER_H + 8;
    for (i, &name) in FILTER_NAMES.iter().enumerate() {
        let fx = 20 + i as u32 * 180;
        let selected = i == app.filter;
        fill(fx, ctrl_y, 172, 28, if selected { C_SEL } else { C_CARD });
        border(fx, ctrl_y, 172, 28, if selected { C_SEL } else { C_BORDER });
        let color = if selected { C_BG } else { C_HINT };
        text(fx + 6, ctrl_y + 6, color, &format!("{} [{}]", name, i + 1));
    }

    let nav_y = ctrl_y + 36;
    text(20, nav_y, C_HINT, "←→: file  1-5: filter  Enter: save as filtered.ppm");
    if app.files.len() > 1 {
        text(600, nav_y, C_HINT, &format!("File {}/{}", app.file_idx + 1, app.files.len()));
    }

    // Image preview
    if !app.loaded {
        text(CANVAS_X, CANVAS_Y + 60, C_HINT, "No image loaded.");
        text(CANVAS_X, CANVAS_Y + 84, C_HINT, "Place .ppm files in /data/ and restart.");
    } else {
        let disp_w = app.img_w.min(MAX_PX_W);
        let disp_h = app.img_h.min(MAX_PX_H);

        for py in 0..disp_h {
            for px in 0..disp_w {
                let idx = (py * app.img_w + px) as usize;
                if idx >= app.pixels.len() { break; }
                let (r, g, b) = app.pixels[idx];
                let (fr, fg, fb) = apply_filter(r, g, b, app.filter);
                let rgba = ((fr as u32) << 24) | ((fg as u32) << 16) | ((fb as u32) << 8) | 0xFF;
                let sx = CANVAS_X + px * CELL;
                let sy = CANVAS_Y + py * CELL;
                fill(sx, sy, CELL, CELL, rgba);
            }
        }

        // Border around image
        border(CANVAS_X - 1, CANVAS_Y - 1, disp_w * CELL + 2, disp_h * CELL + 2, C_BORDER);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise photo-filter");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        app.status.clear();

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[C" => { app.next_file(); }
            "\x1b[D" => { app.prev_file(); }
            "" => { app.save_filtered(); }
            "1" => { app.filter = 0; }
            "2" => { app.filter = 1; }
            "3" => { app.filter = 2; }
            "4" => { app.filter = 3; }
            "5" => { app.filter = 4; }
            _ => {}
        }
        draw(&app);
    }
}
