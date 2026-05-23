use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn border(x: i32, y: i32, w: i32, h: i32, c: u32) {
    println!("VYOMA_DRAW:rect_border:{},{},{},{},{}", x, y, w, h, c);
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

// Pure integer HSV → RGBA
// h: 0-359, s: 0-100, v: 0-100
fn hsv_to_rgba(h: u32, s: u32, v: u32) -> u32 {
    let v8 = v * 255 / 100;
    if s == 0 {
        return (v8 << 24) | (v8 << 16) | (v8 << 8) | 0xFF;
    }
    let region = h / 60;
    let remainder = (h % 60) * 255 / 60;
    let p = v8 * (100 - s) / 100;
    let q = v8 * (100 - s * remainder / 255) / 100;
    let t = v8 * (100 - s * (255 - remainder) / 255) / 100;
    let (r, g, b) = match region {
        0 => (v8, t,  p),
        1 => (q,  v8, p),
        2 => (p,  v8, t),
        3 => (p,  q,  v8),
        4 => (t,  p,  v8),
        _ => (v8, p,  q),
    };
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

fn rgba_to_rgb(c: u32) -> (u8, u8, u8) {
    (((c >> 24) & 0xFF) as u8, ((c >> 16) & 0xFF) as u8, ((c >> 8) & 0xFF) as u8)
}

// Panel focus: 0=HSV square, 1=H slider, 2=S slider, 3=V slider
#[derive(PartialEq, Clone, Copy)]
enum Focus { Hue, Sat, Val, Rgb }

struct App {
    h: u32,    // 0-359
    s: u32,    // 0-100
    v: u32,    // 0-100
    focus: Focus,
    swatches: [u32; 16],
    swatch_count: usize,
}

impl App {
    fn new() -> Self {
        App {
            h: 200, s: 80, v: 90,
            focus: Focus::Hue,
            swatches: [0u32; 16],
            swatch_count: 0,
        }
    }

    fn current_rgba(&self) -> u32 {
        hsv_to_rgba(self.h, self.s, self.v)
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Color Picker");
        text(150, 8, C_HINT, "Tab:focus  Arrows:adjust  S:save swatch  Q:quit");

        let rgba = self.current_rgba();
        let (rr, gg, bb) = rgba_to_rgb(rgba);

        // ── HSV Gradient square (left panel) ──────────────────────────────────
        // 60×60 cells, each 4px = 240×240 px, top-left at (20, 52)
        let sq_x = 20i32;
        let sq_y = 52i32;
        let sq_cells = 60usize;
        let sq_cell = 4i32;
        for sy in 0..sq_cells {
            for sx in 0..sq_cells {
                let s_val = sx as u32 * 100 / (sq_cells as u32 - 1);
                let v_val = 100 - sy as u32 * 100 / (sq_cells as u32 - 1);
                let c = hsv_to_rgba(self.h, s_val, v_val);
                fill(sq_x + sx as i32 * sq_cell, sq_y + sy as i32 * sq_cell, sq_cell, sq_cell, c);
            }
        }
        // Cursor on square
        let cursor_x = sq_x + (self.s as i32 * (sq_cells as i32 - 1) / 100) * sq_cell;
        let cursor_y = sq_y + ((100 - self.v) as i32 * (sq_cells as i32 - 1) / 100) * sq_cell;
        border(cursor_x - 2, cursor_y - 2, sq_cell + 4, sq_cell + 4, 0xFFFFFFFF);
        border(sq_x - 1, sq_y - 1, sq_cells as i32 * sq_cell + 2, sq_cells as i32 * sq_cell + 2,
               if self.focus == Focus::Hue { C_SEL } else { C_BORDER });

        // ── Hue slider ────────────────────────────────────────────────────────
        let sl_x = 20i32;
        let sl_y = sq_y + sq_cells as i32 * sq_cell + 12;
        let sl_w = 240i32;
        let sl_h = 16i32;
        for i in 0..sl_w {
            let hue = i as u32 * 360 / sl_w as u32;
            let c = hsv_to_rgba(hue, 100, 100);
            fill(sl_x + i, sl_y, 1, sl_h, c);
        }
        let hue_x = sl_x + self.h as i32 * sl_w / 360;
        fill(hue_x - 1, sl_y - 2, 3, sl_h + 4, 0xFFFFFFFF);
        border(sl_x - 1, sl_y - 1, sl_w + 2, sl_h + 2,
               if self.focus == Focus::Sat { C_SEL } else { C_BORDER });
        text(sl_x, sl_y - 12, C_HINT, "Hue");

        // ── Saturation slider ─────────────────────────────────────────────────
        let sl2_y = sl_y + sl_h + 28;
        for i in 0..sl_w {
            let s_val = i as u32 * 100 / sl_w as u32;
            let c = hsv_to_rgba(self.h, s_val, self.v);
            fill(sl_x + i, sl2_y, 1, sl_h, c);
        }
        let sat_x = sl_x + self.s as i32 * sl_w / 100;
        fill(sat_x - 1, sl2_y - 2, 3, sl_h + 4, 0xFFFFFFFF);
        border(sl_x - 1, sl2_y - 1, sl_w + 2, sl_h + 2,
               if self.focus == Focus::Val { C_SEL } else { C_BORDER });
        text(sl_x, sl2_y - 12, C_HINT, "Saturation");

        // ── Value slider ──────────────────────────────────────────────────────
        let sl3_y = sl2_y + sl_h + 28;
        for i in 0..sl_w {
            let v_val = i as u32 * 100 / sl_w as u32;
            let c = hsv_to_rgba(self.h, self.s, v_val);
            fill(sl_x + i, sl3_y, 1, sl_h, c);
        }
        let val_x = sl_x + self.v as i32 * sl_w / 100;
        fill(val_x - 1, sl3_y - 2, 3, sl_h + 4, 0xFFFFFFFF);
        border(sl_x - 1, sl3_y - 1, sl_w + 2, sl_h + 2,
               if self.focus == Focus::Rgb { C_SEL } else { C_BORDER });
        text(sl_x, sl3_y - 12, C_HINT, "Value");

        // ── Right info panel ──────────────────────────────────────────────────
        let px = 280i32;
        fill(px, 52, W - px - 8, 420, C_CARD);
        border(px, 52, W - px - 8, 420, C_BORDER);

        // Color preview
        fill(px + 12, 64, 120, 120, rgba);
        border(px + 12, 64, 120, 120, C_BORDER);

        // Hex value
        text(px + 12, 196, C_ORANGE, "HEX");
        text(px + 12, 214, C_TEXT, &format!("#{:02X}{:02X}{:02X}", rr, gg, bb));

        // RGB values
        text(px + 12, 242, C_ORANGE, "RGB");
        text(px + 12, 260, 0xFF6B6BFF, &format!("R: {}", rr));
        text(px + 12, 278, 0x3FB950FF, &format!("G: {}", gg));
        text(px + 12, 296, 0x58A6FFFF, &format!("B: {}", bb));

        // HSV values
        text(px + 12, 324, C_ORANGE, "HSV");
        text(px + 12, 342, C_HINT, &format!("H: {}°", self.h));
        text(px + 12, 360, C_HINT, &format!("S: {}%", self.s));
        text(px + 12, 378, C_HINT, &format!("V: {}%", self.v));

        // Focus indicator
        let focus_label = match self.focus {
            Focus::Hue => "Adjusting: Square (S/V)",
            Focus::Sat => "Adjusting: Hue slider",
            Focus::Val => "Adjusting: Sat slider",
            Focus::Rgb => "Adjusting: Val slider",
        };
        text(px + 12, 412, C_SEL, focus_label);

        // ── Swatches ──────────────────────────────────────────────────────────
        let sw_y = 490i32;
        text(20, sw_y - 16, C_ORANGE, "Saved Swatches (S=save):");
        for i in 0..16 {
            let sx = 20 + i as i32 * 46;
            if i < self.swatch_count {
                fill(sx, sw_y, 40, 40, self.swatches[i]);
                border(sx, sw_y, 40, 40, C_BORDER);
            } else {
                fill(sx, sw_y, 40, 40, C_CARD);
                border(sx, sw_y, 40, 40, C_BORDER);
                text(sx + 14, sw_y + 12, C_BORDER, "+");
            }
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!("#{:02X}{:02X}{:02X}  H={}  S={}  V={}  Swatches: {}/16",
             rr, gg, bb, self.h, self.s, self.v, self.swatch_count));
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\t" => {
                self.focus = match self.focus {
                    Focus::Hue => Focus::Sat,
                    Focus::Sat => Focus::Val,
                    Focus::Val => Focus::Rgb,
                    Focus::Rgb => Focus::Hue,
                };
            }
            "\x1b[C" => {
                match self.focus {
                    Focus::Hue => self.s = (self.s + 2).min(100),
                    Focus::Sat => self.h = (self.h + 3).min(359),
                    Focus::Val => self.s = (self.s + 2).min(100),
                    Focus::Rgb => self.v = (self.v + 2).min(100),
                }
            }
            "\x1b[D" => {
                match self.focus {
                    Focus::Hue => self.s = self.s.saturating_sub(2),
                    Focus::Sat => self.h = self.h.saturating_sub(3),
                    Focus::Val => self.s = self.s.saturating_sub(2),
                    Focus::Rgb => self.v = self.v.saturating_sub(2),
                }
            }
            "\x1b[A" => {
                match self.focus {
                    Focus::Hue => self.v = (self.v + 2).min(100),
                    Focus::Sat => self.h = (self.h + 3).min(359),
                    Focus::Val => self.s = (self.s + 2).min(100),
                    Focus::Rgb => self.v = (self.v + 2).min(100),
                }
            }
            "\x1b[B" => {
                match self.focus {
                    Focus::Hue => self.v = self.v.saturating_sub(2),
                    Focus::Sat => self.h = self.h.saturating_sub(3),
                    Focus::Val => self.s = self.s.saturating_sub(2),
                    Focus::Rgb => self.v = self.v.saturating_sub(2),
                }
            }
            "s" | "S" => {
                if self.swatch_count < 16 {
                    self.swatches[self.swatch_count] = self.current_rgba();
                    self.swatch_count += 1;
                } else {
                    // Replace last swatch
                    self.swatches[15] = self.current_rgba();
                }
            }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => {}
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
    }
}
