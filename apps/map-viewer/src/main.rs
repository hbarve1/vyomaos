// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 800;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;

const MAP_W: usize = 80;
const MAP_H: usize = 40;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_SEL: u32     = 0x58A6FFFF;

// Terrain colors
const COL_OCEAN:    u32 = 0x0D2B4AFF;
const COL_MOUNTAIN: u32 = 0x7A8490FF;
const COL_HILL:     u32 = 0x5A7040FF;
const COL_DESERT:   u32 = 0xC4A96AFF;
const COL_FOREST:   u32 = 0x1A5A1AFF;
const COL_PLAINS:   u32 = 0x3A7A28FF;
const COL_CITY:     u32 = 0xFF3B30FF;

fn fill(x: u32, y: u32, w: u32, h: u32, rgba: u32) {
    println!("VYOMA_DRAW:fill_rect:{x},{y},{w},{h},{rgba:#010x}");
}
fn text(x: u32, y: u32, rgba: u32, s: &str) {
    println!("VYOMA_DRAW:draw_text:{x},{y},{rgba:#010x},m,{s}");
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

const LANDMARKS: &[(usize, usize, &str)] = &[
    (10, 18, "New York"),
    (8,  45, "London"),
    (9,  46, "Paris"),
    (7,  57, "Moscow"),
    (11, 64, "Beijing"),
    (12, 70, "Tokyo"),
    (16, 48, "Cairo"),
    (19, 59, "Mumbai"),
    (30, 70, "Sydney"),
    (26, 17, "Sao Paulo"),
    (22, 46, "Lagos"),
    (18, 56, "Dubai"),
];

fn terrain_color(ch: char) -> u32 {
    match ch {
        '.' => COL_OCEAN,
        '#' => COL_MOUNTAIN,
        '^' => COL_HILL,
        '~' => COL_DESERT,
        '*' => COL_FOREST,
        ',' => COL_PLAINS,
        'C' => COL_CITY,
        _   => C_BG,
    }
}

fn make_map() -> Vec<Vec<char>> {
    let mut map: Vec<Vec<char>> = (0..MAP_H).map(|_| vec!['.'; MAP_W]).collect();

    // Western continent (Americas-like): cols 8-28, rows 5-34
    for r in 5..35usize {
        let (cs, ce) = match r {
            5..=9   => (12, 22),
            10..=19 => (8,  25),
            20..=28 => (9,  23),
            29..=34 => (11, 20),
            _       => continue,
        };
        for c in cs..ce {
            let edge = (c - cs).min(ce - 1 - c).min(r - 5).min(34 - r);
            map[r][c] = if edge == 0 { '#' }
                else if edge <= 1 { '^' }
                else if r >= 22 && r < 32 && c >= 15 { '*' }
                else if r >= 16 && r < 24 && c >= 13 && c < 20 { '~' }
                else { ',' };
        }
    }

    // Eastern continent (Eurasia): cols 36-72, rows 4-24
    for r in 4..24usize {
        let (cs, ce) = match r {
            4..=7   => (42, 68),
            8..=14  => (36, 72),
            15..=20 => (38, 70),
            21..=23 => (40, 65),
            _       => continue,
        };
        for c in cs..ce {
            let edge = (c - cs).min(ce - 1 - c).min(r - 4).min(23 - r);
            map[r][c] = if edge == 0 { '#' }
                else if edge <= 1 { '^' }
                else if c >= 52 && c < 62 && r >= 12 { '~' }
                else if c >= 60 && r >= 9 { '#' }
                else if c >= 64 { '*' }
                else { ',' };
        }
    }

    // Africa: cols 42-58, rows 18-35
    for r in 18..35usize {
        let w = if r < 28 { 16usize } else { 16 - (r - 27) * 2 };
        let cs = 42usize;
        let ce = cs + w;
        for c in cs..ce.min(MAP_W) {
            let edge = (c - cs).min(ce - 1 - c).min(r - 18).min(34 - r);
            map[r][c] = if edge == 0 { '#' }
                else if edge <= 1 { '^' }
                else if r >= 20 && r < 27 { '~' }
                else { ',' };
        }
    }

    // Australia: cols 62-74, rows 27-34
    for r in 27..34usize {
        for c in 62..74usize {
            let edge = (c - 62).min(73 - c).min(r - 27).min(33 - r);
            map[r][c] = if edge == 0 { '#' } else if edge <= 1 { '^' } else { '~' };
        }
    }

    // Place cities at landmark positions
    for &(lr, lc, _) in LANDMARKS {
        if lr < MAP_H && lc < MAP_W { map[lr][lc] = 'C'; }
    }

    map
}

fn lon_label(c: i32) -> i32 { c * 360 / MAP_W as i32 - 180 }
fn lat_label(r: i32) -> i32 { 90 - r * 180 / MAP_H as i32 }

const CELL_SIZES: [u32; 4] = [8, 12, 16, 20];

struct Viewer {
    map:      Vec<Vec<char>>,
    view_x:   i32,
    view_y:   i32,
    zoom_idx: usize,
}

impl Viewer {
    fn new() -> Self {
        Viewer {
            map: make_map(),
            view_x: 0,
            view_y: 0,
            zoom_idx: 1, // 12px default
        }
    }

    fn cell(&self) -> u32 { CELL_SIZES[self.zoom_idx] }

    fn visible_cols(&self) -> i32 { (W / self.cell()) as i32 }
    fn visible_rows(&self) -> i32 { ((H - HEADER_H - STATUS_H) / self.cell()) as i32 }

    fn clamp_view(&mut self) {
        let vc = self.visible_cols();
        let vr = self.visible_rows();
        self.view_x = self.view_x.clamp(0, (MAP_W as i32 - vc).max(0));
        self.view_y = self.view_y.clamp(0, (MAP_H as i32 - vr).max(0));
    }

    fn pan(&mut self, dx: i32, dy: i32) {
        self.view_x += dx;
        self.view_y += dy;
        self.clamp_view();
    }

    fn zoom_in(&mut self) {
        if self.zoom_idx + 1 < CELL_SIZES.len() { self.zoom_idx += 1; }
        self.clamp_view();
    }

    fn zoom_out(&mut self) {
        if self.zoom_idx > 0 { self.zoom_idx -= 1; }
        self.clamp_view();
    }

    fn reset(&mut self) {
        self.view_x = 0; self.view_y = 0; self.zoom_idx = 1;
    }
}

fn draw(v: &Viewer) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Map Viewer");
    text(160, 16, C_HINT, &format!("Zoom: {}px/cell", v.cell()));
    text(360, 16, C_HINT, &format!("Origin: ({},{})", lon_label(v.view_x), lat_label(v.view_y)));
    text(580, 16, C_HINT, "Arrows:pan  +/-:zoom  R:reset");

    let cell = v.cell();
    let vc = v.visible_cols();
    let vr = v.visible_rows();

    for dr in 0..vr {
        for dc in 0..vc {
            let mr = v.view_y + dr;
            let mc = v.view_x + dc;
            if mr < 0 || mr >= MAP_H as i32 || mc < 0 || mc >= MAP_W as i32 { continue; }
            let ch = v.map[mr as usize][mc as usize];
            let color = terrain_color(ch);
            let sx = dc as u32 * cell;
            let sy = HEADER_H + dr as u32 * cell;
            fill(sx, sy, cell, cell, color);
        }
    }

    // Landmark labels (when cell is large enough to show text)
    if cell >= 12 {
        for &(lr, lc, name) in LANDMARKS {
            let dr = lr as i32 - v.view_y;
            let dc = lc as i32 - v.view_x;
            if dr >= 0 && dr < vr && dc >= 0 && dc < vc {
                let lx = dc as u32 * cell;
                let ly = HEADER_H + dr as u32 * cell;
                // City dot already colored; draw label above
                if ly >= HEADER_H + 16 {
                    text(lx + 4, ly.saturating_sub(16), C_TEXT, name);
                }
            }
        }
    }

    // Grid lines at zoom ≥ 16
    if cell >= 16 {
        for dc in 0..=vc {
            let sx = dc as u32 * cell;
            fill(sx, HEADER_H, 1, vr as u32 * cell, 0x1C2128FF);
        }
        for dr in 0..=vr {
            let sy = HEADER_H + dr as u32 * cell;
            fill(0, sy, vc as u32 * cell, 1, 0x1C2128FF);
        }
    }

    // Legend
    let lx = vc as u32 * cell + 4;
    if lx + 4 < W {
        let ly = HEADER_H;
        text(lx, ly, C_HINT, "Terrain:");
        let legend = [('.', COL_OCEAN, "Ocean"), (',', COL_PLAINS, "Plains"),
                      ('*', COL_FOREST, "Forest"), ('~', COL_DESERT, "Desert"),
                      ('^', COL_HILL, "Hill"), ('#', COL_MOUNTAIN, "Mountain"),
                      ('C', COL_CITY, "City")];
        for (i, &(_, color, label)) in legend.iter().enumerate() {
            let ey = ly + 20 + i as u32 * 22;
            fill(lx, ey, 14, 14, color);
            text(lx + 18, ey, C_HINT, label);
        }
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let lon = lon_label(v.view_x);
    let lat = lat_label(v.view_y);
    let lon_dir = if lon >= 0 { "E" } else { "W" };
    let lat_dir = if lat >= 0 { "N" } else { "S" };
    text(8, sb_y + 6, C_HINT, &format!("{}°{} {}°{}  |  Map: {}×{} cells  |  View: {}×{} cells",
        lon.abs(), lon_dir, lat.abs(), lat_dir,
        MAP_W, MAP_H, vc, vr));

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut viewer = Viewer::new();

    println!("@supervisor: raise map-viewer");
    let _ = io::stdout().flush();
    draw(&viewer);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x1b" | "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => { viewer.pan(0, -2); }
            "\x1b[B" => { viewer.pan(0, 2); }
            "\x1b[C" => { viewer.pan(2, 0); }
            "\x1b[D" => { viewer.pan(-2, 0); }
            "+" | "=" => { viewer.zoom_in(); }
            "-" => { viewer.zoom_out(); }
            "r" | "R" => { viewer.reset(); }
            _ => {}
        }
        draw(&viewer);
    }
}
