// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: i32 = 1100;
const H: i32 = 720;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
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

const CATS: [(&str, u32); 11] = [
    ("Alkali Metal",      0xFF7B72FF),
    ("Alkaline Earth",    0xFFA657FF),
    ("Transition Metal",  0x58A6FFFF),
    ("Post-Transition",   0x8B949EFF),
    ("Metalloid",         0xD29922FF),
    ("Reactive Nonmetal", 0x3FB950FF),
    ("Halogen",           0x3BC9B0FF),
    ("Noble Gas",         0xBC8CFFFF),
    ("Lanthanide",        0x2EA7CCFF),
    ("Actinide",          0x4A8DB8FF),
    ("Unknown",           0x6E7681FF),
];

// (symbol, name, category, mass)
static ELEMENTS: &[(&str, &str, u8, &str)] = &[
    ("H",  "Hydrogen",      5, "1.008"),   // 1
    ("He", "Helium",        7, "4.003"),   // 2
    ("Li", "Lithium",       0, "6.941"),   // 3
    ("Be", "Beryllium",     1, "9.012"),   // 4
    ("B",  "Boron",         4, "10.81"),   // 5
    ("C",  "Carbon",        5, "12.011"),  // 6
    ("N",  "Nitrogen",      5, "14.007"),  // 7
    ("O",  "Oxygen",        5, "15.999"),  // 8
    ("F",  "Fluorine",      6, "18.998"),  // 9
    ("Ne", "Neon",          7, "20.180"),  // 10
    ("Na", "Sodium",        0, "22.990"),  // 11
    ("Mg", "Magnesium",     1, "24.305"),  // 12
    ("Al", "Aluminum",      3, "26.982"),  // 13
    ("Si", "Silicon",       4, "28.086"),  // 14
    ("P",  "Phosphorus",    5, "30.974"),  // 15
    ("S",  "Sulfur",        5, "32.06"),   // 16
    ("Cl", "Chlorine",      6, "35.45"),   // 17
    ("Ar", "Argon",         7, "39.948"),  // 18
    ("K",  "Potassium",     0, "39.098"),  // 19
    ("Ca", "Calcium",       1, "40.078"),  // 20
    ("Sc", "Scandium",      2, "44.956"),  // 21
    ("Ti", "Titanium",      2, "47.867"),  // 22
    ("V",  "Vanadium",      2, "50.942"),  // 23
    ("Cr", "Chromium",      2, "51.996"),  // 24
    ("Mn", "Manganese",     2, "54.938"),  // 25
    ("Fe", "Iron",          2, "55.845"),  // 26
    ("Co", "Cobalt",        2, "58.933"),  // 27
    ("Ni", "Nickel",        2, "58.693"),  // 28
    ("Cu", "Copper",        2, "63.546"),  // 29
    ("Zn", "Zinc",          2, "65.38"),   // 30
    ("Ga", "Gallium",       3, "69.723"),  // 31
    ("Ge", "Germanium",     4, "72.630"),  // 32
    ("As", "Arsenic",       4, "74.922"),  // 33
    ("Se", "Selenium",      5, "78.971"),  // 34
    ("Br", "Bromine",       6, "79.904"),  // 35
    ("Kr", "Krypton",       7, "83.798"),  // 36
    ("Rb", "Rubidium",      0, "85.468"),  // 37
    ("Sr", "Strontium",     1, "87.62"),   // 38
    ("Y",  "Yttrium",       2, "88.906"),  // 39
    ("Zr", "Zirconium",     2, "91.224"),  // 40
    ("Nb", "Niobium",       2, "92.906"),  // 41
    ("Mo", "Molybdenum",    2, "95.96"),   // 42
    ("Tc", "Technetium",    2, "(98)"),    // 43
    ("Ru", "Ruthenium",     2, "101.07"),  // 44
    ("Rh", "Rhodium",       2, "102.906"), // 45
    ("Pd", "Palladium",     2, "106.42"),  // 46
    ("Ag", "Silver",        2, "107.868"), // 47
    ("Cd", "Cadmium",       2, "112.414"), // 48
    ("In", "Indium",        3, "114.818"), // 49
    ("Sn", "Tin",           3, "118.710"), // 50
    ("Sb", "Antimony",      4, "121.760"), // 51
    ("Te", "Tellurium",     4, "127.60"),  // 52
    ("I",  "Iodine",        6, "126.904"), // 53
    ("Xe", "Xenon",         7, "131.293"), // 54
    ("Cs", "Caesium",       0, "132.905"), // 55
    ("Ba", "Barium",        1, "137.327"), // 56
    ("La", "Lanthanum",     8, "138.905"), // 57
    ("Ce", "Cerium",        8, "140.116"), // 58
    ("Pr", "Praseodymium",  8, "140.908"), // 59
    ("Nd", "Neodymium",     8, "144.242"), // 60
    ("Pm", "Promethium",    8, "(145)"),   // 61
    ("Sm", "Samarium",      8, "150.36"),  // 62
    ("Eu", "Europium",      8, "151.964"), // 63
    ("Gd", "Gadolinium",    8, "157.25"),  // 64
    ("Tb", "Terbium",       8, "158.925"), // 65
    ("Dy", "Dysprosium",    8, "162.500"), // 66
    ("Ho", "Holmium",       8, "164.930"), // 67
    ("Er", "Erbium",        8, "167.259"), // 68
    ("Tm", "Thulium",       8, "168.934"), // 69
    ("Yb", "Ytterbium",     8, "173.054"), // 70
    ("Lu", "Lutetium",      8, "174.967"), // 71
    ("Hf", "Hafnium",       2, "178.49"),  // 72
    ("Ta", "Tantalum",      2, "180.948"), // 73
    ("W",  "Tungsten",      2, "183.84"),  // 74
    ("Re", "Rhenium",       2, "186.207"), // 75
    ("Os", "Osmium",        2, "190.23"),  // 76
    ("Ir", "Iridium",       2, "192.217"), // 77
    ("Pt", "Platinum",      2, "195.084"), // 78
    ("Au", "Gold",          2, "196.967"), // 79
    ("Hg", "Mercury",       2, "200.592"), // 80
    ("Tl", "Thallium",      3, "204.38"),  // 81
    ("Pb", "Lead",          3, "207.2"),   // 82
    ("Bi", "Bismuth",       3, "208.980"), // 83
    ("Po", "Polonium",      3, "(209)"),   // 84
    ("At", "Astatine",      4, "(210)"),   // 85
    ("Rn", "Radon",         7, "(222)"),   // 86
    ("Fr", "Francium",      0, "(223)"),   // 87
    ("Ra", "Radium",        1, "(226)"),   // 88
    ("Ac", "Actinium",      9, "(227)"),   // 89
    ("Th", "Thorium",       9, "232.038"), // 90
    ("Pa", "Protactinium",  9, "231.036"), // 91
    ("U",  "Uranium",       9, "238.029"), // 92
    ("Np", "Neptunium",     9, "(237)"),   // 93
    ("Pu", "Plutonium",     9, "(244)"),   // 94
    ("Am", "Americium",     9, "(243)"),   // 95
    ("Cm", "Curium",        9, "(247)"),   // 96
    ("Bk", "Berkelium",     9, "(247)"),   // 97
    ("Cf", "Californium",   9, "(251)"),   // 98
    ("Es", "Einsteinium",   9, "(252)"),   // 99
    ("Fm", "Fermium",       9, "(257)"),   // 100
    ("Md", "Mendelevium",   9, "(258)"),   // 101
    ("No", "Nobelium",      9, "(259)"),   // 102
    ("Lr", "Lawrencium",    9, "(262)"),   // 103
    ("Rf", "Rutherfordium", 2, "(267)"),   // 104
    ("Db", "Dubnium",       2, "(270)"),   // 105
    ("Sg", "Seaborgium",    2, "(271)"),   // 106
    ("Bh", "Bohrium",       2, "(270)"),   // 107
    ("Hs", "Hassium",       2, "(277)"),   // 108
    ("Mt", "Meitnerium",   10, "(278)"),   // 109
    ("Ds", "Darmstadtium", 10, "(281)"),   // 110
    ("Rg", "Roentgenium",  10, "(282)"),   // 111
    ("Cn", "Copernicium",   2, "(285)"),   // 112
    ("Nh", "Nihonium",      3, "(286)"),   // 113
    ("Fl", "Flerovium",     3, "(289)"),   // 114
    ("Mc", "Moscovium",    10, "(290)"),   // 115
    ("Lv", "Livermorium",  10, "(293)"),   // 116
    ("Ts", "Tennessine",   10, "(294)"),   // 117
    ("Og", "Oganesson",     7, "(294)"),   // 118
];

// Grid[row][col] = atomic number (1-118), 0 = empty
const GRID: [[u8; 18]; 10] = [
    [ 1,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  2],
    [ 3,  4,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  5,  6,  7,  8,  9, 10],
    [11, 12,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0, 13, 14, 15, 16, 17, 18],
    [19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36],
    [37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52, 53, 54],
    [55, 56,  0, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84, 85, 86],
    [87, 88,  0,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118],
    [ 0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0,  0],
    [ 0,  0, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68, 69, 70, 71,  0],
    [ 0,  0, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99,100,101,102,103,  0],
];

const CELL_W: i32 = 54;
const CELL_H: i32 = 38;
const STRIDE_X: i32 = 56;
const STRIDE_Y: i32 = 40;
const GRID_X: i32 = 46;  // (1100 - 18*56) / 2
const GRID_Y: i32 = 36;

fn cell_xy(row: usize, col: usize) -> (i32, i32) {
    let row_offset = if row >= 8 { 6 } else { 0 }; // blank row adds 6px gap
    let y = GRID_Y + row as i32 * STRIDE_Y + row_offset;
    (GRID_X + col as i32 * STRIDE_X, y)
}

struct App { sel_r: usize, sel_c: usize }

impl App {
    fn new() -> Self { App { sel_r: 0, sel_c: 0 } }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Periodic Table of the Elements");
        text(500, 8, C_HINT, "←→↑↓:select  Q:quit");

        // Draw all cells
        for r in 0..10usize {
            if r == 7 { continue; } // blank separator row
            for c in 0..18usize {
                let num = GRID[r][c] as usize;
                let (cx, cy) = cell_xy(r, c);
                let is_sel = r == self.sel_r && c == self.sel_c;

                if num == 0 {
                    // placeholder for lanthanide/actinide series
                    if (r == 5 || r == 6) && c == 2 {
                        fill(cx, cy, CELL_W, CELL_H, C_CARD);
                        border(cx, cy, CELL_W, CELL_H, C_BORDER);
                        let lbl = if r == 5 { "57-71" } else { "89-103" };
                        text(cx + 4, cy + 2, C_HINT, lbl);
                    }
                    continue;
                }

                let (sym, _, cat, _) = ELEMENTS[num - 1];
                let cat_color = CATS[cat as usize].1;

                if is_sel {
                    fill(cx, cy, CELL_W, CELL_H, cat_color & 0xFFFFFF44 | 0x00000044);
                    // approximate: just use a dark tinted bg
                    fill(cx, cy, CELL_W, CELL_H, 0x1E2A3AFF);
                    border(cx, cy, CELL_W, CELL_H, cat_color);
                } else {
                    fill(cx, cy, CELL_W, CELL_H, C_CARD);
                    border(cx, cy, CELL_W, CELL_H, C_BORDER);
                }

                let num_s = format!("{}", num);
                text(cx + 2, cy + 2, C_HINT, &num_s);
                let sym_x = cx + (CELL_W - sym.len() as i32 * 8) / 2;
                text(sym_x, cy + 18, if is_sel { C_TEXT } else { cat_color }, sym);
            }
        }

        // Row labels: "57-71" and "89-103" pointers
        let (lx5, ly5) = cell_xy(5, 2);
        let (lx6, ly6) = cell_xy(6, 2);
        let (lx8, ly8) = cell_xy(8, 2);
        let (lx9, ly9) = cell_xy(9, 2);
        fill(lx5 + 2, ly5 + CELL_H / 2, lx8 - lx5 + STRIDE_X, 1, C_BORDER);
        fill(lx6 + 2, ly6 + CELL_H / 2, lx9 - lx6 + STRIDE_X, 1, C_BORDER);
        text(lx8 - 48, ly8 + 12, C_HINT, "La");
        text(lx9 - 48, ly9 + 12, C_HINT, "Ac");

        // Info panel for selected element
        let num = GRID[self.sel_r][self.sel_c] as usize;
        if num > 0 {
            let (sym, name, cat, mass) = ELEMENTS[num - 1];
            let cat_color = CATS[cat as usize].1;
            let cat_name  = CATS[cat as usize].0;

            let panel_y = GRID_Y + 8 * STRIDE_Y + 14; // below actinides
            fill(GRID_X, panel_y, W - GRID_X * 2, H - panel_y - 26, C_CARD);
            border(GRID_X, panel_y, W - GRID_X * 2, H - panel_y - 26, C_BORDER);

            // Large symbol
            let big_x = GRID_X + 12;
            text(big_x, panel_y + 6, cat_color, sym);
            text(big_x, panel_y + 24, C_TEXT, name);
            text(big_x, panel_y + 42, C_HINT, &format!("Z = {}   Mass = {} u", num, mass));

            // Category
            fill(big_x + 260, panel_y + 8, 14, 14, cat_color);
            text(big_x + 280, panel_y + 8, cat_color, cat_name);

            // Period and group
            let period = if self.sel_r <= 6 { self.sel_r + 1 } else if self.sel_r == 8 { 6 } else { 7 };
            let group  = self.sel_c + 1;
            text(big_x + 260, panel_y + 28, C_HINT,
                &format!("Period {}  Group {}  (f-block: {})", period, group, self.sel_r >= 8));
        }

        // Category legend at bottom
        let leg_y = H - 48;
        for (i, &(name, color)) in CATS.iter().enumerate() {
            let lx = GRID_X + i as i32 * 96;
            fill(lx, leg_y, 12, 12, color);
            text(lx + 14, leg_y, C_HINT, name);
        }

        fill(0, H - 24, W, 24, C_HEADER);
        let num = GRID[self.sel_r][self.sel_c] as usize;
        if num > 0 {
            let (sym, name, _, _) = ELEMENTS[num - 1];
            text(12, H - 18, C_HINT, &format!("{} — {}  |  Z={}  |  118 elements", sym, name, num));
        }
        flush();
    }

    fn find_right(&self) -> (usize, usize) {
        for dc in 1..18 {
            let c = (self.sel_c + dc) % 18;
            if GRID[self.sel_r][c] > 0 { return (self.sel_r, c); }
        }
        (self.sel_r, self.sel_c)
    }

    fn find_left(&self) -> (usize, usize) {
        for dc in 1..18 {
            let c = (self.sel_c + 18 - dc) % 18;
            if GRID[self.sel_r][c] > 0 { return (self.sel_r, c); }
        }
        (self.sel_r, self.sel_c)
    }

    fn find_down(&self) -> (usize, usize) {
        for dr in 1..10 {
            let r = (self.sel_r + dr) % 10;
            if r == 7 { continue; }
            if GRID[r][self.sel_c] > 0 { return (r, self.sel_c); }
            for d in 1..=6usize {
                if self.sel_c + d < 18 && GRID[r][self.sel_c + d] > 0 {
                    return (r, self.sel_c + d);
                }
                if self.sel_c >= d && GRID[r][self.sel_c - d] > 0 {
                    return (r, self.sel_c - d);
                }
            }
        }
        (self.sel_r, self.sel_c)
    }

    fn find_up(&self) -> (usize, usize) {
        for dr in 1..10 {
            let r = (self.sel_r + 10 - dr) % 10;
            if r == 7 { continue; }
            if GRID[r][self.sel_c] > 0 { return (r, self.sel_c); }
            for d in 1..=6usize {
                if self.sel_c + d < 18 && GRID[r][self.sel_c + d] > 0 {
                    return (r, self.sel_c + d);
                }
                if self.sel_c >= d && GRID[r][self.sel_c - d] > 0 {
                    return (r, self.sel_c - d);
                }
            }
        }
        (self.sel_r, self.sel_c)
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[C" => { let (r,c) = self.find_right(); self.sel_r=r; self.sel_c=c; }
            "\x1b[D" => { let (r,c) = self.find_left();  self.sel_r=r; self.sel_c=c; }
            "\x1b[B" => { let (r,c) = self.find_down();  self.sel_r=r; self.sel_c=c; }
            "\x1b[A" => { let (r,c) = self.find_up();    self.sel_r=r; self.sel_c=c; }
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
