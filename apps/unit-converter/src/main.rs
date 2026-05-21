use std::io::{self, BufRead, Write};

const W: u32 = 760;
const H: u32 = 560;
const HEADER_H: u32  = 36;
const TABS_H: u32    = 36;
const CONTENT_Y: u32 = HEADER_H + TABS_H;
const STATUS_H: u32  = 28;
const RESULT_Y: u32  = CONTENT_Y + 220;

const C_BG: u32      = 0x161B22FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_SEL: u32     = 0x58A6FFFF;
const C_SEL_BG: u32  = 0x1F4068FF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_DIM: u32     = 0x8B949EFF;
const C_HINT: u32    = 0x6E7681FF;
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

struct Category {
    name:  &'static str,
    units: &'static [(&'static str, f64)],  // (name, factor_to_base)
}

// Temperature handled specially. Base unit is Celsius.
fn to_celsius(val: f64, unit_idx: usize) -> f64 {
    match unit_idx {
        0 => val,             // Celsius
        1 => (val - 32.0) * 5.0 / 9.0, // Fahrenheit
        2 => val - 273.15,    // Kelvin
        _ => val,
    }
}
fn from_celsius(celsius: f64, unit_idx: usize) -> f64 {
    match unit_idx {
        0 => celsius,
        1 => celsius * 9.0 / 5.0 + 32.0,
        2 => celsius + 273.15,
        _ => celsius,
    }
}

const CATEGORIES: &[Category] = &[
    Category { name: "Length", units: &[
        ("Meter",      1.0),
        ("Kilometer",  1000.0),
        ("Mile",       1609.344),
        ("Foot",       0.3048),
        ("Inch",       0.0254),
        ("Centimeter", 0.01),
        ("Yard",       0.9144),
        ("Nautical mi",1852.0),
    ]},
    Category { name: "Mass", units: &[
        ("Kilogram",   1.0),
        ("Gram",       0.001),
        ("Pound",      0.453592),
        ("Ounce",      0.0283495),
        ("Tonne",      1000.0),
        ("Stone",      6.35029),
        ("Milligram",  0.000001),
    ]},
    Category { name: "Temperature", units: &[
        ("Celsius",    1.0),
        ("Fahrenheit", 1.0),
        ("Kelvin",     1.0),
    ]},
    Category { name: "Speed", units: &[
        ("m/s",        1.0),
        ("km/h",       1.0/3.6),
        ("mph",        0.44704),
        ("knot",       0.514444),
        ("ft/s",       0.3048),
    ]},
    Category { name: "Area", units: &[
        ("m²",         1.0),
        ("km²",        1_000_000.0),
        ("ft²",        0.092903),
        ("acre",       4046.86),
        ("hectare",    10_000.0),
        ("mile²",      2_589_988.0),
    ]},
    Category { name: "Volume", units: &[
        ("Liter",      1.0),
        ("Milliliter", 0.001),
        ("Gallon (US)",3.78541),
        ("Gallon (UK)",4.54609),
        ("Fluid oz",   0.0295735),
        ("Cup",        0.24),
        ("m³",         1000.0),
    ]},
    Category { name: "Time", units: &[
        ("Second",     1.0),
        ("Minute",     60.0),
        ("Hour",       3600.0),
        ("Day",        86400.0),
        ("Week",       604800.0),
        ("Month",      2_629_800.0),
        ("Year",       31_557_600.0),
    ]},
];

fn convert(val: f64, from_idx: usize, to_idx: usize, cat_idx: usize) -> f64 {
    let cat = &CATEGORIES[cat_idx];
    if cat.name == "Temperature" {
        let celsius = to_celsius(val, from_idx);
        from_celsius(celsius, to_idx)
    } else {
        let base = val * cat.units[from_idx].1;
        base / cat.units[to_idx].1
    }
}

#[derive(PartialEq)]
enum Focus { From, To }

fn draw(cat_idx: usize, from_idx: usize, to_idx: usize, input: &str, focus: &Focus) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H, W, 1, C_BORDER);
    text(16, 10, C_TEXT, "Unit Converter");
    text(W - 320, 10, C_HINT, "Tab: switch side  ↑↓: unit  Esc: close");

    // Category tabs
    fill(0, HEADER_H, W, TABS_H, 0x0D1117FF);
    fill(0, HEADER_H + TABS_H, W, 1, C_BORDER);
    let tab_w = W / CATEGORIES.len() as u32;
    for (i, cat) in CATEGORIES.iter().enumerate() {
        let tx = i as u32 * tab_w;
        let is_sel = i == cat_idx;
        if is_sel { fill(tx, HEADER_H, tab_w, TABS_H, C_SEL_BG); }
        let tc = if is_sel { C_SEL } else { C_DIM };
        let nw = cat.name.len() as u32 * 8;
        text(tx + (tab_w - nw.min(tab_w)) / 2, HEADER_H + 10, tc, cat.name);
        if is_sel { fill(tx, HEADER_H + TABS_H - 2, tab_w, 2, C_SEL); }
    }

    let cat = &CATEGORIES[cat_idx];
    let col_w = W / 2;

    // Column headers
    fill(0, CONTENT_Y, col_w, 28, 0x21262DFF);
    fill(col_w, CONTENT_Y, col_w, 28, 0x21262DFF);
    fill(0, CONTENT_Y + 28, W, 1, C_BORDER);
    let from_label = format!("From: {}", cat.units[from_idx].0);
    let to_label   = format!("To: {}", cat.units[to_idx].0);
    let fc = if *focus == Focus::From { C_SEL } else { C_DIM };
    let tc = if *focus == Focus::To   { C_SEL } else { C_DIM };
    text(12, CONTENT_Y + 8, fc, &from_label);
    text(col_w + 12, CONTENT_Y + 8, tc, &to_label);
    fill(col_w, CONTENT_Y, 1, H - CONTENT_Y, C_BORDER);

    // Unit lists
    let item_h = 28u32;
    let max_visible = ((RESULT_Y - CONTENT_Y - 30) / item_h) as usize;
    for (i, &(name, _)) in cat.units.iter().take(max_visible).enumerate() {
        let iy = CONTENT_Y + 30 + i as u32 * item_h;
        let is_from_sel = i == from_idx && *focus == Focus::From;
        let is_to_sel   = i == to_idx   && *focus == Focus::To;

        if is_from_sel {
            fill(0, iy, col_w - 1, item_h, C_SEL_BG);
            fill(0, iy, 3, item_h, C_SEL);
        }
        if is_to_sel {
            fill(col_w + 1, iy, col_w - 1, item_h, C_SEL_BG);
            fill(col_w + 1, iy, 3, item_h, C_SEL);
        }

        let ftc = if is_from_sel { C_TEXT } else if i == from_idx { C_DIM } else { C_HINT };
        let ttc = if is_to_sel   { C_TEXT } else if i == to_idx   { C_DIM } else { C_HINT };
        text(12, iy + 6, ftc, name);
        text(col_w + 12, iy + 6, ttc, name);
    }

    // Divider before result
    fill(0, RESULT_Y, W, 1, C_BORDER);

    // Input row
    let input_y = RESULT_Y + 10;
    let prompt = format!("{}_", input);
    text(16, input_y, C_TEXT, &prompt);
    text(120, input_y, C_DIM, cat.units[from_idx].0);

    // Convert and show result
    if let Ok(val) = input.parse::<f64>() {
        let result = convert(val, from_idx, to_idx, cat_idx);
        let result_str = if result.abs() < 1e6 && result.abs() > 1e-4 {
            format!("{:.6}", result).trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            format!("{:.4e}", result)
        };
        let rw = result_str.len() as u32 * 24;
        let rx = (W - rw) / 2;
        println!("VYOMA_DRAW:draw_text:{},{},{:#010x},l,{result_str}", rx, input_y + 30, C_GREEN);
        let unit_w = cat.units[to_idx].0.len() as u32 * 8;
        text((W - unit_w) / 2, input_y + 80, C_DIM, cat.units[to_idx].0);
    } else if !input.is_empty() {
        text(16, input_y + 40, 0xFF7B72FF, "Invalid number");
    }

    // Status
    fill(0, H - STATUS_H, W, STATUS_H, C_HEADER);
    fill(0, H - STATUS_H, W, 1, C_BORDER);
    text(16, H - STATUS_H + 8, C_HINT, "Type number  ←→: category  Tab: switch side  Backspace: delete");

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut cat_idx  = 0usize;
    let mut from_idx = 0usize;
    let mut to_idx   = 1usize;
    let mut input    = String::new();
    let mut focus    = Focus::From;

    println!("@supervisor: raise unit-converter");
    let _ = io::stdout().flush();

    draw(cat_idx, from_idx, to_idx, &input, &focus);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" | "\x1b" => {
                fill(0, 0, W, H, 0x0D1117FF);
                flush();
                std::process::exit(0);
            }
            "\x09" => {
                focus = if focus == Focus::From { Focus::To } else { Focus::From };
            }
            "\x1b[D" => {
                if cat_idx > 0 { cat_idx -= 1; from_idx = 0; to_idx = 1; input.clear(); }
            }
            "\x1b[C" => {
                if cat_idx + 1 < CATEGORIES.len() { cat_idx += 1; from_idx = 0; to_idx = 1; input.clear(); }
            }
            "\x1b[A" => {
                let n = CATEGORIES[cat_idx].units.len();
                match focus {
                    Focus::From => { if from_idx > 0 { from_idx -= 1; } }
                    Focus::To   => { if to_idx > 0   { to_idx -= 1; } }
                }
            }
            "\x1b[B" => {
                let n = CATEGORIES[cat_idx].units.len();
                match focus {
                    Focus::From => { if from_idx + 1 < n { from_idx += 1; } }
                    Focus::To   => { if to_idx   + 1 < n { to_idx   += 1; } }
                }
            }
            "\x7f" => { input.pop(); }
            ch if ch.len() == 1 => {
                let c = ch.chars().next().unwrap();
                if c.is_ascii_digit() || c == '.' || (c == '-' && input.is_empty()) {
                    input.push(c);
                }
            }
            _ => {}
        }
        draw(cat_idx, from_idx, to_idx, &input, &focus);
    }
}
