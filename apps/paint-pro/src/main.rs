use std::io::{self, BufRead, Write};

const W: u32 = 1280;
const H: u32 = 800;
const HEADER_H: u32 = 48;

const COLS: usize = 200;
const ROWS: usize = 150;
const CELL: u32 = 5;
const CANVAS_X: u32 = 0;
const CANVAS_Y: u32 = HEADER_H;

const PAL_X: u32 = COLS as u32 * CELL + 8; // 1008
const PAL_SZ: u32 = 40;
const PAL_GAP: u32 = 6;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_EMPTY: u32  = 0x1A1A1AFF;

const PALETTE: [u32; 10] = [
    0x000000FF, 0xFFFFFFFF, 0xFF3B30FF, 0x3FB950FF,
    0x0A84FFFF, 0xFFD60AFF, 0x32D2FFFF, 0xFF375FFF,
    0xA2845EFF, 0x8E8E93FF,
];
const PAL_NAMES: [&str; 10] = [
    "Black", "White", "Red", "Green", "Blue", "Yellow", "Cyan", "Magenta", "Brown", "Gray",
];

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

fn idx(r: usize, c: usize) -> usize { r * COLS + c }

struct App {
    canvas:  Vec<u32>,        // ROWS*COLS
    undo:    Vec<Vec<u32>>,   // up to 10 snapshots
    cx:      usize,
    cy:      usize,
    color:   usize,
    brush:   usize,           // 1, 2, or 3
    drawing: bool,
    status:  String,
}

impl App {
    fn new() -> Self {
        App {
            canvas: vec![C_EMPTY; ROWS * COLS],
            undo: Vec::new(),
            cx: 0,
            cy: 0,
            color: 1, // white
            brush: 1,
            drawing: false,
            status: String::new(),
        }
    }

    fn snapshot(&mut self) {
        if self.undo.len() >= 10 { self.undo.remove(0); }
        self.undo.push(self.canvas.clone());
    }

    fn undo(&mut self) {
        if let Some(snap) = self.undo.pop() {
            self.canvas = snap;
        }
    }

    fn paint_at(&mut self, r: usize, c: usize) {
        let color = PALETTE[self.color];
        let half = (self.brush as i32 - 1) / 2;
        let dim = self.brush as i32;
        for dr in 0..dim {
            for dc in 0..dim {
                let nr = r as i32 + dr - half;
                let nc = c as i32 + dc - half;
                if nr >= 0 && nr < ROWS as i32 && nc >= 0 && nc < COLS as i32 {
                    // Opacity simulation: corners of brush > 1 get C_HINT overlay effect
                    let is_edge = self.brush > 1 && (dr == 0 || dr == dim-1) && (dc == 0 || dc == dim-1);
                    if is_edge {
                        // blend: mix color with existing at 50%
                        let existing = self.canvas[idx(nr as usize, nc as usize)];
                        let r1 = ((color >> 24) & 0xFF) / 2 + ((existing >> 24) & 0xFF) / 2;
                        let g1 = ((color >> 16) & 0xFF) / 2 + ((existing >> 16) & 0xFF) / 2;
                        let b1 = ((color >> 8) & 0xFF) / 2 + ((existing >> 8) & 0xFF) / 2;
                        self.canvas[idx(nr as usize, nc as usize)] = (r1 << 24) | (g1 << 16) | (b1 << 8) | 0xFF;
                    } else {
                        self.canvas[idx(nr as usize, nc as usize)] = color;
                    }
                }
            }
        }
    }

    fn erase_at(&mut self, r: usize, c: usize) {
        let half = (self.brush as i32 - 1) / 2;
        let dim = self.brush as i32;
        for dr in 0..dim {
            for dc in 0..dim {
                let nr = r as i32 + dr - half;
                let nc = c as i32 + dc - half;
                if nr >= 0 && nr < ROWS as i32 && nc >= 0 && nc < COLS as i32 {
                    self.canvas[idx(nr as usize, nc as usize)] = C_EMPTY;
                }
            }
        }
    }

    fn flood_fill(&mut self) {
        let target = self.canvas[idx(self.cy, self.cx)];
        let fill_color = PALETTE[self.color];
        if target == fill_color { return; }
        let mut queue: Vec<(usize, usize)> = vec![(self.cy, self.cx)];
        self.canvas[idx(self.cy, self.cx)] = fill_color;
        let mut qi = 0;
        while qi < queue.len() {
            let (r, c) = queue[qi]; qi += 1;
            let neighbors = [(r.wrapping_sub(1), c), (r+1, c), (r, c.wrapping_sub(1)), (r, c+1)];
            for (nr, nc) in neighbors {
                if nr < ROWS && nc < COLS && self.canvas[idx(nr, nc)] == target {
                    self.canvas[idx(nr, nc)] = fill_color;
                    queue.push((nr, nc));
                }
            }
        }
    }

    fn clear(&mut self) {
        self.snapshot();
        self.canvas = vec![C_EMPTY; ROWS * COLS];
    }

    fn save(&mut self) {
        let mut bytes = Vec::with_capacity(ROWS * COLS * 4);
        for &px in &self.canvas {
            bytes.extend_from_slice(&px.to_le_bytes());
        }
        match std::fs::write("/data/paint-pro.bin", &bytes) {
            Ok(_) => self.status = "Saved to /data/paint-pro.bin".to_string(),
            Err(e) => self.status = format!("Save failed: {}", e),
        }
    }

    fn load(&mut self) {
        match std::fs::read("/data/paint-pro.bin") {
            Ok(bytes) if bytes.len() == ROWS * COLS * 4 => {
                self.snapshot();
                for (i, chunk) in bytes.chunks(4).enumerate() {
                    if i < ROWS * COLS {
                        self.canvas[i] = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    }
                }
                self.status = "Loaded /data/paint-pro.bin".to_string();
            }
            Ok(_) => self.status = "Load failed: wrong file size".to_string(),
            Err(e) => self.status = format!("Load failed: {}", e),
        }
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Paint Pro");
    text(140, 16, C_HINT, &format!("Color:{} Brush:{} ({},{})", PAL_NAMES[app.color], app.brush, app.cx, app.cy));
    text(560, 16, C_HINT, "Arrows:move Spc:draw E:erase F:fill C:clear U:undo 1-3:brush");

    // Canvas
    for r in 0..ROWS {
        for c in 0..COLS {
            let px = CANVAS_X + c as u32 * CELL;
            let py = CANVAS_Y + r as u32 * CELL;
            fill(px, py, CELL, CELL, app.canvas[idx(r, c)]);
        }
    }

    // Cursor
    let cur_half = (app.brush as i32 - 1) * CELL as i32 / 2;
    let cx = CANVAS_X + app.cx as u32 * CELL - cur_half as u32;
    let cy = CANVAS_Y + app.cy as u32 * CELL - cur_half as u32;
    let csz = app.brush as u32 * CELL;
    border(cx, cy, csz, csz, C_SEL);

    // Palette
    text(PAL_X, CANVAS_Y, C_HINT, "Palette");
    for i in 0..10 {
        let py = CANVAS_Y + 20 + i as u32 * (PAL_SZ + PAL_GAP);
        fill(PAL_X, py, PAL_SZ, PAL_SZ, PALETTE[i]);
        if i == app.color {
            border(PAL_X - 2, py - 2, PAL_SZ + 4, PAL_SZ + 4, C_SEL);
        } else {
            border(PAL_X, py, PAL_SZ, PAL_SZ, C_BORDER);
        }
        text(PAL_X + PAL_SZ + 4, py + 12, C_HINT, PAL_NAMES[i]);
    }

    // Brush size indicator
    let brush_y = CANVAS_Y + 20 + 10 * (PAL_SZ + PAL_GAP) + 12;
    text(PAL_X, brush_y, C_HINT, &format!("Brush: {}", app.brush));
    text(PAL_X, brush_y + 20, C_HINT, "Keys 1 2 3");

    // Undo count
    text(PAL_X, brush_y + 44, C_HINT, &format!("Undo: {}/10", app.undo.len()));

    // Ctrl+W/L hint
    text(PAL_X, brush_y + 68, C_HINT, "Ctrl+W: save");
    text(PAL_X, brush_y + 88, C_HINT, "Ctrl+L: load");

    // Status
    if !app.status.is_empty() {
        text(16, H - 20, C_GREEN, &app.status);
    }

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise paint-pro");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                if app.cy > 0 { app.cy -= 1; }
                if app.drawing { app.paint_at(app.cy, app.cx); }
            }
            "\x1b[B" => {
                if app.cy + 1 < ROWS { app.cy += 1; }
                if app.drawing { app.paint_at(app.cy, app.cx); }
            }
            "\x1b[C" => {
                if app.cx + 1 < COLS { app.cx += 1; }
                if app.drawing { app.paint_at(app.cy, app.cx); }
            }
            "\x1b[D" => {
                if app.cx > 0 { app.cx -= 1; }
                if app.drawing { app.paint_at(app.cy, app.cx); }
            }
            " " => {
                if !app.drawing { app.snapshot(); }
                app.drawing = !app.drawing;
                if app.drawing { app.paint_at(app.cy, app.cx); }
            }
            "e" | "E" => { app.snapshot(); app.erase_at(app.cy, app.cx); }
            "f" | "F" => { app.snapshot(); app.flood_fill(); }
            "c" | "C" => { app.clear(); }
            "u" | "U" | "\x1a" => { app.undo(); }
            "\x17" => { app.save(); }   // Ctrl+W
            "\x0c" => { app.load(); }   // Ctrl+L
            "\t" => { app.color = (app.color + 1) % 10; }
            "1" => { app.brush = 1; }
            "2" => { app.brush = 2; }
            "3" => { app.brush = 3; }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= b'1' && b <= b'9' {
                    let i = (b - b'1') as usize;
                    if i < 10 { app.color = i; }
                }
            }
            _ => {}
        }
        draw(&app);
    }
}
