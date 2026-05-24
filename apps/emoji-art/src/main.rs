// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
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

struct Art {
    title: &'static str,
    rows: &'static [&'static str],
    fg: u32,
    bg: u32,
}

static ARTS: &[Art] = &[
    Art { title: "Heart", fg: 0xFF6B6BFF, bg: 0x1A0A0AFF, rows: &[
        "  ###   ###  ",
        " ##### ##### ",
        "#############",
        "#############",
        " ########### ",
        "  #########  ",
        "   #######   ",
        "    #####    ",
        "     ###     ",
        "      #      ",
    ]},
    Art { title: "Star", fg: 0xFFD700FF, bg: 0x0A0A1AFF, rows: &[
        "      *      ",
        "     ***     ",
        "*************",
        " *********** ",
        "  *  ***  *  ",
        " *   ***   * ",
        "*    * *    *",
    ]},
    Art { title: "House", fg: 0xFFA657FF, bg: 0x0D1117FF, rows: &[
        "      #      ",
        "     ###     ",
        "    #####    ",
        "   #######   ",
        "  #########  ",
        " ########### ",
        "#############",
        "## ## ## ## #",
        "## ## ## ## #",
        "#####_#######",
    ]},
    Art { title: "Tree", fg: 0x3FB950FF, bg: 0x0A1A0AFF, rows: &[
        "      *      ",
        "     ***     ",
        "    *****    ",
        "   *******   ",
        "  *********  ",
        " *********** ",
        "*************",
        "     ***     ",
        "     ***     ",
        "     ***     ",
    ]},
    Art { title: "Sun", fg: 0xFFD700FF, bg: 0x001030FF, rows: &[
        " * *   * * * ",
        "  *** *** *  ",
        "   *******   ",
        "* *#######* *",
        "  *#######*  ",
        "* *#######* *",
        "   *******   ",
        "  *** *** *  ",
        " * *   * * * ",
    ]},
    Art { title: "Moon", fg: 0xE6EDF3FF, bg: 0x020510FF, rows: &[
        "    ####     ",
        "  ########   ",
        " ##########  ",
        "###########  ",
        "###########  ",
        "###########  ",
        " ##########  ",
        "  ########   ",
        "    ####     ",
    ]},
    Art { title: "Fish", fg: 0x58A6FFFF, bg: 0x001030FF, rows: &[
        "  *          ",
        " *  ******   ",
        "*  ########* ",
        "* ###@#####* ",
        "*  ########* ",
        " *  ******   ",
        "  *          ",
    ]},
    Art { title: "Bird", fg: 0xE6EDF3FF, bg: 0x001A30FF, rows: &[
        "*           *",
        "**         **",
        "***       ***",
        " **  *** **  ",
        "   * *** *   ",
        "   *  *  *   ",
        "    * * *    ",
        "     ***     ",
    ]},
    Art { title: "Diamond", fg: 0x79C0FFFF, bg: 0x0D1117FF, rows: &[
        "      *      ",
        "     ***     ",
        "    *****    ",
        "   *######   ",
        "  *########  ",
        "   ########  ",
        "    ######   ",
        "     ####    ",
        "      ##     ",
        "       *     ",
    ]},
    Art { title: "Rocket", fg: 0xE6EDF3FF, bg: 0x010108FF, rows: &[
        "      *      ",
        "     ***     ",
        "    *###*    ",
        "    *###*    ",
        "   **###**   ",
        "  ***###***  ",
        " *###   ###* ",
        "  **     **  ",
        "   * *** *   ",
        "     * *     ",
    ]},
    Art { title: "Castle", fg: 0xBC8CFFFF, bg: 0x0D1117FF, rows: &[
        "# # # # # # #",
        "#############",
        "#   ## ##   #",
        "#   ## ##   #",
        "#############",
        "#     _     #",
        "#    ###    #",
        "#    ###    #",
        "#############",
        "#############",
    ]},
    Art { title: "Crown", fg: 0xFFD700FF, bg: 0x1A1000FF, rows: &[
        "*   *   *   *",
        "**  *   *  **",
        "*** *   * ***",
        "**** * * ****",
        "#############",
        "#############",
        " ########### ",
        "  #########  ",
    ]},
    Art { title: "Anchor", fg: 0x58A6FFFF, bg: 0x001030FF, rows: &[
        "    *****    ",
        "   *     *   ",
        "    *   *    ",
        "     *#*     ",
        " ****###**** ",
        "*   *###*   *",
        " *  *###*  * ",
        "  * *###* *  ",
        "   **###**   ",
        "    *   *    ",
    ]},
    Art { title: "Hourglass", fg: 0xFFA657FF, bg: 0x0D1117FF, rows: &[
        "#############",
        " ########### ",
        "  #########  ",
        "   #######   ",
        "    #####    ",
        "     ###     ",
        "    #####    ",
        "   #######   ",
        "  #########  ",
        " ########### ",
        "#############",
    ]},
    Art { title: "Eye", fg: 0x3FB950FF, bg: 0x0D1117FF, rows: &[
        "   *******   ",
        " ***     *** ",
        "*   *****   *",
        "*  *#####*  *",
        "* *##   ##* *",
        "* *## @ ##* *",
        "* *##   ##* *",
        "*  *#####*  *",
        "*   *****   *",
        " ***     *** ",
        "   *******   ",
    ]},
    Art { title: "Mushroom", fg: 0xFF7B72FF, bg: 0x0A1A0AFF, rows: &[
        "   #######   ",
        "  #########  ",
        " ########### ",
        "#############",
        "# * # # * # #",
        "#############",
        " ########### ",
        "    #####    ",
        "    #####    ",
        "   #######   ",
    ]},
    Art { title: "Infinity", fg: 0xBC8CFFFF, bg: 0x0D1117FF, rows: &[
        "  ***   ***  ",
        " *   * *   * ",
        "*     *     *",
        "*     *     *",
        " *   * *   * ",
        "  ***   ***  ",
    ]},
    Art { title: "Lightning", fg: 0xFFD700FF, bg: 0x0D0D00FF, rows: &[
        "     ######  ",
        "    ######   ",
        "   ######    ",
        "  ######     ",
        " ##########  ",
        "  ##########",
        "     ######  ",
        "    ######   ",
        "   ######    ",
        "  ####       ",
    ]},
    Art { title: "Mountain", fg: 0xE6EDF3FF, bg: 0x001020FF, rows: &[
        "      *      ",
        "     ***     ",
        "    *****    ",
        "   *##***    ",
        "  *####***   ",
        " *########** ",
        "*##########**",
        "*############",
        "#############",
    ]},
    Art { title: "Flower", fg: 0xFF6B6BFF, bg: 0x0A1A0AFF, rows: &[
        "  *  ***  *  ",
        " **  ***  ** ",
        "***  ***  ***",
        " *** *** *** ",
        "  ***#*#***  ",
        "   *#####*   ",
        "  ***#*#***  ",
        " *** *** *** ",
        "***  ***  ***",
        " **  ***  ** ",
        "  *  ***  *  ",
    ]},
];

struct App {
    idx:  usize,
    zoom: usize,
}

impl App {
    fn new() -> Self { App { idx: 0, zoom: 1 } }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Emoji Art");
        text(130, 8, C_HINT, "Left/Right:prev/next  Z:zoom  Q:quit");

        let art = &ARTS[self.idx];

        // Title bar
        fill(0, 32, W, 20, 0x161B22FF);
        text(W / 2 - 40, 36, C_ORANGE, art.title);
        text(W - 120, 36, C_HINT, &format!("{}/{}", self.idx + 1, ARTS.len()));

        // Canvas
        let cw = W - 40;
        let ch = H - 80;
        fill(20, 55, cw, ch, art.bg);
        border(20, 55, cw, ch, C_BORDER);

        let cell = self.zoom as i32 * 8;
        let rows = art.rows.len() as i32;
        let cols = art.rows.iter().map(|r| r.len()).max().unwrap_or(0) as i32;

        let ox = 20 + (cw - cols * cell) / 2;
        let oy = 55 + (ch - rows * cell) / 2;

        for (r, row) in art.rows.iter().enumerate() {
            for (c, ch_byte) in row.bytes().enumerate() {
                let x = ox + c as i32 * cell;
                let y = oy + r as i32 * cell;
                let color = match ch_byte {
                    b'#' => art.fg,
                    b'*' => {
                        // lighter variant
                        let r2 = ((art.fg >> 24) & 0xFF).saturating_add(30).min(255);
                        let g2 = ((art.fg >> 16) & 0xFF).saturating_add(30).min(255);
                        let b2 = ((art.fg >> 8)  & 0xFF).saturating_add(30).min(255);
                        (r2 << 24) | (g2 << 16) | (b2 << 8) | 0xFF
                    }
                    b'@' => 0xFFFFFFFF,
                    b'_' => 0x4A3728FF,
                    _ => continue,
                };
                fill(x, y, cell, cell, color);
            }
        }

        // Status bar
        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, &format!("Art: {}  Zoom: {}x  Cells: {}x{}", art.title, self.zoom, cols, rows));
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[D" | "h" | "H" => {
                self.idx = if self.idx == 0 { ARTS.len() - 1 } else { self.idx - 1 };
            }
            "\x1b[C" | "l" | "L" => {
                self.idx = (self.idx + 1) % ARTS.len();
            }
            "z" | "Z" => {
                self.zoom = if self.zoom >= 4 { 1 } else { self.zoom + 1 };
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
