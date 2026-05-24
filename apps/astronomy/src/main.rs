// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 800;
const HEADER_H: u32 = 36;
const STATUS_H: u32 = 24;
const INFO_W: u32 = 220;
const SKY_W: u32 = W - INFO_W;
const SKY_H: u32 = H - HEADER_H - STATUS_H;
const SKY_X: u32 = 0;
const SKY_Y: u32 = HEADER_H;

const C_BG: u32      = 0x0D1117FF;
const C_SKY: u32     = 0x020408FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_YELLOW: u32  = 0xD29922FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_PURPLE: u32  = 0xBC8CFFFF;
const C_CARD: u32    = 0x161B22FF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

struct Star {
    // Fractional position on sky sphere [0.0, 1.0)
    ra:   f32, // right ascension mapped to [0,1)
    dec:  f32, // declination mapped to [0,1)
    mag:  u32, // magnitude 1..5 (1=brightest)
    col:  u32, // RGBA
    name: &'static str,
}

const STAR_NAMES: &[&str] = &[
    "Sirius", "Canopus", "Arcturus", "Rigel Kentaurus", "Vega",
    "Capella", "Rigel", "Procyon", "Achernar", "Betelgeuse",
    "Hadar", "Altair", "Acrux", "Aldebaran", "Antares",
    "Spica", "Pollux", "Fomalhaut", "Mimosa", "Deneb",
    "Regulus", "Adhara", "Castor", "Gamma Crucis", "Shaula",
    "Bellatrix", "El Nath", "Miaplacidus", "Alnilam", "Alnitak",
    "Alioth", "Dubhe", "Mirfak", "Wezen", "Kaus Australis",
    "Avior", "Alkaid", "Sargas", "Menkent", "Atria",
];

// Temperature → color
const HOT_COLORS: [u32; 4] = [0xAABBFFFF, 0xCCDDFFFF, 0xEEEEFFFF, 0xFFFFFFFF];
const MED_COLORS: [u32; 4] = [0xFFFFAAFF, 0xFFEE88FF, 0xFFDD66FF, 0xFFCC44FF];
const COOL_COLORS: [u32; 4] = [0xFFAA44FF, 0xFF8833FF, 0xFF6622FF, 0xFF4411FF];

fn gen_stars() -> Vec<Star> {
    let mut stars = Vec::with_capacity(200);
    let mut s = 0xA0C5E_12345678ABu64;
    for i in 0..200usize {
        s = lcg(s);
        let ra = (s & 0xFFFF) as f32 / 65536.0;
        s = lcg(s);
        let dec = (s & 0xFFFF) as f32 / 65536.0;
        s = lcg(s);
        let mag = 1 + (s % 5) as u32;
        s = lcg(s);
        let temp_class = s % 3; // 0=hot, 1=med, 2=cool
        s = lcg(s);
        let ci = (s % 4) as usize;
        let col = match temp_class {
            0 => HOT_COLORS[ci],
            1 => MED_COLORS[ci],
            _ => COOL_COLORS[ci],
        };
        let name = if i < STAR_NAMES.len() { STAR_NAMES[i] } else { "HD-*" };
        stars.push(Star { ra, dec, mag, col, name });
    }
    stars
}

struct Planet {
    name:  &'static str,
    col:   u32,
    size:  u32,
    orbit: f32, // base orbital position [0,1)
    speed: f32, // fractional speed
}

const PLANETS: &[Planet] = &[
    Planet { name: "Mercury", col: 0xAAAAAAFF, size: 4, orbit: 0.12, speed: 0.04 },
    Planet { name: "Venus",   col: 0xFFDDAEFF, size: 6, orbit: 0.35, speed: 0.016 },
    Planet { name: "Mars",    col: 0xFF6644FF, size: 5, orbit: 0.58, speed: 0.009 },
    Planet { name: "Jupiter", col: 0xDDBB88FF, size: 10, orbit: 0.74, speed: 0.001 },
    Planet { name: "Saturn",  col: 0xEECC99FF, size: 9, orbit: 0.90, speed: 0.0004 },
];

// Constellation data: pairs of star indices (in the first 40 named stars)
const CONSTELLATIONS: &[(&str, &[(usize, usize)])] = &[
    ("Orion",     &[(6, 9), (9, 28), (28, 29), (29, 25), (25, 6), (28, 19)]),
    ("Ursa Major",&[(31, 36), (36, 38), (38, 30), (30, 31), (30, 33)]),
    ("Scorpius",  &[(14, 27), (27, 17), (17, 23), (23, 24)]),
    ("Lyra",      &[(4, 19), (19, 20)]),
    ("Aquila",    &[(11, 21), (21, 31)]),
    ("Crux",      &[(12, 22), (12, 23)]),
    ("Taurus",    &[(13, 18), (18, 28)]),
    ("Gemini",    &[(21, 2), (2, 7)]),
];

fn sky_to_screen(ra: f32, dec: f32, view_ra: f32, view_dec: f32, zoom: f32) -> Option<(u32, u32)> {
    let dx = (ra - view_ra) * zoom * SKY_W as f32;
    let dy = (dec - view_dec) * zoom * SKY_H as f32;
    let sx = SKY_W as f32 / 2.0 + dx;
    let sy = SKY_H as f32 / 2.0 + dy;
    if sx >= 0.0 && sx < SKY_W as f32 && sy >= 0.0 && sy < SKY_H as f32 {
        Some((SKY_X + sx as u32, SKY_Y + sy as u32))
    } else {
        None
    }
}

fn draw_line_approx(x0: u32, y0: u32, x1: u32, y1: u32, col: u32) {
    // Bresenham-lite: draw dots along the line
    let steps = ((x1 as i32 - x0 as i32).abs().max((y1 as i32 - y0 as i32).abs())) as u32;
    if steps == 0 { return; }
    let steps = steps.min(200);
    for i in 0..=steps {
        let t = i as f32 / steps as f32;
        let x = (x0 as f32 + t * (x1 as f32 - x0 as f32)) as u32;
        let y = (y0 as f32 + t * (y1 as f32 - y0 as f32)) as u32;
        fill(x, y, 1, 1, col);
    }
}

struct App {
    stars:       Vec<Star>,
    view_ra:     f32,
    view_dec:    f32,
    zoom:        f32,
    show_consts: bool,
    show_planets: bool,
    tick:        u64,
    selected:    Option<usize>, // index into stars or 200+idx for planets
    status:      String,
}

impl App {
    fn new() -> Self {
        App {
            stars: gen_stars(),
            view_ra: 0.5,
            view_dec: 0.5,
            zoom: 1.0,
            show_consts: true,
            show_planets: true,
            tick: 0,
            selected: None,
            status: String::from("←→↑↓=pan  +/-=zoom  C=constellations  P=planets  Ctrl+C=quit"),
        }
    }

    fn planet_pos(&self, p: &Planet) -> (f32, f32) {
        let t = self.tick as f32 * p.speed;
        let ra = (p.orbit + t).fract();
        let angle = p.orbit * std::f32::consts::PI * 2.0;
        let dec = 0.5 + 0.15 * angle.sin();
        (ra, dec)
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Sky background
    fill(SKY_X, SKY_Y, SKY_W, SKY_H, C_SKY);

    // Constellation lines
    if app.show_consts {
        for (_, pairs) in CONSTELLATIONS {
            for (a, b) in *pairs {
                if *a >= app.stars.len() || *b >= app.stars.len() { continue; }
                let sa = &app.stars[*a];
                let sb = &app.stars[*b];
                if let (Some((ax, ay)), Some((bx, by))) = (
                    sky_to_screen(sa.ra, sa.dec, app.view_ra, app.view_dec, app.zoom),
                    sky_to_screen(sb.ra, sb.dec, app.view_ra, app.view_dec, app.zoom),
                ) {
                    draw_line_approx(ax, ay, bx, by, 0x1E2D45FF);
                }
            }
        }
    }

    // Stars
    for (i, star) in app.stars.iter().enumerate() {
        if let Some((sx, sy)) = sky_to_screen(star.ra, star.dec, app.view_ra, app.view_dec, app.zoom) {
            let size = (6u32).saturating_sub(star.mag).max(1);
            let is_sel = app.selected == Some(i);
            if is_sel {
                fill(sx.saturating_sub(size + 2), sy.saturating_sub(size + 2), (size + 2) * 2, (size + 2) * 2, 0x1C2D4EFF);
            }
            fill(sx.saturating_sub(size / 2), sy.saturating_sub(size / 2), size, size, star.col);
            // Label for mag 1-2 stars (brightest)
            if star.mag <= 2 && app.zoom >= 1.0 {
                text(sx + size + 2, sy.saturating_sub(4), C_HINT, star.name);
            }
        }
    }

    // Planets
    if app.show_planets {
        for (i, planet) in PLANETS.iter().enumerate() {
            let (pra, pdec) = app.planet_pos(planet);
            if let Some((px, py)) = sky_to_screen(pra, pdec, app.view_ra, app.view_dec, app.zoom) {
                let is_sel = app.selected == Some(200 + i);
                if is_sel {
                    border(px.saturating_sub(planet.size + 3), py.saturating_sub(planet.size + 3),
                           (planet.size + 3) * 2, (planet.size + 3) * 2, C_SEL);
                }
                fill(px.saturating_sub(planet.size / 2), py.saturating_sub(planet.size / 2),
                     planet.size, planet.size, planet.col);
                text(px + planet.size + 2, py.saturating_sub(4), C_YELLOW, planet.name);
            }
        }
    }

    // Sky border
    border(SKY_X, SKY_Y, SKY_W, SKY_H, C_BORDER);

    // Info panel
    let info_x = SKY_W;
    fill(info_x, SKY_Y, INFO_W, SKY_H, C_CARD);
    border(info_x, SKY_Y, INFO_W, SKY_H, C_BORDER);
    text(info_x + 8, SKY_Y + 6, C_HINT, "Sky Map");
    fill(info_x, SKY_Y + 22, INFO_W, 1, C_BORDER);

    text(info_x + 8, SKY_Y + 28, C_HINT, "View");
    let ra_deg = (app.view_ra * 360.0) as u32;
    let dec_deg = ((app.view_dec - 0.5) * 180.0) as i32;
    text(info_x + 8, SKY_Y + 44, C_TEXT, &format!("RA:  {}°", ra_deg));
    text(info_x + 8, SKY_Y + 60, C_TEXT, &format!("Dec: {}°", dec_deg));
    text(info_x + 8, SKY_Y + 76, C_TEXT, &format!("Zoom: {:.1}x", app.zoom));

    fill(info_x, SKY_Y + 94, INFO_W, 1, C_BORDER);
    text(info_x + 8, SKY_Y + 100, C_HINT, "Visibility");
    let const_label = if app.show_consts { "ON " } else { "OFF" };
    let planet_label = if app.show_planets { "ON " } else { "OFF" };
    text(info_x + 8, SKY_Y + 116, C_TEXT, &format!("Constellations: {}", const_label));
    text(info_x + 8, SKY_Y + 132, C_TEXT, &format!("Planets:        {}", planet_label));

    fill(info_x, SKY_Y + 150, INFO_W, 1, C_BORDER);
    text(info_x + 8, SKY_Y + 156, C_HINT, "Selected");
    if let Some(idx) = app.selected {
        if idx < app.stars.len() {
            let s = &app.stars[idx];
            text(info_x + 8, SKY_Y + 172, C_SEL, s.name);
            text(info_x + 8, SKY_Y + 188, C_TEXT, &format!("Mag: {}", s.mag));
            let temp = if s.col & 0xFF00 > s.col & 0xFF0000 { "Hot (blue)" }
                else if (s.col >> 8) & 0xFF > (s.col >> 24) & 0xFF { "Cool (red)" }
                else { "Medium (yellow)" };
            text(info_x + 8, SKY_Y + 204, C_TEXT, temp);
            let ra_s = (s.ra * 360.0) as u32;
            let dec_s = ((s.dec - 0.5) * 180.0) as i32;
            text(info_x + 8, SKY_Y + 220, C_HINT, &format!("RA {}° Dec {}°", ra_s, dec_s));
        } else if idx >= 200 {
            let p = &PLANETS[idx - 200];
            text(info_x + 8, SKY_Y + 172, C_YELLOW, p.name);
            text(info_x + 8, SKY_Y + 188, C_TEXT, "Planet");
            text(info_x + 8, SKY_Y + 204, C_TEXT, &format!("Size: {}", p.size));
        }
    } else {
        text(info_x + 8, SKY_Y + 172, C_HINT, "—");
        text(info_x + 8, SKY_Y + 188, C_HINT, "Navigate to");
        text(info_x + 8, SKY_Y + 204, C_HINT, "select objects");
    }

    // Legend
    fill(info_x, SKY_Y + 250, INFO_W, 1, C_BORDER);
    text(info_x + 8, SKY_Y + 256, C_HINT, "Star types");
    fill(info_x + 8, SKY_Y + 274, 8, 8, HOT_COLORS[2]);
    text(info_x + 20, SKY_Y + 272, C_TEXT, "Hot (blue/white)");
    fill(info_x + 8, SKY_Y + 290, 8, 8, MED_COLORS[0]);
    text(info_x + 20, SKY_Y + 288, C_TEXT, "Medium (yellow)");
    fill(info_x + 8, SKY_Y + 306, 8, 8, COOL_COLORS[1]);
    text(info_x + 20, SKY_Y + 304, C_TEXT, "Cool (red/orange)");

    text(info_x + 8, SKY_Y + 330, C_HINT, "Constellations:");
    for (ci, (name, _)) in CONSTELLATIONS.iter().enumerate() {
        let cy = SKY_Y + 346 + ci as u32 * 16;
        if cy + 16 > SKY_Y + SKY_H { break; }
        text(info_x + 8, cy, C_HINT, name);
    }

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 10, C_ORANGE, "Astronomy Viewer");
    text(200, 10, C_HINT, &format!("200 stars · 5 planets · 8 constellations · zoom {:.1}x", app.zoom));

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn find_nearest(app: &App) -> Option<usize> {
    // Find the star/planet closest to the view center
    let mut best_dist = f32::MAX;
    let mut best_idx = None;
    for (i, star) in app.stars.iter().enumerate() {
        let dx = star.ra - app.view_ra;
        let dy = star.dec - app.view_dec;
        let d = dx * dx + dy * dy;
        if d < best_dist {
            best_dist = d;
            best_idx = Some(i);
        }
    }
    if app.show_planets {
        for (i, planet) in PLANETS.iter().enumerate() {
            let (pra, pdec) = app.planet_pos(planet);
            let dx = pra - app.view_ra;
            let dy = pdec - app.view_dec;
            let d = dx * dx + dy * dy;
            if d < best_dist {
                best_dist = d;
                best_idx = Some(200 + i);
            }
        }
    }
    best_idx
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise astronomy");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        let pan = 0.05 / app.zoom;

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                app.view_dec = (app.view_dec - pan).max(0.0);
                app.selected = find_nearest(&app);
                app.status = format!("RA {:.2}  Dec {:.2}", app.view_ra, app.view_dec);
            }
            "\x1b[B" => {
                app.view_dec = (app.view_dec + pan).min(1.0);
                app.selected = find_nearest(&app);
                app.status = format!("RA {:.2}  Dec {:.2}", app.view_ra, app.view_dec);
            }
            "\x1b[D" => {
                app.view_ra = (app.view_ra - pan).rem_euclid(1.0);
                app.selected = find_nearest(&app);
                app.status = format!("RA {:.2}  Dec {:.2}", app.view_ra, app.view_dec);
            }
            "\x1b[C" => {
                app.view_ra = (app.view_ra + pan).rem_euclid(1.0);
                app.selected = find_nearest(&app);
                app.status = format!("RA {:.2}  Dec {:.2}", app.view_ra, app.view_dec);
            }
            "+" | "=" => {
                app.zoom = (app.zoom * 1.5).min(8.0);
                app.status = format!("Zoom: {:.1}x", app.zoom);
            }
            "-" | "_" => {
                app.zoom = (app.zoom / 1.5).max(0.5);
                app.status = format!("Zoom: {:.1}x", app.zoom);
            }
            "c" | "C" => {
                app.show_consts = !app.show_consts;
                app.status = format!("Constellations: {}", if app.show_consts { "ON" } else { "OFF" });
            }
            "p" | "P" => {
                app.show_planets = !app.show_planets;
                app.status = format!("Planets: {}", if app.show_planets { "ON" } else { "OFF" });
            }
            "\x1b[H" => {
                app.view_ra = 0.5;
                app.view_dec = 0.5;
                app.zoom = 1.0;
                app.status = "View reset to center.".to_string();
            }
            _ => {}
        }
        app.tick += 1;
        draw(&app);
    }
}
