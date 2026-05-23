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
const C_GREEN: u32  = 0x3FB950FF;
const C_CARD: u32   = 0x161B22FF;
const C_RED: u32    = 0xFF7B72FF;

const C_RESIST: u32 = 0x3D1F00FF;
const C_CAP: u32    = 0x001A2DFF;
const C_ELED: u32   = 0x2D0505FF;
const C_BATT: u32   = 0x003300FF;
const C_SWITCH: u32 = 0x1A1A1AFF;
const C_IND: u32    = 0x1A003DFF;

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

fn wh(x1: i32, y: i32, x2: i32) { if x2 > x1 { fill(x1, y, x2 - x1, 2, C_TEXT); } }
fn wv(x: i32, y1: i32, y2: i32) { if y2 > y1 { fill(x, y1, 2, y2 - y1, C_TEXT); } }

fn comp(cx: i32, cy: i32, w: i32, lbl: &str, col: u32) {
    let h = 26i32;
    fill(cx - w / 2, cy - h / 2, w, h, col);
    border(cx - w / 2, cy - h / 2, w, h, C_BORDER);
    let tx = cx - (lbl.len() as i32 * 8) / 2;
    text(tx, cy - 8, C_TEXT, lbl);
}

fn battery_sym(x: i32, y: i32) {
    fill(x, y - 20, 4, 40, C_BATT);
    fill(x + 8, y - 12, 4, 24, C_BATT);
    text(x - 10, y - 32, C_GREEN, "-");
    text(x + 14, y - 32, C_GREEN, "+");
}

fn cap_sym(cx: i32, cy: i32) {
    fill(cx - 14, cy - 3, 28, 3, C_SEL);
    fill(cx - 14, cy + 4, 28, 3, C_SEL);
    wv(cx, cy - 16, cy - 3);
    wv(cx, cy + 7, cy + 20);
}

fn gnd_sym(x: i32, y: i32) {
    fill(x - 12, y, 24, 2, C_HINT);
    fill(x - 8, y + 5, 16, 2, C_HINT);
    fill(x - 4, y + 10, 8, 2, C_HINT);
}

fn node(x: i32, y: i32) { fill(x - 3, y - 3, 6, 6, C_TEXT); }

const CIRCUIT_NAMES: &[&str] = &[
    "01: Simple LED Circuit",
    "02: Parallel Resistors",
    "03: RC Low-Pass Filter",
    "04: Voltage Divider",
    "05: LED Array (Series)",
    "06: LED Array (Parallel)",
    "07: Band-Pass Filter",
    "08: Half-Wave Rectifier",
    "09: Wheatstone Bridge",
    "10: H-Bridge Driver",
];

const CIRCUIT_DESC: &[&str] = &[
    "Battery + switch + resistor + LED in series",
    "Battery + two parallel resistors + LED",
    "Input signal + resistor, capacitor shunted to GND",
    "Battery + R1 + R2 to GND, tap at junction",
    "Battery + resistor + three LEDs in series",
    "Battery + resistor → three parallel LEDs",
    "R1 + C1 shunt + R2 + C2 shunt = band-pass",
    "AC source + diode + filter capacitor + load resistor",
    "Battery + four resistors in diamond, meter across middle",
    "Battery + 4 switches + motor load (M)",
];

fn draw_circuit(idx: usize, ox: i32, oy: i32) {
    let my = oy + 200; // main wire y
    let by = oy + 380; // bottom/GND wire y

    match idx {
        0 => {
            // Simple: B - SW - R - LED
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, ox + 200);
            comp(ox + 240, my, 60, "SW", C_SWITCH);
            wh(ox + 270, my, ox + 340);
            comp(ox + 380, my, 60, "R1", C_RESIST);
            wh(ox + 410, my, ox + 480);
            comp(ox + 520, my, 70, "LED", C_ELED);
            wh(ox + 555, my, ox + 680);
            wv(ox + 680, my, by);
            wh(ox + 60, by, ox + 680);
            wv(ox + 60, my + 40, by);
            gnd_sym(ox + 370, by + 2);
            text(ox + 350, by + 18, C_HINT, "GND");
            text(ox + 224, my - 24, C_HINT, "Switch");
            text(ox + 364, my - 24, C_ORANGE, "Resistor");
            text(ox + 502, my - 24, C_RED, "LED");
        }
        1 => {
            // Parallel resistors: B - (R1||R2) - LED
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, ox + 200);
            node(ox + 200, my);
            wv(ox + 200, my, my - 60);
            comp(ox + 290, my - 60, 60, "R1", C_RESIST);
            wv(ox + 200, my, my + 60);
            comp(ox + 290, my + 60, 60, "R2", C_RESIST);
            wv(ox + 380, my - 60, my + 60);
            node(ox + 380, my);
            wh(ox + 380, my, ox + 460);
            comp(ox + 500, my, 70, "LED", C_ELED);
            wh(ox + 535, my, ox + 640);
            wv(ox + 640, my, by);
            wh(ox + 60, by, ox + 640);
            wv(ox + 60, my + 40, by);
            gnd_sym(ox + 350, by + 2);
            text(ox + 196, my - 84, C_ORANGE, "R1");
            text(ox + 196, my + 76, C_ORANGE, "R2");
        }
        2 => {
            // RC Low-pass: B - R - node(C to GND) - output
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, ox + 220);
            comp(ox + 260, my, 60, "R1", C_RESIST);
            wh(ox + 290, my, ox + 420);
            node(ox + 420, my);
            cap_sym(ox + 420, my + 60);
            wv(ox + 420, my + 80, by);
            node(ox + 420, by);
            wh(ox + 420, my, ox + 580);
            text(ox + 570, my - 20, C_SEL, "Output");
            fill(ox + 580, my - 6, 12, 12, C_SEL);
            wv(ox + 680, my, by);
            wh(ox + 60, by, ox + 680);
            wv(ox + 60, my + 40, by);
            gnd_sym(ox + 280, by + 2);
            text(ox + 420, my + 88, C_SEL, "C1");
            text(ox + 244, my - 24, C_ORANGE, "R1");
        }
        3 => {
            // Voltage divider: B - R1 - R2 - GND, tap at junction
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, ox + 240);
            comp(ox + 280, my, 60, "R1", C_RESIST);
            wh(ox + 310, my, ox + 340);
            node(ox + 340, my);
            wh(ox + 340, my, ox + 580);
            text(ox + 570, my - 20, C_SEL, "Vout");
            fill(ox + 578, my - 6, 12, 12, C_SEL);
            wv(ox + 340, my, my + 100);
            comp(ox + 340, my + 140, 60, "R2", C_RESIST);
            wv(ox + 340, my + 154, by);
            node(ox + 340, by);
            wv(ox + 680, my, by);
            wh(ox + 60, by, ox + 680);
            wv(ox + 60, my + 40, by);
            gnd_sym(ox + 480, by + 2);
            text(ox + 264, my - 24, C_ORANGE, "R1");
            text(ox + 354, my + 134, C_ORANGE, "R2");
        }
        4 => {
            // LED series: B - R - D1 - D2 - D3
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, ox + 160);
            comp(ox + 195, my, 60, "R1", C_RESIST);
            wh(ox + 225, my, ox + 280);
            comp(ox + 315, my, 60, "D1", C_ELED);
            wh(ox + 345, my, ox + 400);
            comp(ox + 435, my, 60, "D2", C_ELED);
            wh(ox + 465, my, ox + 520);
            comp(ox + 555, my, 60, "D3", C_ELED);
            wh(ox + 585, my, ox + 680);
            wv(ox + 680, my, by);
            wh(ox + 60, by, ox + 680);
            wv(ox + 60, my + 40, by);
            gnd_sym(ox + 370, by + 2);
        }
        5 => {
            // LED parallel: B - R - branch to 3 LEDs
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, ox + 200);
            comp(ox + 240, my, 60, "R1", C_RESIST);
            wh(ox + 270, my, ox + 360);
            node(ox + 360, my);
            // 3 parallel LEDs
            for (k, dy) in [-70i32, 0, 70].iter().enumerate() {
                let lmy = my + dy;
                wv(ox + 360, my.min(lmy), my.max(lmy) + 1);
                wh(ox + 360, lmy, ox + 440);
                comp(ox + 475, lmy, 60, &format!("D{}", k + 1), C_ELED);
                wh(ox + 505, lmy, ox + 560);
                wv(ox + 560, lmy.min(by), lmy.max(by) + 1);
            }
            node(ox + 560, my);
            wh(ox + 560, my, ox + 680);
            wv(ox + 680, my, by);
            wh(ox + 60, by, ox + 680);
            wv(ox + 60, my + 40, by);
            gnd_sym(ox + 370, by + 2);
        }
        6 => {
            // Band-pass: R1 - C1 shunt - R2 - C2 shunt
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, ox + 140);
            comp(ox + 180, my, 60, "R1", C_RESIST);
            wh(ox + 210, my, ox + 260);
            node(ox + 260, my);
            cap_sym(ox + 260, my + 60);
            wv(ox + 260, my + 80, by);
            node(ox + 260, by);
            wh(ox + 260, my, ox + 340);
            comp(ox + 380, my, 60, "R2", C_RESIST);
            wh(ox + 410, my, ox + 480);
            node(ox + 480, my);
            cap_sym(ox + 480, my + 60);
            wv(ox + 480, my + 80, by);
            node(ox + 480, by);
            wh(ox + 480, my, ox + 600);
            text(ox + 590, my - 20, C_SEL, "Out");
            fill(ox + 600, my - 6, 12, 12, C_SEL);
            wv(ox + 680, my, by);
            wh(ox + 60, by, ox + 680);
            wv(ox + 60, my + 40, by);
            gnd_sym(ox + 370, by + 2);
            text(ox + 260, my + 88, C_SEL, "C1");
            text(ox + 480, my + 88, C_SEL, "C2");
        }
        7 => {
            // Half-wave rectifier: AC - D - C(shunt) - R(load)
            fill(ox + 40, my - 18, 36, 36, C_BATT);
            border(ox + 40, my - 18, 36, 36, C_BORDER);
            text(ox + 46, my - 8, C_GREEN, "AC");
            wh(ox + 76, my, ox + 180);
            comp(ox + 215, my, 60, "D1", C_ELED);
            wh(ox + 245, my, ox + 340);
            node(ox + 340, my);
            cap_sym(ox + 340, my + 60);
            wv(ox + 340, my + 80, by);
            node(ox + 340, by);
            wh(ox + 340, my, ox + 460);
            comp(ox + 500, my, 60, "RL", C_RESIST);
            wh(ox + 530, my, ox + 640);
            wv(ox + 640, my, by);
            wh(ox + 40, by, ox + 640);
            wv(ox + 40, my + 18, by);
            gnd_sym(ox + 340, by + 2);
            text(ox + 340, my + 88, C_SEL, "C1");
            text(ox + 200, my - 24, C_RED, "Diode");
            text(ox + 480, my - 24, C_ORANGE, "Load");
        }
        8 => {
            // Wheatstone bridge: V top, GND bottom, 4 Rs in diamond
            let mid_x = ox + 380;
            let top_y = my - 80;
            let bot_y = my + 80;
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, mid_x);
            node(mid_x, my);
            // Top diamond
            wv(mid_x, my, top_y);
            node(mid_x, top_y);
            wh(mid_x, top_y, mid_x + 80);
            comp(mid_x + 120, top_y, 60, "R1", C_RESIST);
            wh(mid_x + 150, top_y, mid_x + 220);
            node(mid_x + 220, top_y);
            wh(mid_x, top_y, mid_x - 80); // other top
            comp(mid_x - 120, top_y, 60, "R2", C_RESIST);
            wh(mid_x - 150, top_y, mid_x - 220);
            node(mid_x - 220, top_y);
            // Bottom diamond
            wv(mid_x, my, bot_y);
            node(mid_x, bot_y);
            wh(mid_x, bot_y, mid_x + 80);
            comp(mid_x + 120, bot_y, 60, "R3", C_RESIST);
            wh(mid_x + 150, bot_y, mid_x + 220);
            wv(mid_x + 220, top_y, bot_y);
            wh(mid_x - 80, bot_y, mid_x);
            comp(mid_x - 120, bot_y, 60, "R4", C_RESIST);
            wh(mid_x - 220, bot_y, mid_x - 150);
            wv(mid_x - 220, top_y, bot_y);
            // Meter across middle horizontal
            wh(mid_x - 220, my, mid_x - 80);
            comp(mid_x - 40, my, 60, "M", 0x001A2DFF);
            wh(mid_x + 20, my, mid_x + 220);
            // Right to GND
            wv(mid_x + 220, bot_y, by);
            wh(mid_x - 220, by, mid_x + 220);
            wv(mid_x - 220, bot_y, by);
            wv(ox + 60, my + 40, by);
            gnd_sym(ox + 380, by + 2);
        }
        9 => {
            // H-Bridge: B + 4 switches + motor
            let mid_x = ox + 400;
            battery_sym(ox + 60, my + 20);
            wh(ox + 72, my, mid_x - 160);
            node(mid_x - 160, my);
            wv(mid_x - 160, my, my - 80);
            comp(mid_x - 160, my - 120, 60, "S1", C_SWITCH);
            wv(mid_x - 160, my - 154, my - 200);
            wh(mid_x - 160, my - 200, mid_x + 160);
            wv(mid_x + 160, my - 200, my - 154);
            comp(mid_x + 160, my - 120, 60, "S2", C_SWITCH);
            wv(mid_x + 160, my - 80, my);
            wh(mid_x + 160, my, mid_x + 280);
            wv(mid_x + 280, my, by);
            // Bottom rail
            wh(mid_x - 160, my, mid_x - 80);
            wv(mid_x - 160, my, my + 80);
            comp(mid_x - 160, my + 120, 60, "S3", C_SWITCH);
            wv(mid_x - 160, my + 154, by);
            wh(mid_x + 80, my, mid_x + 160);
            wv(mid_x + 160, my, my + 80);
            comp(mid_x + 160, my + 120, 60, "S4", C_SWITCH);
            wv(mid_x + 160, my + 154, by);
            // Motor across middle
            comp(mid_x, my, 80, "MOTOR", 0x001A1AFF);
            wh(mid_x - 80, my, mid_x - 40);
            wh(mid_x + 40, my, mid_x + 80);
            wh(mid_x - 160, by, mid_x + 280);
            wv(ox + 60, my + 40, by);
            gnd_sym(mid_x, by + 2);
        }
        _ => {}
    }
}

struct App {
    idx: usize,
}

impl App {
    fn new() -> Self { App { idx: 0 } }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        text(12, 8, C_TEXT, "Circuit Diagrams");
        text(150, 8, C_HINT, &format!("←→ to browse  {}/{}  Q=quit", self.idx + 1, CIRCUIT_NAMES.len()));

        // Canvas
        let ox = 80i32;
        let oy = 52i32;
        fill(ox, oy, 800, 560, C_CARD);
        border(ox, oy, 800, 560, C_BORDER);

        // Circuit title
        text(ox + 8, oy + 8, C_ORANGE, CIRCUIT_NAMES[self.idx]);
        text(ox + 8, oy + 28, C_HINT, CIRCUIT_DESC[self.idx]);

        draw_circuit(self.idx, ox, oy + 48);

        // Component legend
        let ly = oy + 488;
        text(ox + 8, ly, C_HINT, "Components:");
        fill(ox + 108, ly, 40, 14, C_BATT);   text(ox + 112, ly + 1, C_GREEN, "BATT");
        fill(ox + 168, ly, 40, 14, C_RESIST);  text(ox + 172, ly + 1, C_ORANGE, "R");
        fill(ox + 228, ly, 40, 14, C_CAP);     text(ox + 232, ly + 1, C_SEL, "C");
        fill(ox + 288, ly, 40, 14, C_ELED);    text(ox + 292, ly + 1, C_RED, "LED/D");
        fill(ox + 368, ly, 40, 14, C_SWITCH);  text(ox + 372, ly + 1, C_TEXT, "SW");
        fill(ox + 428, ly, 40, 14, C_IND);     text(ox + 432, ly + 1, C_TEXT, "L");

        fill(0, H - 24, W, 24, C_HEADER);
        text(12, H - 18, C_HINT, "← = previous circuit    → = next circuit");
        flush();
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[C" => { self.idx = (self.idx + 1) % CIRCUIT_NAMES.len(); }
            "\x1b[D" => { self.idx = (self.idx + CIRCUIT_NAMES.len() - 1) % CIRCUIT_NAMES.len(); }
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
