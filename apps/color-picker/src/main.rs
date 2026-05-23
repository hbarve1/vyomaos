use std::io::{self, BufRead, Write};

const W: u32 = 640;
const H: u32 = 500;
const C_BG: u32     = 0x0D1117FF;
const C_BORDER: u32 = 0x30363DFF;
const C_TITLE: u32  = 0xFFFFFFFF;
const C_DIM: u32    = 0x8B949EFF;
const C_ACCENT: u32 = 0x58A6FFFF;
const C_HINT: u32   = 0x6E7681FF;

const GRAD_X: u32 = 20;
const GRAD_Y: u32 = 48;
const GRAD_W: u32 = 256;
const GRAD_H: u32 = 256;
const SLIDER_X: u32 = GRAD_X;
const SLIDER_Y: u32 = GRAD_Y + GRAD_H + 8;
const SLIDER_W: u32 = GRAD_W;
const SLIDER_H: u32 = 20;
const PREVIEW_X: u32 = GRAD_X + GRAD_W + 24;
const PREVIEW_Y: u32 = GRAD_Y;
const PREVIEW_W: u32 = 120;
const PREVIEW_H: u32 = 60;

fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

/// Integer HSV→RGBA. h: 0–359, s: 0–255, v: 0–255. Returns 0xRRGGBBFF.
fn hsv_to_rgba(h: u32, s: u32, v: u32) -> u32 {
    let s = s as u64;
    let v = v as u64;
    if s == 0 {
        let g = (v * 255 / 255) as u32;
        return (g << 24) | (g << 16) | (g << 8) | 0xFF;
    }
    let region = h / 60;
    let remainder = (h % 60) * 255 / 60;
    let p = v * (255 - s) / 255;
    let q = v * (255 - (s * remainder as u64) / 255) / 255;
    let t = v * (255 - (s * (255 - remainder as u64)) / 255) / 255;
    let (r, g, b) = match region {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | 0xFF
}

struct State {
    hue: u32,   // 0–359
    sat: u32,   // 0–255
    val: u32,   // 0–255
}

impl Default for State {
    fn default() -> Self { Self { hue: 200, sat: 200, val: 220 } }
}

impl State {
    fn current_rgba(&self) -> u32 {
        hsv_to_rgba(self.hue, self.sat, self.val)
    }
    fn hex(&self) -> String {
        let rgba = self.current_rgba();
        format!("#{:08X}", rgba)
    }
}

const CELL: u32 = 4; // each gradient cell is 4×4 pixels — 64×64 cells = 4096 draw calls

fn draw_gradient(s: &State) {
    let cells = GRAD_W / CELL; // 64 cells across and down
    for col in 0..cells {
        let h = col * 359 / (cells - 1);
        for row in 0..cells {
            let sat = 255 - row * 255 / (cells - 1);
            let rgba = hsv_to_rgba(h, sat, s.val);
            println!("VYOMA_DRAW:fill_rect:{},{},{},{},{rgba:#010x}",
                GRAD_X + col * CELL, GRAD_Y + row * CELL, CELL, CELL);
        }
    }
    // Crosshair at current hue/sat
    let cx = GRAD_X + s.hue * (GRAD_W - CELL) / 359;
    let cy = GRAD_Y + (255 - s.sat) * (GRAD_H - CELL) / 255;
    println!("VYOMA_DRAW:fill_rect:{},{},12,2,0xFFFFFFFF", cx.saturating_sub(6), cy + CELL / 2);
    println!("VYOMA_DRAW:fill_rect:{},{},2,12,0xFFFFFFFF", cx + CELL / 2, cy.saturating_sub(6));
}

fn draw_slider(s: &State) {
    let cells = SLIDER_W / CELL; // 64 cells
    for col in 0..cells {
        let v = col * 255 / (cells - 1);
        let rgba = hsv_to_rgba(s.hue, s.sat, v);
        println!("VYOMA_DRAW:fill_rect:{},{},{},{},{rgba:#010x}",
            SLIDER_X + col * CELL, SLIDER_Y, CELL, SLIDER_H);
    }
    // Cursor at current value
    let sx = SLIDER_X + s.val * (SLIDER_W - CELL) / 255;
    println!("VYOMA_DRAW:rect_border:{},{},4,{},0xFFFFFFFF", sx, SLIDER_Y, SLIDER_H);
}

fn draw(s: &State) {
    println!("VYOMA_DRAW:fill_rect:0,0,{W},{H},{:#010x}", C_BG);
    println!("VYOMA_DRAW:draw_text:20,14,{C_ACCENT:#010x},m,Color Picker");
    println!("VYOMA_DRAW:fill_rect:0,36,{W},1,{C_BORDER:#010x}");

    draw_gradient(s);
    draw_slider(s);

    // Preview swatch
    let rgba = s.current_rgba();
    println!("VYOMA_DRAW:fill_rect:{PREVIEW_X},{PREVIEW_Y},{PREVIEW_W},{PREVIEW_H},{rgba:#010x}");
    println!("VYOMA_DRAW:rect_border:{PREVIEW_X},{PREVIEW_Y},{PREVIEW_W},{PREVIEW_H},{C_BORDER:#010x}");

    // Hex label
    let hex = s.hex();
    println!("VYOMA_DRAW:draw_text:{},{},{C_TITLE:#010x},m,{hex}",
        PREVIEW_X, PREVIEW_Y + PREVIEW_H + 10);

    // RGB breakdown
    let r = (rgba >> 24) & 0xFF;
    let g = (rgba >> 16) & 0xFF;
    let b = (rgba >>  8) & 0xFF;
    println!("VYOMA_DRAW:draw_text:{},{},{C_DIM:#010x},m,R:{r} G:{g} B:{b}",
        PREVIEW_X, PREVIEW_Y + PREVIEW_H + 30);

    // HSV values
    println!("VYOMA_DRAW:draw_text:{},{},{C_DIM:#010x},m,H:{} S:{} V:{}",
        PREVIEW_X, PREVIEW_Y + PREVIEW_H + 50, s.hue, s.sat, s.val);

    // Labels
    println!("VYOMA_DRAW:draw_text:{GRAD_X},{},{C_DIM:#010x},m,Hue / Saturation →",
        GRAD_Y - 14);
    println!("VYOMA_DRAW:draw_text:{SLIDER_X},{},{C_DIM:#010x},m,Value →",
        SLIDER_Y + SLIDER_H + 6);

    println!("VYOMA_DRAW:draw_text:20,{},{C_HINT:#010x},m,Click gradient or slider  Ctrl+W: copy  Ctrl+C: quit",
        H - 16);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut s = State::default();
    draw(&s);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        if let Some(rest) = raw.strip_prefix("VYOMA_INPUT:mouse:down:") {
            let parts: Vec<&str> = rest.splitn(2, ',').collect();
            if parts.len() == 2 {
                let mx: u32 = parts[0].parse().unwrap_or(0);
                let my: u32 = parts[1].parse().unwrap_or(0);
                if mx >= GRAD_X && mx < GRAD_X + GRAD_W
                    && my >= GRAD_Y && my < GRAD_Y + GRAD_H
                {
                    let cells = GRAD_W / CELL;
                    let col = ((mx - GRAD_X) / CELL).min(cells - 1);
                    let row = ((my - GRAD_Y) / CELL).min(cells - 1);
                    s.hue = col * 359 / (cells - 1);
                    s.sat = 255 - row * 255 / (cells - 1);
                    draw(&s);
                } else if mx >= SLIDER_X && mx < SLIDER_X + SLIDER_W
                    && my >= SLIDER_Y && my < SLIDER_Y + SLIDER_H
                {
                    let cells = SLIDER_W / CELL;
                    let col = ((mx - SLIDER_X) / CELL).min(cells - 1);
                    s.val = col * 255 / (cells - 1);
                    draw(&s);
                }
            }
            continue;
        }

        match raw.as_str() {
            "\x17" => {
                // Ctrl+W: output color and exit
                let hex = s.hex();
                println!("color: {hex}");
                let _ = io::stdout().flush();
                println!("VYOMA_DRAW:fill_rect:0,0,{W},{H},0x0D1117FF");
                flush();
                std::process::exit(0);
            }
            "\x03" => {
                println!("VYOMA_DRAW:fill_rect:0,0,{W},{H},0x0D1117FF");
                flush();
                std::process::exit(0);
            }
            _ => {}
        }
    }
}
