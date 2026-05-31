// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P73 — PNG Image Viewer for VyomaOS.
//!
//! Opens PNG images from `/data/` using `VYOMA_DRAW:draw_image` protocol.
//! Accepts `VYOMA_SYSTEM:open:<path>` on stdin to open a specific file.
//! If no image is specified, lists PNG files in `/data/` for selection.

mod ui;

use std::io::{self, BufRead, Write};

// --- Layout constants (shared with ui module) ---
pub const W: u32 = 1240;
pub const H: u32 = 760;
pub const TITLE_H: u32 = 32;
pub const STATUS_H: u32 = 24;
pub const CONTENT_Y: u32 = TITLE_H + 1;
pub const CONTENT_H: u32 = H - TITLE_H - STATUS_H - 2;
pub const CONTENT_W: u32 = W;
pub const LINE_H: u32 = 20;

// --- Zoom levels (percentage) ---
const ZOOM_LEVELS: &[u32] = &[25, 50, 75, 100, 150, 200, 300, 400];
pub const ZOOM_FIT: u32 = 0; // Sentinel: fit to window

// --- PNG header reading ---
/// Read PNG width and height from the IHDR chunk (bytes 16..24).
pub fn png_dimensions(path: &str) -> Option<(u32, u32)> {
    let data = std::fs::read(path).ok()?;
    if data.len() < 24 {
        return None;
    }
    if &data[0..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
    Some((w, h))
}

/// List PNG files in /data.
pub fn list_png_files() -> Vec<String> {
    let Ok(dir) = std::fs::read_dir("/data") else {
        return Vec::new();
    };
    let mut files: Vec<String> = dir
        .filter_map(|e| e.ok())
        .map(|e| format!("/data/{}", e.file_name().to_string_lossy()))
        .filter(|n| n.to_ascii_lowercase().ends_with(".png"))
        .collect();
    files.sort();
    files
}

/// Get file size in bytes.
pub fn file_size(path: &str) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Format bytes as human-readable string.
pub fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

/// Extract filename from path.
pub fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

// --- Application state ---

pub enum Mode {
    Browser { cursor: usize, scroll: usize },
    Viewer(ImageState),
}

pub struct ImageState {
    pub path: String,
    pub img_w: u32,
    pub img_h: u32,
    pub zoom: u32,
    pub pan_x: i32,
    pub pan_y: i32,
    pub error: String,
}

impl ImageState {
    pub fn new(path: String) -> Self {
        let (img_w, img_h, error) = match png_dimensions(&path) {
            Some((w, h)) => (w, h, String::new()),
            None => (0, 0, format!("Cannot read PNG: {}", basename(&path))),
        };
        Self { path, img_w, img_h, zoom: ZOOM_FIT, pan_x: 0, pan_y: 0, error }
    }

    pub fn display_size(&self) -> (u32, u32) {
        if self.img_w == 0 || self.img_h == 0 {
            return (0, 0);
        }
        if self.zoom == ZOOM_FIT {
            fit_dimensions(self.img_w, self.img_h, CONTENT_W - 20, CONTENT_H - 10)
        } else {
            let dw = (self.img_w as u64 * self.zoom as u64 / 100) as u32;
            let dh = (self.img_h as u64 * self.zoom as u64 / 100) as u32;
            (dw.max(1), dh.max(1))
        }
    }

    pub fn zoom_label(&self) -> String {
        if self.zoom == ZOOM_FIT {
            "Fit".to_string()
        } else {
            format!("{}%", self.zoom)
        }
    }
}

/// Compute dimensions that fit inside (max_w, max_h) preserving aspect ratio.
fn fit_dimensions(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (0, 0);
    }
    let scale_w = max_w as f64 / src_w as f64;
    let scale_h = max_h as f64 / src_h as f64;
    let scale = scale_w.min(scale_h).min(1.0);
    let dw = (src_w as f64 * scale) as u32;
    let dh = (src_h as f64 * scale) as u32;
    (dw.max(1), dh.max(1))
}

pub struct App {
    pub mode: Mode,
    pub files: Vec<String>,
}

impl App {
    fn new() -> Self {
        Self {
            mode: Mode::Browser { cursor: 0, scroll: 0 },
            files: list_png_files(),
        }
    }

    fn open_image(&mut self, path: &str) {
        let idx = self.files.iter().position(|f| f == path);
        self.mode = Mode::Viewer(ImageState::new(path.to_string()));
        if idx.is_none() {
            self.files = list_png_files();
        }
    }

    pub fn current_index(&self) -> Option<usize> {
        if let Mode::Viewer(ref st) = self.mode {
            self.files.iter().position(|f| f == &st.path)
        } else {
            None
        }
    }
}

// --- Zoom helpers ---

fn zoom_in(st: &mut ImageState) {
    let current = effective_zoom(st);
    for &z in ZOOM_LEVELS {
        if z > current {
            st.zoom = z;
            return;
        }
    }
    st.zoom = *ZOOM_LEVELS.last().unwrap_or(&400);
}

fn zoom_out(st: &mut ImageState) {
    let current = effective_zoom(st);
    for &z in ZOOM_LEVELS.iter().rev() {
        if z < current {
            st.zoom = z;
            return;
        }
    }
    st.zoom = *ZOOM_LEVELS.first().unwrap_or(&25);
}

fn effective_zoom(st: &ImageState) -> u32 {
    if st.zoom == ZOOM_FIT {
        let (dw, _) = st.display_size();
        if st.img_w > 0 {
            (dw as u64 * 100 / st.img_w as u64) as u32
        } else {
            100
        }
    } else {
        st.zoom
    }
}

// --- Input handling ---

enum BrowserAction {
    None,
    OpenFile(String),
}

fn handle_browser_input(
    raw: &str,
    cursor: &mut usize,
    scroll: &mut usize,
    files: &[String],
) -> BrowserAction {
    let max_visible = ((CONTENT_H - 10) / LINE_H) as usize;

    match raw {
        "\x03" => {
            ui::clear();
            ui::flush();
            std::process::exit(0);
        }
        "\x1b[A" => {
            if *cursor > 0 {
                *cursor -= 1;
                if *cursor < *scroll {
                    *scroll = *cursor;
                }
            }
        }
        "\x1b[B" => {
            if *cursor + 1 < files.len() {
                *cursor += 1;
                if *cursor >= *scroll + max_visible {
                    *scroll = *cursor - max_visible + 1;
                }
            }
        }
        "" => {
            if let Some(path) = files.get(*cursor) {
                return BrowserAction::OpenFile(path.clone());
            }
        }
        _ => {}
    }
    BrowserAction::None
}

fn handle_viewer_input(raw: &str, st: &mut ImageState, files: &[String]) -> bool {
    match raw {
        "\x1b" | "\x03" => {
            if files.len() <= 1 {
                ui::clear();
                ui::flush();
                std::process::exit(0);
            }
            return true;
        }
        "\x1b[D" => {
            if !files.is_empty() {
                let idx = files.iter().position(|f| f == &st.path).unwrap_or(0);
                let new_idx = if idx > 0 { idx - 1 } else { files.len() - 1 };
                if let Some(path) = files.get(new_idx) {
                    *st = ImageState::new(path.clone());
                }
            }
        }
        "\x1b[C" => {
            if !files.is_empty() {
                let idx = files.iter().position(|f| f == &st.path).unwrap_or(0);
                let new_idx = (idx + 1) % files.len();
                if let Some(path) = files.get(new_idx) {
                    *st = ImageState::new(path.clone());
                }
            }
        }
        "+" | "=" => {
            zoom_in(st);
            st.pan_x = 0;
            st.pan_y = 0;
        }
        "-" | "_" => {
            zoom_out(st);
            st.pan_x = 0;
            st.pan_y = 0;
        }
        "f" | "F" => {
            st.zoom = ZOOM_FIT;
            st.pan_x = 0;
            st.pan_y = 0;
        }
        "q" => {
            if files.len() <= 1 {
                ui::clear();
                ui::flush();
                std::process::exit(0);
            }
            return true;
        }
        _ => {}
    }
    false
}

// --- Main loop ---

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise image-viewer");
    let _ = io::stdout().flush();

    ui::redraw(&app);

    for line in stdin.lock().lines() {
        let raw = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if raw.starts_with("VYOMA_SYSTEM:open:") {
            let path = &raw["VYOMA_SYSTEM:open:".len()..];
            app.open_image(path);
            ui::redraw(&app);
            continue;
        }

        if raw.starts_with("VYOMA_SYSTEM:") || raw.starts_with("REPLY:") {
            continue;
        }

        match &mut app.mode {
            Mode::Browser { ref mut cursor, ref mut scroll } => {
                let action = handle_browser_input(&raw, cursor, scroll, &app.files);
                if let BrowserAction::OpenFile(path) = action {
                    app.mode = Mode::Viewer(ImageState::new(path));
                }
                ui::redraw(&app);
            }
            Mode::Viewer(ref mut st) => {
                let should_back = handle_viewer_input(&raw, st, &app.files);
                if should_back {
                    let idx = app.current_index().unwrap_or(0);
                    app.mode = Mode::Browser { cursor: idx, scroll: 0 };
                }
                ui::redraw(&app);
            }
        }
    }
}
