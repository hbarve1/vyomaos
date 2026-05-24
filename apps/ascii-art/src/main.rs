// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 680;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_GREEN: u32  = 0x3FB950FF;

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

// 10-level intensity palette; image chars must come from this set
const FULL: &[u8] = b" .:-=+*#%@";

const CHARSETS: [(&str, &str); 8] = [
    (" .",        "Minimal  (2)"),
    (" .:",        "Sparse   (3)"),
    (" .:-",       "Light    (4)"),
    (" .:-=",      "Medium   (5)"),
    (" .:-=+",     "Normal   (6)"),
    (" .:-=+*",    "Dense    (7)"),
    (" .:-=+*#",   "Heavy    (8)"),
    (" .:-=+*#%@", "Full    (10)"),
];

static IMAGES: &[(&str, &[&str])] = &[
    ("Heart", &[
        "   =*=   =*=   ",
        "  =*##= =##*=  ",
        " =*##########*=",
        "=*############*=",
        "*##############*",
        " *############* ",
        "  *##########*  ",
        "   *########*   ",
        "    *######*    ",
        "     *####*     ",
        "      *##*      ",
        "       **       ",
        "        *       ",
    ]),
    ("Diamond", &[
        "          .          ",
        "         .:.         ",
        "        .:=:.        ",
        "       .:=+=:.       ",
        "      .:=+*+=:.      ",
        "     .:=+*#*+=:.     ",
        "    .:=+*###*+=:.    ",
        "     .:=+*#*+=:.     ",
        "      .:=+*+=:.      ",
        "       .:=+=:.       ",
        "        .:=:.        ",
        "         .:.         ",
        "          .          ",
    ]),
    ("Mountain", &[
        "             *               ",
        "            *#*              ",
        "           *###*             ",
        "     *    *#####*    *       ",
        "    *#*  *#######*  *#*      ",
        "   *###**#########**###*     ",
        "  *######*#######*######*    ",
        " *########*#####*########*   ",
        "*##########*###*##########*  ",
        "============***=============  ",
    ]),
    ("Tree", &[
        "          *          ",
        "         *#*         ",
        "        *###*        ",
        "       *#####*       ",
        "      *#######*      ",
        "     *#########*     ",
        "       *#####*       ",
        "      *#######*      ",
        "     *#########*     ",
        "    *###########*    ",
        "   *#############*   ",
        "        *###*        ",
        "       *#####*       ",
        "        *###*        ",
    ]),
    ("Star", &[
        "          @          ",
        "         @#@         ",
        " @  .  @###@  .  @   ",
        "  @=  =@####=  =@    ",
        "   @+*##########*+@  ",
        "  @@@#############@@@",
        "   @+*##########*+@  ",
        "  @=  =@####=  =@    ",
        " @  .  @###@  .  @   ",
        "         @#@         ",
        "          @          ",
    ]),
    ("Rocket", &[
        "      .##.      ",
        "     .####.     ",
        "    .######.    ",
        "    ########    ",
        "   ##########   ",
        "   ##+=====+##  ",
        "   ##+=====+##  ",
        "   ##########   ",
        "   #####*#####  ",
        "   ####   ####  ",
        "  ..*. . . .*.. ",
        " ..             ..",
    ]),
    ("Castle", &[
        "  #   #   #   #   #  ",
        "  #   #   #   #   #  ",
        "  #####################",
        "  ##*################*##",
        "  ####=+##########+=####",
        "  #####-=########=-#####",
        "  ######...######...####",
        "  ########.####.########",
        "  #########*##*#########",
        "  ###########*###########",
        "  ###########*###########",
        "  ###########*###########",
        "  ###########*###########",
        "  ###########*###########",
        "  ###########*###########",
    ]),
    ("Eye", &[
        "      ..:===========:..      ",
        "    .:=+*############*+=:.   ",
        "   .:+*################*+:.  ",
        "  .:-*########@@########*-:. ",
        "  .:-*########@@########*-:. ",
        "   .:+*################*+:.  ",
        "    .:=+*############*+=:.   ",
        "      ..:===========:..      ",
    ]),
    ("Fish", &[
        "       . .:.              ",
        "      .:-*#=.             ",
        "    .:-=*####=:..         ",
        "  .:-=+*########=:..      ",
        ".:-=+**#############+=:.  ",
        " .:=+*###############@*:. ",
        "  .:-=+*############*=:.  ",
        "    .:-=+**#######*+=:.   ",
        "       .:-=+*####*=:. ..  ",
        "            .:-=*=:..::.. ",
    ]),
    ("Sun", &[
        "           @             ",
        "    .      @      .      ",
        "     .:   @#@   :.       ",
        "      .:-*###*-:.        ",
        "   .  .:*#####*:.  .     ",
        "   .:-+*########*+-:.    ",
        "@@@##*###############*##@@@",
        "   .:-+*########*+-:.    ",
        "   .  .:*#####*:.  .     ",
        "      .:-*###*-:.        ",
        "     .:   @#@   :.       ",
        "    .      @      .      ",
        "           @             ",
    ]),
];

fn map_char(c: char, charset: &[u8], invert: bool) -> char {
    let idx = FULL.iter().position(|&b| b == c as u8).unwrap_or(0);
    let n = charset.len();
    let mapped = idx * (n - 1) / 9;
    let mapped = if invert { n - 1 - mapped } else { mapped };
    charset[mapped] as char
}

struct App {
    image:   usize,
    density: usize,
    invert:  bool,
}

impl App {
    fn new() -> Self { App { image: 0, density: 7, invert: false } }

    fn draw(&self) {
        let (img_name, rows) = IMAGES[self.image];
        let (charset_str, charset_name) = CHARSETS[self.density];
        let charset_bytes = charset_str.as_bytes();

        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "ASCII Art Gallery");
        text(240, 8, C_HINT, "←→:image  D:density  I:invert  Q:quit");

        // Sidebar
        let sx = 720i32;
        fill(sx, 36, W - sx, H - 60, C_CARD);
        border(sx, 36, W - sx, H - 60, C_BORDER);
        text(sx + 12, 44, C_ORANGE, img_name);
        text(sx + 12, 62, C_HINT, &format!("Image {} / {}", self.image + 1, IMAGES.len()));
        text(sx + 12, 82, C_HINT, "Charset:");
        text(sx + 12, 100, C_SEL, charset_name);

        if self.invert {
            text(sx + 12, 120, C_GREEN, "INVERTED");
        }

        text(sx + 12, 146, C_HINT, "Density:");
        for i in 0..8 {
            let (_, cname) = CHARSETS[i];
            let cy = 162 + i as i32 * 18;
            let cc = if i == self.density { C_SEL } else { C_HINT };
            let marker = if i == self.density { ">" } else { " " };
            text(sx + 12, cy, cc, &format!("{} {}", marker, cname));
        }

        // Image canvas (centered in left 720px area)
        let max_w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(1) as i32;
        let img_h  = rows.len() as i32;
        let area_h = H - 60 - 36;
        let canvas_x = (sx - max_w * 8) / 2;
        let canvas_y = 36 + (area_h - img_h * 16) / 2;

        fill(canvas_x - 8, canvas_y - 8, max_w * 8 + 16, img_h * 16 + 16, C_CARD);
        border(canvas_x - 8, canvas_y - 8, max_w * 8 + 16, img_h * 16 + 16, C_BORDER);

        let fg = if self.invert { C_HINT } else { C_TEXT };
        for (r, row) in rows.iter().enumerate() {
            let mapped: String = row.chars()
                .map(|c| map_char(c, charset_bytes, self.invert))
                .collect();
            text(canvas_x, canvas_y + r as i32 * 16, fg, &mapped);
        }

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!(
            "{} | {} | {} | image {}/{}",
            img_name, charset_name,
            if self.invert { "inverted" } else { "normal" },
            self.image + 1, IMAGES.len()
        ));

        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[C" => { self.image = (self.image + 1) % IMAGES.len(); }
            "\x1b[D" => { self.image = (self.image + IMAGES.len() - 1) % IMAGES.len(); }
            "d" | "D" => { self.density = (self.density + 1) % CHARSETS.len(); }
            "i" | "I" => { self.invert = !self.invert; }
            "q" | "Q" | "\x03" => std::process::exit(0),
            _ => return,
        }
        self.draw();
    }
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();
    app.draw();
    let _ = io::stdout().flush();

    for line in stdin.lock().lines() {
        let line = match line { Ok(l) => l, Err(_) => break };
        app.handle(&line);
        let _ = io::stdout().flush();
    }
}
