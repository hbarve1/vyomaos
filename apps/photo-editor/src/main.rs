// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

mod image;
mod app;

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

fn main() {
    let stdin = io::stdin();
    let mut app = app::App::new();

    println!("@supervisor: raise photo-editor");
    let _ = io::stdout().flush();
    app::draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        match &app.mode {
            app::Mode::CropInput => {
                match raw.as_str() {
                    "\x03" | "\x1b" => {
                        app.mode = app::Mode::Browse;
                        app.input.clear();
                        app.status = "Crop cancelled.".to_string();
                    }
                    "\x7f" => { app.input.pop(); }
                    "" => {
                        let inp = app.input.clone();
                        app.input.clear();
                        app.mode = app::Mode::Browse;
                        if let Some(img) = &app.image {
                            if let Some((cx, cy, cw, ch)) = app::parse_quad(&inp) {
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
                app::draw(&app);
                continue;
            }
            app::Mode::ResizeInput => {
                match raw.as_str() {
                    "\x03" | "\x1b" => {
                        app.mode = app::Mode::Browse;
                        app.input.clear();
                        app.status = "Resize cancelled.".to_string();
                    }
                    "\x7f" => { app.input.pop(); }
                    "" => {
                        let inp = app.input.clone();
                        app.input.clear();
                        app.mode = app::Mode::Browse;
                        if let Some(img) = &app.image {
                            if let Some((nw, nh)) = app::parse_pair(&inp) {
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
                app::draw(&app);
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
                    app.mode = app::Mode::CropInput;
                    app.input.clear();
                    app.status = "Enter crop: x,y,w,h".to_string();
                }
            }
            "r" | "R" => {
                if app.image.is_some() {
                    app.mode = app::Mode::ResizeInput;
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
        app::draw(&app);
    }
}
