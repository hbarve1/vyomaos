use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;

const CANVAS_COLS: usize = 60;
const CANVAS_ROWS: usize = 25;
const CHAR_W: u32 = 12;
const CHAR_H: u32 = 20;

const CANVAS_X: u32 = 40;
const CANVAS_Y: u32 = HEADER_H + 8;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD: u32    = 0x161B22FF;
const C_GREEN: u32   = 0x3FB950FF;

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

// Palette of brush characters
const PALETTE: &[char] = &[
    ' ', '.',  ':',  '*',  '#',  '@',
    '|',  '-',  '+',  '/',  '\\', 'X',
    'O',  '0',  'o',  '.',  ',',  '\'',
    '!',  '~',  '%',  '&',  '$',  '^',
    '<',  '>',  '(',  ')',  '[',  ']',
    '{',  '}',  '?',  ';',
];

struct Canvas {
    cells:    [[char; CANVAS_COLS]; CANVAS_ROWS],
    cursor_c: usize,
    cursor_r: usize,
    brush:    usize, // index into PALETTE
    drawing:  bool,  // true = placing brush on arrow move
}

impl Canvas {
    fn new() -> Self {
        Canvas {
            cells: [[' '; CANVAS_COLS]; CANVAS_ROWS],
            cursor_c: 0,
            cursor_r: 0,
            brush: 1, // '.'
            drawing: false,
        }
    }

    fn place(&mut self) {
        self.cells[self.cursor_r][self.cursor_c] = PALETTE[self.brush];
    }

    fn erase(&mut self) {
        self.cells[self.cursor_r][self.cursor_c] = ' ';
    }

    fn save(&self) -> Result<(), String> {
        let mut out = String::new();
        for row in &self.cells {
            let line: String = row.iter()
                .collect::<String>()
                .trim_end()
                .to_string();
            out.push_str(&line);
            out.push('\n');
        }
        std::fs::write("/data/ascii-art.txt", out).map_err(|e| e.to_string())
    }

    fn load(&mut self) -> Result<(), String> {
        let content = std::fs::read_to_string("/data/ascii-art.txt").map_err(|e| e.to_string())?;
        for (r, line) in content.lines().enumerate().take(CANVAS_ROWS) {
            for (c, ch) in line.chars().enumerate().take(CANVAS_COLS) {
                if ch.is_ascii() { self.cells[r][c] = ch; }
            }
        }
        Ok(())
    }
}

fn draw(canvas: &Canvas, status: &str) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "ASCII Art Editor");
    text(240, 16, C_HINT, "←→↑↓:move  Space:draw  Bksp:erase  Tab:brush  Ctrl+W:save  Ctrl+L:load");

    // Canvas background
    let cw = CANVAS_COLS as u32 * CHAR_W;
    let ch = CANVAS_ROWS as u32 * CHAR_H;
    fill(CANVAS_X - 2, CANVAS_Y - 2, cw + 4, ch + 4, C_CARD);
    border(CANVAS_X - 2, CANVAS_Y - 2, cw + 4, ch + 4, C_BORDER);

    // Render each row as a string
    for r in 0..CANVAS_ROWS {
        let row_str: String = canvas.cells[r].iter().collect();
        let ry = CANVAS_Y + r as u32 * CHAR_H;
        text(CANVAS_X, ry + 3, C_TEXT, &row_str);
    }

    // Cursor highlight
    let cx = CANVAS_X + canvas.cursor_c as u32 * CHAR_W;
    let cy = CANVAS_Y + canvas.cursor_r as u32 * CHAR_H;
    border(cx, cy, CHAR_W, CHAR_H, C_SEL);

    // Brush palette panel (right side)
    let px = CANVAS_X + cw + 16;
    let py = CANVAS_Y;
    text(px, py, C_HINT, "Brush:");
    for (i, &ch) in PALETTE.iter().enumerate() {
        let pi = i as u32;
        let col = (pi % 6) as u32;
        let row = (pi / 6) as u32;
        let bx = px + col * 20;
        let by = py + 20 + row * 20;
        if i == canvas.brush {
            fill(bx - 2, by - 1, 18, 18, C_SEL);
            text(bx, by, C_BG, &ch.to_string());
        } else {
            text(bx, by, C_HINT, &ch.to_string());
        }
    }

    // Current brush large display
    let bbx = px + 60;
    let bby = py + 160;
    text(px, bby - 2, C_HINT, "Current:");
    fill(bbx - 4, bby - 4, 28, 28, C_CARD);
    text(bbx, bby, C_ORANGE, &PALETTE[canvas.brush].to_string());

    // Draw mode indicator
    if canvas.drawing {
        text(px, bby + 30, C_GREEN, "DRAW MODE");
    }

    // Canvas info
    text(px, bby + 52, C_HINT, &format!("{}x{}", CANVAS_COLS, CANVAS_ROWS));
    text(px, bby + 72, C_HINT, &format!("({},{})", canvas.cursor_c + 1, canvas.cursor_r + 1));

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let msg = if status.is_empty() {
        format!("Col:{} Row:{}  Brush:'{}'  Tab=next brush  Space=draw  Bksp=erase",
            canvas.cursor_c + 1, canvas.cursor_r + 1, PALETTE[canvas.brush])
    } else {
        status.to_string()
    };
    text(8, sb_y + 6, C_HINT, &msg);
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut canvas = Canvas::new();
    let mut status = String::new();

    println!("@supervisor: raise ascii-art");
    let _ = io::stdout().flush();
    draw(&canvas, &status);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        status.clear();
        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                canvas.cursor_r = canvas.cursor_r.saturating_sub(1);
                if canvas.drawing { canvas.place(); }
            }
            "\x1b[B" => {
                if canvas.cursor_r + 1 < CANVAS_ROWS { canvas.cursor_r += 1; }
                if canvas.drawing { canvas.place(); }
            }
            "\x1b[C" => {
                if canvas.cursor_c + 1 < CANVAS_COLS { canvas.cursor_c += 1; }
                if canvas.drawing { canvas.place(); }
            }
            "\x1b[D" => {
                canvas.cursor_c = canvas.cursor_c.saturating_sub(1);
                if canvas.drawing { canvas.place(); }
            }
            " " => { canvas.place(); }
            "\x7f" => { canvas.erase(); }
            "\t" => {
                canvas.brush = (canvas.brush + 1) % PALETTE.len();
            }
            "d" | "D" => {
                canvas.drawing = !canvas.drawing;
                status = format!("Draw mode: {}", if canvas.drawing { "ON" } else { "OFF" });
            }
            "c" | "C" => {
                canvas.cells = [[' '; CANVAS_COLS]; CANVAS_ROWS];
                status = "Canvas cleared".to_string();
            }
            "\x17" => { // Ctrl+W
                match canvas.save() {
                    Ok(())  => status = "Saved to /data/ascii-art.txt".to_string(),
                    Err(e)  => status = format!("Save error: {}", e),
                }
            }
            "\x0c" => { // Ctrl+L
                match canvas.load() {
                    Ok(())  => status = "Loaded from /data/ascii-art.txt".to_string(),
                    Err(e)  => status = format!("Load error: {}", e),
                }
            }
            s if s.len() == 1 => {
                let b = s.as_bytes()[0];
                if b >= 0x21 && b < 0x7f {
                    // Type a character directly onto canvas
                    canvas.cells[canvas.cursor_r][canvas.cursor_c] = b as char;
                    if canvas.cursor_c + 1 < CANVAS_COLS { canvas.cursor_c += 1; }
                }
            }
            _ => {}
        }
        draw(&canvas, &status);
    }
}
