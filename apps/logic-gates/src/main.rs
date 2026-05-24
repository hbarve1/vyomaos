use std::io::{self, BufRead, Write};

const W: i32 = 960;
const H: i32 = 720;
const GW: i32 = 84;
const GH: i32 = 40;

const C_BG:     u32 = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT:   u32 = 0xE6EDF3FF;
const C_HINT:   u32 = 0x6E7681FF;
const C_CARD:   u32 = 0x161B22FF;
const C_GREEN:  u32 = 0x3FB950FF;
const C_RED:    u32 = 0xFF7B72FF;
const C_SEL:    u32 = 0x58A6FFFF;
const C_ORANGE: u32 = 0xFFA657FF;

fn fill(x: i32, y: i32, w: i32, h: i32, c: u32) {
    if w > 0 && h > 0 { println!("VYOMA_DRAW:fill_rect:{},{},{},{},{}", x, y, w, h, c); }
}
fn text(x: i32, y: i32, c: u32, s: &str) {
    if !s.is_empty() { println!("VYOMA_DRAW:draw_text:{},{},{},m,{}", x, y, c, s); }
}
fn flush() {
    println!("VYOMA_DRAW:flush");
    let _ = io::stdout().flush();
}

fn sig_color(v: bool) -> u32 { if v { C_GREEN } else { C_RED } }

fn draw_line(x1: i32, y1: i32, x2: i32, y2: i32, c: u32) {
    // Horizontal then vertical (L-shaped)
    fill(x1.min(x2), y1, (x2 - x1).abs() + 1, 2, c);
    if y1 != y2 {
        let mx = x2;
        fill(mx, y1.min(y2), 2, (y2 - y1).abs() + 2, c);
    }
}

#[derive(Clone, Copy, PartialEq)]
enum GK { And, Or, Not, Xor, Nand, Nor }

impl GK {
    fn label(self) -> &'static str {
        match self { GK::And=>"AND", GK::Or=>"OR", GK::Not=>"NOT", GK::Xor=>"XOR", GK::Nand=>"NAND", GK::Nor=>"NOR" }
    }
    fn eval(self, a: bool, b: bool) -> bool {
        match self { GK::And=>a&&b, GK::Or=>a||b, GK::Not=>!a, GK::Xor=>a^b, GK::Nand=>!(a&&b), GK::Nor=>!(a||b) }
    }
}

// Gate: kind, signal index of in1, in2, output; screen x,y; optional output label
struct Gate { k: GK, i1: usize, i2: usize, out: usize, x: i32, y: i32, lbl: &'static str }

struct Circuit {
    name:     &'static str,
    n_in:     usize,
    in_lbls:  &'static [&'static str],
    in_ys:    &'static [i32],
    gates:    Vec<Gate>,
}

impl Circuit {
    fn propagate(&self, sigs: &mut Vec<bool>) {
        for g in &self.gates {
            sigs[g.out] = g.k.eval(sigs[g.i1], sigs[g.i2]);
        }
    }
}

fn make_circuits() -> Vec<Circuit> {
    vec![
        // 1: Half Adder (A XOR B = Sum, A AND B = Carry)
        Circuit {
            name: "Half Adder", n_in: 2,
            in_lbls: &["A", "B"],
            in_ys:   &[220, 380],
            gates: vec![
                Gate { k: GK::Xor,  i1:0, i2:1, out:2, x:320, y:188, lbl:"Sum"   },
                Gate { k: GK::And,  i1:0, i2:1, out:3, x:320, y:348, lbl:"Carry" },
            ],
        },
        // 2: Majority Gate (A,B,C) → out = (A&B)|(B&C)|(A&C)
        Circuit {
            name: "Majority Gate", n_in: 3,
            in_lbls: &["A", "B", "C"],
            in_ys:   &[150, 300, 450],
            gates: vec![
                Gate { k: GK::And, i1:0, i2:1, out:3, x:220, y:135, lbl:"" },
                Gate { k: GK::And, i1:1, i2:2, out:4, x:220, y:278, lbl:"" },
                Gate { k: GK::And, i1:0, i2:2, out:5, x:220, y:420, lbl:"" },
                Gate { k: GK::Or,  i1:3, i2:4, out:6, x:420, y:200, lbl:"" },
                Gate { k: GK::Or,  i1:6, i2:5, out:7, x:580, y:310, lbl:"Out" },
            ],
        },
        // 3: 2:1 Multiplexer (Sel=0→A, Sel=1→B)
        Circuit {
            name: "2:1 Multiplexer", n_in: 3,
            in_lbls: &["A", "B", "Sel"],
            in_ys:   &[160, 300, 450],
            gates: vec![
                Gate { k: GK::Not, i1:2, i2:2, out:3, x:200, y:420, lbl:"" },   // NOT Sel
                Gate { k: GK::And, i1:0, i2:3, out:4, x:380, y:128, lbl:"" },   // A & !Sel
                Gate { k: GK::And, i1:1, i2:2, out:5, x:380, y:268, lbl:"" },   // B & Sel
                Gate { k: GK::Or,  i1:4, i2:5, out:6, x:560, y:200, lbl:"Out" },
            ],
        },
        // 4: XOR from 4×NAND
        Circuit {
            name: "XOR from NAND", n_in: 2,
            in_lbls: &["A", "B"],
            in_ys:   &[220, 400],
            gates: vec![
                Gate { k: GK::Nand, i1:0, i2:1, out:2, x:220, y:290, lbl:"" },
                Gate { k: GK::Nand, i1:0, i2:2, out:3, x:400, y:188, lbl:"" },
                Gate { k: GK::Nand, i1:1, i2:2, out:4, x:400, y:368, lbl:"" },
                Gate { k: GK::Nand, i1:3, i2:4, out:5, x:570, y:278, lbl:"Out" },
            ],
        },
        // 5: 2-to-4 Decoder
        Circuit {
            name: "2-to-4 Decoder", n_in: 2,
            in_lbls: &["A", "B"],
            in_ys:   &[200, 380],
            gates: vec![
                Gate { k: GK::Not, i1:0, i2:0, out:2, x:180, y:140, lbl:"" },  // A'
                Gate { k: GK::Not, i1:1, i2:1, out:3, x:180, y:320, lbl:"" },  // B'
                Gate { k: GK::And, i1:2, i2:3, out:4, x:370, y: 68, lbl:"D0" },
                Gate { k: GK::And, i1:0, i2:3, out:5, x:370, y:188, lbl:"D1" },
                Gate { k: GK::And, i1:2, i2:1, out:6, x:370, y:308, lbl:"D2" },
                Gate { k: GK::And, i1:0, i2:1, out:7, x:370, y:428, lbl:"D3" },
            ],
        },
    ]
}

struct App {
    circuits: Vec<Circuit>,
    idx:      usize,
    sigs:     Vec<bool>,
}

impl App {
    fn new() -> Self {
        let circuits = make_circuits();
        let n_sigs = circuits[0].n_in + circuits[0].gates.len();
        let mut app = App { circuits, idx: 0, sigs: vec![false; n_sigs.max(16)] };
        app.propagate();
        app
    }

    fn load_circuit(&mut self) {
        let c = &self.circuits[self.idx];
        let n = c.n_in + c.gates.len();
        self.sigs = vec![false; n.max(16)];
        self.propagate();
    }

    fn propagate(&mut self) {
        let sigs = &mut self.sigs;
        for g in &self.circuits[self.idx].gates {
            sigs[g.out] = g.k.eval(sigs[g.i1], sigs[g.i2]);
        }
    }

    fn draw(&self) {
        fill(0, 0, W, H, C_BG);
        fill(0, 0, W, 32, C_HEADER);
        fill(0, H - 28, W, 28, C_HEADER);

        let c = &self.circuits[self.idx];
        text(12, 8, C_TEXT, &format!("Logic Gates — {} ({}/{})", c.name, self.idx + 1, self.circuits.len()));
        text(500, 8, C_HINT, "←→=circuit  1-4=toggle input  Q=quit");

        // Draw input signals
        let in_x = 60i32;
        for (i, (&lbl, &iy)) in c.in_lbls.iter().zip(c.in_ys.iter()).enumerate() {
            let v = self.sigs[i];
            fill(in_x, iy - 12, 30, 24, sig_color(v));
            text(in_x + 4, iy - 6, C_BG, if v { "1" } else { "0" });
            text(in_x + 36, iy - 6, C_TEXT, &format!("[{}] {}", i + 1, lbl));
        }

        // Draw gates and wires
        for g in &c.gates {
            let gx = g.x; let gy = g.y;
            // Gate box
            fill(gx, gy, GW, GH, C_CARD);
            fill(gx, gy, GW, 1, C_BORDER);
            fill(gx, gy + GH - 1, GW, 1, C_BORDER);
            fill(gx, gy, 1, GH, C_BORDER);
            fill(gx + GW - 1, gy, 1, GH, C_BORDER);
            text(gx + 4, gy + 12, C_TEXT, g.k.label());

            // Input signal dots (left side of gate)
            let v1 = self.sigs[g.i1];
            fill(gx - 8, gy + 8, 8, 8, sig_color(v1));
            if g.k != GK::Not {
                let v2 = self.sigs[g.i2];
                fill(gx - 8, gy + GH - 16, 8, 8, sig_color(v2));
            }

            // Output signal dot (right side)
            let vout = self.sigs[g.out];
            fill(gx + GW, gy + GH / 2 - 4, 8, 8, sig_color(vout));

            // Output label
            if !g.lbl.is_empty() {
                let lc = sig_color(vout);
                text(gx + GW + 14, gy + GH / 2 - 6, lc, &format!("{}={}", g.lbl, if vout { 1 } else { 0 }));
            }

            // Wire from in1 source to gate
            let (src_x, src_y) = self.signal_pos(c, g.i1);
            draw_line(src_x, src_y, gx - 8, gy + 8, C_BORDER);

            // Wire from in2 source to gate (skip for NOT)
            if g.k != GK::Not {
                let (src_x2, src_y2) = self.signal_pos(c, g.i2);
                draw_line(src_x2, src_y2, gx - 8, gy + GH - 12, C_BORDER);
            }
        }

        // Truth table hint
        text(12, H - 20, C_HINT, &format!(
            "Inputs: {}",
            c.in_lbls.iter().enumerate()
                .map(|(i, &l)| format!("{}={}", l, if self.sigs[i] { 1 } else { 0 }))
                .collect::<Vec<_>>().join("  ")
        ));

        flush();
    }

    fn signal_pos(&self, c: &Circuit, sig: usize) -> (i32, i32) {
        // If sig is an input, return right edge of input square
        if sig < c.n_in {
            return (90, c.in_ys[sig]);
        }
        // Otherwise it's a gate output — find that gate
        for g in &c.gates {
            if g.out == sig {
                return (g.x + GW + 8, g.y + GH / 2);
            }
        }
        (0, 360)
    }

    fn handle(&mut self, line: &str) {
        match line {
            "\x1b[C" => {
                self.idx = (self.idx + 1) % self.circuits.len();
                self.load_circuit();
            }
            "\x1b[D" => {
                self.idx = (self.idx + self.circuits.len() - 1) % self.circuits.len();
                self.load_circuit();
            }
            "1" => { self.sigs[0] = !self.sigs[0]; self.propagate(); }
            "2" => { if self.circuits[self.idx].n_in > 1 { self.sigs[1] = !self.sigs[1]; self.propagate(); } }
            "3" => { if self.circuits[self.idx].n_in > 2 { self.sigs[2] = !self.sigs[2]; self.propagate(); } }
            "4" => { if self.circuits[self.idx].n_in > 3 { self.sigs[3] = !self.sigs[3]; self.propagate(); } }
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
