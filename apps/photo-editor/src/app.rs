// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use super::image::{Image, make_demo_image};
use super::{
    C_BG, C_HEADER, C_BORDER, C_TEXT, C_HINT, C_ORANGE, C_SEL, C_CARD,
    W, H, HEADER_H, STATUS_H, FILE_W, PREVIEW_X, PREVIEW_W, PREVIEW_Y, PREVIEW_H,
    LINE_H, CHAR_W,
};

pub enum Mode {
    Browse,
    CropInput,
    ResizeInput,
}

pub struct App {
    pub files:   Vec<String>,
    pub sel:     usize,
    pub image:   Option<Image>,
    pub mode:    Mode,
    pub input:   String,
    pub status:  String,
}

impl App {
    pub fn new() -> Self {
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

    pub fn load_selected(&mut self) {
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

pub fn draw_preview(img: &Image) {
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
    super::fill(PREVIEW_X, PREVIEW_Y, PREVIEW_W, PREVIEW_H, 0x080C10FF);

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
            super::fill(
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

pub fn draw(app: &App) {
    super::fill(0, 0, W, H, C_BG);

    // Header
    super::fill(0, 0, W, HEADER_H, C_HEADER);
    super::fill(0, HEADER_H - 1, W, 1, C_BORDER);
    super::text(16, 12, C_ORANGE, "Photo Editor");
    if let Some(img) = &app.image {
        super::text(160, 12, C_HINT, &format!("{}×{} px", img.w, img.h));
    }

    // File list panel
    super::fill(0, HEADER_H, FILE_W, H - HEADER_H, C_CARD);
    super::fill(FILE_W, HEADER_H, 1, H - HEADER_H, C_BORDER);
    super::text(8, HEADER_H + 6, C_HINT, "Files");
    super::fill(0, HEADER_H + LINE_H + 2, FILE_W, 1, C_BORDER);

    let list_y0 = HEADER_H + LINE_H + 4;
    for (i, fname) in app.files.iter().enumerate() {
        let fy = list_y0 + i as u32 * LINE_H;
        if fy + LINE_H > H - STATUS_H { break; }
        if i == app.sel {
            super::fill(0, fy, FILE_W, LINE_H, 0x1C2D4EFF);
            super::text(6, fy + 2, C_SEL, &truncate(fname, (FILE_W / CHAR_W) as usize - 2));
        } else {
            let col = if fname.ends_with(".ppm") { C_TEXT } else { C_HINT };
            super::text(6, fy + 2, col, &truncate(fname, (FILE_W / CHAR_W) as usize - 2));
        }
    }

    // Preview area
    if let Some(img) = &app.image {
        draw_preview(img);
    } else {
        super::fill(PREVIEW_X, PREVIEW_Y, PREVIEW_W, PREVIEW_H, 0x080C10FF);
        super::text(PREVIEW_X + PREVIEW_W / 2 - 60, PREVIEW_Y + PREVIEW_H / 2, C_HINT, "No image loaded");
    }

    // Input overlay for crop/resize
    match &app.mode {
        Mode::CropInput => {
            let iy = H / 2 - 20;
            super::fill(PREVIEW_X + 40, iy - 4, PREVIEW_W - 80, 36, C_HEADER);
            super::border(PREVIEW_X + 40, iy - 4, PREVIEW_W - 80, 36, C_SEL);
            super::text(PREVIEW_X + 48, iy + 4, C_TEXT, &format!("Crop x,y,w,h: {}_", app.input));
        }
        Mode::ResizeInput => {
            let iy = H / 2 - 20;
            super::fill(PREVIEW_X + 40, iy - 4, PREVIEW_W - 80, 36, C_HEADER);
            super::border(PREVIEW_X + 40, iy - 4, PREVIEW_W - 80, 36, C_SEL);
            super::text(PREVIEW_X + 48, iy + 4, C_TEXT, &format!("Resize w,h: {}_", app.input));
        }
        _ => {}
    }

    // Status bar
    let sb_y = H - STATUS_H;
    super::fill(0, sb_y, W, STATUS_H, C_HEADER);
    super::fill(0, sb_y, W, 1, C_BORDER);
    super::text(8, sb_y + 6, C_HINT, &app.status);

    super::flush();
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max { s.to_string() } else { format!("{}…", &s[..max - 1]) }
}

pub fn parse_pair(s: &str) -> Option<(usize, usize)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() < 2 { return None; }
    let a = parts[0].trim().parse::<usize>().ok()?;
    let b = parts[1].trim().parse::<usize>().ok()?;
    Some((a, b))
}

pub fn parse_quad(s: &str) -> Option<(usize, usize, usize, usize)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() < 4 { return None; }
    let a = parts[0].trim().parse::<usize>().ok()?;
    let b = parts[1].trim().parse::<usize>().ok()?;
    let c = parts[2].trim().parse::<usize>().ok()?;
    let d = parts[3].trim().parse::<usize>().ok()?;
    Some((a, b, c, d))
}
