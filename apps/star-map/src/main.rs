use std::io::{self, BufRead, Write};

const W: u32   = 1200;
const H: u32   = 760;
const HDR: u32 = 40;
const INFO_H: u32 = 80;
const SB_H: u32   = 28;
const MAP_Y: u32  = HDR;
const MAP_H: u32  = H - HDR - INFO_H - SB_H;
const MAP_W: u32  = W;
const INFO_Y: u32 = MAP_Y + MAP_H;

const C_SKY: u32    = 0x050A12FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_BG: u32     = 0x0D1117FF;

// Spectral colors
fn spec_color(s: u8) -> u32 {
    match s {
        b'O' => 0x9BB0FFFF,
        b'B' => 0xBBCFFFFF,
        b'A' => 0xE0E8FFFF,
        b'F' => 0xFFF4EAFF,
        b'G' => 0xFFEE88FF,
        b'K' => 0xFFCC6FFF,
        b'M' => 0xFF8866FF,
        _    => C_TEXT,
    }
}

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

fn draw_line(x0: i32, y0: i32, x1: i32, y1: i32, col: u32, clip_y0: i32, clip_y1: i32) {
    let dx = (x1 - x0).abs();
    let dy = (y1 - y0).abs();
    let sx = if x0 < x1 { 1i32 } else { -1 };
    let sy = if y0 < y1 { 1i32 } else { -1 };
    let mut err = dx - dy;
    let mut x = x0;
    let mut y = y0;
    let mut steps = 0i32;
    loop {
        if x >= 0 && x < W as i32 && y >= clip_y0 && y < clip_y1 {
            // dashed: draw every other pixel
            if steps % 2 == 0 {
                fill(x as u32, y as u32, 1, 1, col);
            }
        }
        if x == x1 && y == y1 { break; }
        let e2 = 2 * err;
        if e2 > -dy { err -= dy; x += sx; }
        if e2 < dx  { err += dx; y += sy; }
        steps += 1;
        if steps > 4000 { break; } // safety limit
    }
}

struct Star {
    name: &'static str,
    ra:   f32,  // hours 0..24
    dec:  f32,  // degrees -90..+90
    mag:  f32,
    spec: u8,
    myth: &'static str,
}

struct Constellation {
    name:  &'static str,
    lines: &'static [(usize, usize)],
}

const STARS: &[Star] = &[
    Star { name: "Sirius",         ra: 6.75,  dec: -16.7,  mag: -1.46, spec: b'A', myth: "Canis Majoris α — the Dog Star, brightest star in the night sky" },
    Star { name: "Canopus",        ra: 6.40,  dec: -52.7,  mag: -0.74, spec: b'F', myth: "Carinae α — the navigator's star, second brightest in the sky" },
    Star { name: "Arcturus",       ra: 14.26, dec: 19.2,   mag: -0.05, spec: b'K', myth: "Boötis α — Guardian of the Bear, a red-orange giant 37 ly away" },
    Star { name: "Vega",           ra: 18.62, dec: 38.8,   mag: 0.03,  spec: b'A', myth: "Lyrae α — the Falling Eagle, anchor of the Summer Triangle" },
    Star { name: "Capella",        ra: 5.28,  dec: 46.0,   mag: 0.08,  spec: b'G', myth: "Aurigae α — the Goat Star, actually two giant stars in close orbit" },
    Star { name: "Rigel",          ra: 5.24,  dec: -8.2,   mag: 0.13,  spec: b'B', myth: "Orionis β — the Hunter's left foot, a blue supergiant 860 ly away" },
    Star { name: "Procyon",        ra: 7.65,  dec: 5.2,    mag: 0.38,  spec: b'F', myth: "Canis Minoris α — Before the Dog, the Lesser Dog Star" },
    Star { name: "Achernar",       ra: 1.63,  dec: -57.2,  mag: 0.46,  spec: b'B', myth: "Eridani α — End of the River, fastest-rotating bright star" },
    Star { name: "Betelgeuse",     ra: 5.92,  dec: 7.4,    mag: 0.50,  spec: b'M', myth: "Orionis α — Armpit of the Giant, a dying red supergiant" },
    Star { name: "Altair",         ra: 19.85, dec: 8.9,    mag: 0.77,  spec: b'A', myth: "Aquilae α — the Eagle Star, spins so fast it bulges at equator" },
    Star { name: "Aldebaran",      ra: 4.60,  dec: 16.5,   mag: 0.87,  spec: b'K', myth: "Tauri α — the Follower, red eye of the Bull, 65 ly away" },
    Star { name: "Spica",          ra: 13.42, dec: -11.2,  mag: 1.04,  spec: b'B', myth: "Virginis α — ear of wheat held by Virgo, used by Hipparchus" },
    Star { name: "Antares",        ra: 16.49, dec: -26.4,  mag: 1.06,  spec: b'M', myth: "Scorpii α — Rival of Mars, the red heart of the Scorpion" },
    Star { name: "Pollux",         ra: 7.76,  dec: 28.0,   mag: 1.16,  spec: b'K', myth: "Geminorum β — the immortal twin, nearest giant to Earth at 34 ly" },
    Star { name: "Fomalhaut",      ra: 22.96, dec: -29.6,  mag: 1.16,  spec: b'A', myth: "Piscis Austrini α — Mouth of the Southern Fish, has a debris disk" },
    Star { name: "Deneb",          ra: 20.69, dec: 45.3,   mag: 1.25,  spec: b'A', myth: "Cygni α — Tail of the Swan, may be the most luminous naked-eye star" },
    Star { name: "Regulus",        ra: 10.14, dec: 11.97,  mag: 1.36,  spec: b'B', myth: "Leonis α — the Little King, heart of Leo, lies near ecliptic" },
    Star { name: "Castor",         ra: 7.58,  dec: 31.9,   mag: 1.58,  spec: b'A', myth: "Geminorum α — mortal twin of Pollux, actually a sextuple star system" },
    Star { name: "Polaris",        ra: 2.53,  dec: 89.3,   mag: 2.02,  spec: b'F', myth: "Ursae Minoris α — the North Star, within 0.7° of the celestial pole" },
    Star { name: "Bellatrix",      ra: 5.42,  dec: 6.3,    mag: 1.64,  spec: b'B', myth: "Orionis γ — the Amazon Star, Orion's right shoulder" },
    Star { name: "Elnath",         ra: 5.44,  dec: 28.6,   mag: 1.65,  spec: b'B', myth: "Tauri β — the Butting One, the northern tip of the Bull's horn" },
    Star { name: "Alnilam",        ra: 5.60,  dec: -1.2,   mag: 1.70,  spec: b'B', myth: "Orionis ε — center of Orion's Belt, a supergiant 1340 ly away" },
    Star { name: "Alnitak",        ra: 5.68,  dec: -1.9,   mag: 1.74,  spec: b'B', myth: "Orionis ζ — eastern belt star, above the famous Horsehead Nebula" },
    Star { name: "Mintaka",        ra: 5.53,  dec: -0.3,   mag: 2.23,  spec: b'O', myth: "Orionis δ — western belt star, almost exactly on the celestial equator" },
    Star { name: "Dubhe",          ra: 11.06, dec: 61.8,   mag: 1.79,  spec: b'K', myth: "Ursae Majoris α — back of the Bear, pointer star to Polaris" },
    Star { name: "Alkaid",         ra: 13.79, dec: 49.3,   mag: 1.86,  spec: b'B', myth: "Ursae Majoris η — end of the Big Dipper's handle, Benetnasch" },
    Star { name: "Alioth",         ra: 12.90, dec: 55.95,  mag: 1.77,  spec: b'A', myth: "Ursae Majoris ε — brightest star in the Big Dipper" },
    Star { name: "Mizar",          ra: 13.40, dec: 54.9,   mag: 2.04,  spec: b'A', myth: "Ursae Majoris ζ — first known visual double star (with Alcor)" },
    Star { name: "Megrez",         ra: 12.26, dec: 57.0,   mag: 3.31,  spec: b'A', myth: "Ursae Majoris δ — root of the Bear's tail, dimmest Dipper star" },
    Star { name: "Phecda",         ra: 11.90, dec: 53.7,   mag: 2.44,  spec: b'A', myth: "Ursae Majoris γ — lower-left star in the Big Dipper bowl" },
    Star { name: "Merak",          ra: 11.03, dec: 56.4,   mag: 2.37,  spec: b'A', myth: "Ursae Majoris β — with Dubhe, the pointer stars to the North Star" },
    Star { name: "Saiph",          ra: 5.80,  dec: -9.7,   mag: 2.09,  spec: b'B', myth: "Orionis κ — Orion's right foot, a hot blue supergiant" },
    Star { name: "Kochab",         ra: 14.85, dec: 74.16,  mag: 2.08,  spec: b'K', myth: "Ursae Minoris β — was the pole star around 1500 BCE" },
    Star { name: "Schedar",        ra: 0.68,  dec: 56.5,   mag: 2.24,  spec: b'K', myth: "Cassiopeiae α — Breast of Cassiopeia, anchors the W pattern" },
    Star { name: "Caph",           ra: 0.15,  dec: 59.1,   mag: 2.28,  spec: b'F', myth: "Cassiopeiae β — the Hand, the westernmost tip of the W" },
    Star { name: "Gamma Cas",      ra: 0.94,  dec: 60.7,   mag: 2.15,  spec: b'B', myth: "Cassiopeiae γ — Navi, the center jewel of Cassiopeia's W" },
    Star { name: "Ruchbah",        ra: 1.43,  dec: 60.2,   mag: 2.68,  spec: b'A', myth: "Cassiopeiae δ — the Knee, fourth star in the W shape" },
    Star { name: "Segin",          ra: 1.91,  dec: 63.7,   mag: 3.35,  spec: b'B', myth: "Cassiopeiae ε — easternmost star completing Cassiopeia's W" },
    Star { name: "Shaula",         ra: 17.56, dec: -37.1,  mag: 1.62,  spec: b'B', myth: "Scorpii λ — the Sting, one of two stars at the Scorpion's tail tip" },
    Star { name: "Lesath",         ra: 17.53, dec: -37.3,  mag: 2.69,  spec: b'B', myth: "Scorpii υ — the Sting's companion; Shaula and Lesath are the Cat's Eyes" },
    Star { name: "Epsilon Sco",    ra: 16.84, dec: -34.3,  mag: 2.29,  spec: b'K', myth: "Scorpii ε — Wei, part of the curving tail of the Scorpion" },
    Star { name: "Delta Sco",      ra: 16.01, dec: -22.6,  mag: 2.01,  spec: b'B', myth: "Scorpii δ — Dschubba, the Scorpion's forehead, brightened in 2000" },
    Star { name: "Beta Sco",       ra: 16.09, dec: -19.8,  mag: 2.62,  spec: b'B', myth: "Scorpii β — Graffias (Claws), one of the Scorpion's claws" },
    Star { name: "Denebola",       ra: 11.82, dec: 14.6,   mag: 2.14,  spec: b'A', myth: "Leonis β — tail of the Lion, a fast-rotating white star" },
    Star { name: "Zosma",          ra: 11.24, dec: 20.5,   mag: 2.56,  spec: b'A', myth: "Leonis δ — the Girdle, the haunches/back of the Lion" },
    Star { name: "Algieba",        ra: 10.33, dec: 19.8,   mag: 1.98,  spec: b'K', myth: "Leonis γ — the Mane, a beautiful orange-yellow double star" },
    Star { name: "Hamal",          ra: 2.12,  dec: 23.5,   mag: 2.01,  spec: b'K', myth: "Arietis α — the Ram's Head, was near the vernal equinox in 68 BCE" },
    Star { name: "Mirfak",         ra: 3.41,  dec: 49.9,   mag: 1.79,  spec: b'F', myth: "Persei α — the Elbow of Perseus, surrounded by the Alpha Per Cluster" },
    Star { name: "Eltanin",        ra: 17.94, dec: 51.5,   mag: 2.24,  spec: b'K', myth: "Draconis γ — the Dragon's Head, brightest star in Draco" },
    Star { name: "Kaus Australis", ra: 18.40, dec: -34.4,  mag: 1.85,  spec: b'B', myth: "Sagittarii ε — Southern Bow, brightest star in Sagittarius" },
    Star { name: "Nunki",          ra: 18.92, dec: -26.3,  mag: 2.05,  spec: b'B', myth: "Sagittarii σ — the Sea (Sumerian), part of the Teapot asterism" },
    Star { name: "Alhena",         ra: 6.63,  dec: 16.4,   mag: 1.93,  spec: b'A', myth: "Geminorum γ — the Brand, bright foot of the twin Pollux in Gemini" },
    Star { name: "Alphard",        ra: 9.46,  dec: -8.66,  mag: 1.98,  spec: b'K', myth: "Hydrae α — the Solitary One, brightest star in the Water Snake" },
    Star { name: "Alphecca",       ra: 15.58, dec: 26.7,   mag: 2.21,  spec: b'A', myth: "Coronae Borealis α — the Gem of the Crown, a spectroscopic binary" },
    Star { name: "Mirzam",         ra: 6.38,  dec: -18.0,  mag: 1.98,  spec: b'B', myth: "Canis Majoris β — the Announcer, heralds Sirius's rising" },
    Star { name: "Adhara",         ra: 6.98,  dec: -29.0,  mag: 1.50,  spec: b'B', myth: "Canis Majoris ε — the Virgins, second brightest in Canis Major" },
    Star { name: "Wezen",          ra: 7.14,  dec: -26.4,  mag: 1.84,  spec: b'F', myth: "Canis Majoris δ — the Weight, a rare yellow-white supergiant" },
    Star { name: "Porrima",        ra: 12.69, dec: -1.45,  mag: 2.74,  spec: b'F', myth: "Virginis γ — Goddess of Prophecy, a beautiful visual binary" },
    Star { name: "Vindemiatrix",   ra: 13.04, dec: 10.96,  mag: 2.83,  spec: b'G', myth: "Virginis ε — the Grape Gatherer, rises at time of grape harvest" },
    Star { name: "Tau Sco",        ra: 16.35, dec: -28.2,  mag: 2.82,  spec: b'B', myth: "Scorpii τ — part of the Scorpion's body beneath the heart" },
    Star { name: "Hadar",          ra: 14.07, dec: -60.4,  mag: 0.61,  spec: b'B', myth: "Centauri β — Agena (the Knee), third brightest star in the sky" },
    Star { name: "Peacock",        ra: 20.43, dec: -56.7,  mag: 1.94,  spec: b'B', myth: "Pavonis α — the Peacock Star, navigational star for southern sky" },
    Star { name: "Atria",          ra: 16.81, dec: -69.0,  mag: 1.92,  spec: b'K', myth: "Trianguli Australis α — brightest star in the Southern Triangle" },
];

const CONSTELLATIONS: &[Constellation] = &[
    Constellation { name: "Orion",      lines: &[(5,31),(5,22),(8,19),(8,22),(19,23),(21,22),(21,23),(23,19)] },
    Constellation { name: "Ursa Major", lines: &[(24,30),(30,29),(29,28),(28,24),(28,26),(26,27),(27,25)] },
    Constellation { name: "Cassiopeia", lines: &[(34,33),(33,35),(35,36),(36,37)] },
    Constellation { name: "Scorpius",   lines: &[(42,41),(41,12),(12,59),(59,40),(40,38),(38,39)] },
    Constellation { name: "Leo",        lines: &[(16,45),(45,43),(43,44),(44,45),(16,44)] },
    Constellation { name: "Gemini",     lines: &[(17,13),(17,51),(13,51)] },
    Constellation { name: "Taurus",     lines: &[(10,20)] },
    Constellation { name: "Canis Major",lines: &[(0,54),(0,55),(55,56)] },
    Constellation { name: "Virgo",      lines: &[(11,57),(57,58)] },
    Constellation { name: "Sagittarius",lines: &[(49,50)] },
    Constellation { name: "Lyra",       lines: &[] },
    Constellation { name: "Cygnus",     lines: &[] },
];

fn project(ra: f32, dec: f32, vra: f32, vra_span: f32, vdec: f32, vdec_span: f32) -> Option<(i32, i32)> {
    if vra_span <= 0.0 || vdec_span <= 0.0 { return None; }
    let fx = (ra - vra) / vra_span;
    let fy = 1.0 - (dec - vdec) / vdec_span;
    if fx < -0.02 || fx > 1.02 || fy < -0.02 || fy > 1.02 { return None; }
    let x = (fx * MAP_W as f32) as i32;
    let y = MAP_Y as i32 + (fy * MAP_H as f32) as i32;
    Some((x, y))
}

fn star_size(mag: f32) -> u32 {
    if mag < 0.0 { 6 } else if mag < 1.0 { 5 } else if mag < 2.0 { 4 } else if mag < 3.0 { 3 } else { 2 }
}

struct App {
    vra:       f32,  // left RA (hours)
    vdec:      f32,  // bottom Dec (degrees)
    vra_span:  f32,
    vdec_span: f32,
    sel:       usize,
    show_const: bool,
    show_names: bool,
    status:    String,
}

impl App {
    fn new() -> Self {
        App {
            vra: 0.0, vdec: -60.0,
            vra_span: 24.0, vdec_span: 120.0,
            sel: 0,
            show_const: true,
            show_names: true,
            status: String::from("←→↑↓=pan  +/-=zoom  C=constellations  N=names  R=reset  Ctrl+C=quit"),
        }
    }
}

fn draw(app: &App) {
    // Sky background
    fill(0, 0, W, H, C_BG);
    fill(0, MAP_Y, MAP_W, MAP_H, C_SKY);

    // Header
    fill(0, 0, W, HDR, C_HEADER);
    fill(0, HDR - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Star Map");
    text(120, 12, C_HINT, &format!("RA {:.1}h..{:.1}h  Dec {:.0}°..{:.0}°",
        app.vra, app.vra + app.vra_span, app.vdec, app.vdec + app.vdec_span));

    let clip0 = MAP_Y as i32;
    let clip1 = (MAP_Y + MAP_H) as i32;

    // Draw constellation lines first (behind stars)
    if app.show_const {
        for con in CONSTELLATIONS {
            for &(a, b) in con.lines {
                if a >= STARS.len() || b >= STARS.len() { continue; }
                let sa = &STARS[a];
                let sb = &STARS[b];
                if let (Some((x0,y0)), Some((x1,y1))) = (
                    project(sa.ra, sa.dec, app.vra, app.vra_span, app.vdec, app.vdec_span),
                    project(sb.ra, sb.dec, app.vra, app.vra_span, app.vdec, app.vdec_span),
                ) {
                    draw_line(x0, y0, x1, y1, 0x30363DFF, clip0, clip1);
                }
            }
        }
    }

    // Draw stars
    for (i, star) in STARS.iter().enumerate() {
        let Some((sx, sy)) = project(star.ra, star.dec, app.vra, app.vra_span, app.vdec, app.vdec_span) else { continue };
        if sy < clip0 || sy >= clip1 { continue; }
        let col = spec_color(star.spec);
        let sz = star_size(star.mag) as i32;
        let px = (sx - sz / 2).max(0) as u32;
        let py = (sy - sz / 2).max(clip0) as u32;
        fill(px, py, sz as u32, sz as u32, col);

        // Selection ring
        if i == app.sel {
            let rx = (sx - sz).max(0) as u32;
            let ry = (sy - sz).max(clip0) as u32;
            let rw = (sz * 2 + 2) as u32;
            border(rx, ry, rw, rw, C_SEL);
        }

        // Star name
        if app.show_names && star.mag < 2.0 {
            let nx = (sx + sz + 2).max(0) as u32;
            let ny = (sy - 6).max(clip0) as u32;
            if ny + 16 < clip1 as u32 {
                text(nx, ny, 0xE6EDF3A0, star.name);
            }
        }
    }

    // Grid lines (RA lines every 3h, Dec lines every 30°)
    for ra_tick in 0..8 {
        let ra = ra_tick as f32 * 3.0;
        if let Some((x, _)) = project(ra, app.vdec, app.vra, app.vra_span, app.vdec, app.vdec_span) {
            if x >= 0 && x < W as i32 {
                fill(x as u32, MAP_Y, 1, MAP_H, 0x30363D40);
                text(x as u32 + 2, MAP_Y + 2, C_HINT, &format!("{}h", ra as u32));
            }
        }
    }
    for dec_tick in -3i32..=3 {
        let dec = dec_tick as f32 * 30.0;
        if let Some((_, y)) = project(app.vra, dec, app.vra, app.vra_span, app.vdec, app.vdec_span) {
            if y >= clip0 && y < clip1 {
                fill(0, y as u32, MAP_W, 1, 0x30363D40);
                text(4, y as u32 + 2, C_HINT, &format!("{:+}°", dec as i32));
            }
        }
    }

    // Info panel
    fill(0, INFO_Y, W, INFO_H, C_CARD);
    fill(0, INFO_Y, W, 1, C_BORDER);

    if app.sel < STARS.len() {
        let s = &STARS[app.sel];
        let col = spec_color(s.spec);
        text(16, INFO_Y + 8, col, s.name);
        text(16 + s.name.len() as u32 * 8 + 8, INFO_Y + 8, C_HINT,
            &format!("  RA {:.2}h  Dec {:+.1}°  mag {:.2}  type {}",
                s.ra, s.dec, s.mag, s.spec as char));

        // Find constellation
        let con_name = CONSTELLATIONS.iter()
            .find(|c| c.lines.iter().any(|&(a,b)| a==app.sel||b==app.sel))
            .map(|c| c.name).unwrap_or("—");
        text(16, INFO_Y + 28, C_HINT, &format!("Constellation: {}  |  {}", con_name, s.myth));
    }

    // Status bar
    let sb_y = H - SB_H;
    fill(0, sb_y, W, SB_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise star-map");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }

            // Pan
            "\x1b[D" => { app.vra = (app.vra - app.vra_span * 0.1).max(-4.0); }
            "\x1b[C" => { app.vra = (app.vra + app.vra_span * 0.1).min(28.0); }
            "\x1b[A" => { app.vdec = (app.vdec + app.vdec_span * 0.1).min(89.0); }
            "\x1b[B" => { app.vdec = (app.vdec - app.vdec_span * 0.1).max(-90.0); }

            // Zoom
            "+" | "=" => {
                let cra  = app.vra  + app.vra_span  / 2.0;
                let cdec = app.vdec + app.vdec_span / 2.0;
                app.vra_span  = (app.vra_span  * 0.67).max(1.0);
                app.vdec_span = (app.vdec_span * 0.67).max(5.0);
                app.vra  = cra  - app.vra_span  / 2.0;
                app.vdec = cdec - app.vdec_span / 2.0;
            }
            "-" => {
                let cra  = app.vra  + app.vra_span  / 2.0;
                let cdec = app.vdec + app.vdec_span / 2.0;
                app.vra_span  = (app.vra_span  * 1.5).min(30.0);
                app.vdec_span = (app.vdec_span * 1.5).min(180.0);
                app.vra  = cra  - app.vra_span  / 2.0;
                app.vdec = cdec - app.vdec_span / 2.0;
            }

            // Navigate stars
            "\x1b[5~" => { if app.sel > 0 { app.sel -= 1; } }
            "\x1b[6~" => { if app.sel + 1 < STARS.len() { app.sel += 1; } }
            "" => {
                // Enter — center view on selected star
                let s = &STARS[app.sel];
                app.vra  = s.ra  - app.vra_span  / 2.0;
                app.vdec = s.dec - app.vdec_span / 2.0;
            }

            "c" | "C" => { app.show_const = !app.show_const; }
            "n" | "N" => { app.show_names = !app.show_names; }
            "r" | "R" => {
                app.vra = 0.0; app.vdec = -60.0;
                app.vra_span = 24.0; app.vdec_span = 120.0;
                app.sel = 0;
            }

            _ => {}
        }

        draw(&app);
    }
}
