use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_CARD: u32   = 0x161B22FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_RED: u32    = 0xFF7B72FF;
const C_PURPLE: u32 = 0xBC8CFFFF;

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

// Node IDs: 0 = root, 1-6 = branches, 7-24 = leaves (3 per branch)
// Branch i (1-based) has leaves at: 3*i+4, 3*i+5, 3*i+6 → actually 3*(i-1)+7

fn leaf_base(branch: usize) -> usize { 7 + (branch - 1) * 3 }

// 6 branches + 3 sub-nodes each = 19 nodes total + 1 root = 20 nodes
const TOTAL_NODES: usize = 1 + 6 + 18;

// Branch colors
const BRANCH_COLORS: [u32; 6] = [
    C_SEL, C_GREEN, C_ORANGE, C_YELLOW, C_RED, C_PURPLE,
];

// Branch names
const BRANCH_NAMES: [&str; 6] = [
    "Architecture", "Apps", "Security", "Performance", "UI", "Future",
];

// Sub-node names: [branch][leaf]
const LEAF_NAMES: [[&str; 3]; 6] = [
    ["WASM Runtime",  "Supervisor",    "IPC Broker"],
    ["Terminal",      "File Manager",  "Settings"],
    ["Capabilities",  "Seccomp",       "Namespaces"],
    ["DRM Display",   "WASM Size",     "Boot Time"],
    ["Menu Bar",      "Dock",          "Spotlight"],
    ["GPU Driver",    "Networking",    "App Store"],
];

// Hardcoded positions (cx, cy) for each node
// Root at center (600, 400)
const CX: i32 = 600;
const CY: i32 = 400;

// Branch positions: 6 branches at 60° intervals, radius 200
// Angle 0=right, going clockwise
// cos/sin * 200 (integer approximation):
// 0°: (200,0) → (800,400)
// 60°: (100,173) → (700,573)
// 120°: (-100,173) → (500,573)
// 180°: (-200,0) → (400,400)
// 240°: (-100,-173) → (500,227)
// 300°: (100,-173) → (700,227)
const BRANCH_POS: [(i32, i32); 6] = [
    (800, 400),
    (700, 573),
    (500, 573),
    (400, 400),
    (500, 227),
    (700, 227),
];

// Leaf positions: 3 sub-nodes per branch, radius 120 from branch
// Placed at branch_angle ± 30° and branch_angle
// Using approximate integer math:
// For branch 0 (right, 0°): leaves at 0°±30° from branch = (-30°, 0°, +30°) from (800,400) radius 120
//   -30°: cos=-0.866... err wait, -30° from 0° = 330°: cos=0.866, sin=-0.5 → (+104,-60) → (904,340)
//    0°: cos=1, sin=0 → (+120,0) → (920,400)
//   +30°: cos=0.866, sin=0.5 → (+104,+60) → (904,460)

// Precomputed leaf offsets for each branch [3 leaves][dx,dy]
// Using: for branch at angle A, leaves at A-30°, A°, A+30° relative to branch center, radius 120
// All integer approximations
const LEAF_OFFSETS: [[(i32, i32); 3]; 6] = [
    // Branch 0 at 0°: leaves at -30°, 0°, +30°
    [(104, -60), (120,  0), (104, 60)],
    // Branch 1 at 60°: leaves at 30°, 60°, 90°
    [(104, 60),  (60, 104), (0, 120)],
    // Branch 2 at 120°: leaves at 90°, 120°, 150°
    [(0, 120),   (-60, 104), (-104, 60)],
    // Branch 3 at 180°: leaves at 150°, 180°, 210°
    [(-104, 60), (-120, 0),  (-104, -60)],
    // Branch 4 at 240°: leaves at 210°, 240°, 270°
    [(-104, -60), (-60, -104), (0, -120)],
    // Branch 5 at 300°: leaves at 270°, 300°, 330°
    [(0, -120),  (60, -104), (104, -60)],
];

fn node_pos(id: usize) -> (i32, i32) {
    if id == 0 { return (CX, CY); }
    if id >= 1 && id <= 6 {
        let b = id - 1;
        return BRANCH_POS[b];
    }
    // Leaf
    let leaf_id = id - 7;
    let branch = leaf_id / 3;
    let leaf   = leaf_id % 3;
    let (bx, by) = BRANCH_POS[branch];
    let (dx, dy) = LEAF_OFFSETS[branch][leaf];
    (bx + dx, by + dy)
}

fn node_label(id: usize) -> &'static str {
    if id == 0 { return "VyomaOS"; }
    if id >= 1 && id <= 6 { return BRANCH_NAMES[id - 1]; }
    let leaf_id = id - 7;
    let branch = leaf_id / 3;
    let leaf   = leaf_id % 3;
    LEAF_NAMES[branch][leaf]
}

fn node_color(id: usize) -> u32 {
    if id == 0 { return C_ORANGE; }
    if id >= 1 && id <= 6 { return BRANCH_COLORS[id - 1]; }
    let branch = (id - 7) / 3;
    // Dim version of branch color
    let c = BRANCH_COLORS[branch];
    let r = ((c >> 24) & 0xFF) * 2 / 3;
    let g = ((c >> 16) & 0xFF) * 2 / 3;
    let b = ((c >> 8) & 0xFF) * 2 / 3;
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

fn node_level(id: usize) -> &'static str {
    if id == 0 { "root" }
    else if id <= 6 { "branch" }
    else { "leaf" }
}

fn node_w(id: usize) -> u32 {
    if id == 0 { 100 }
    else if id <= 6 { 110 }
    else { 100 }
}

fn node_h(id: usize) -> u32 {
    if id == 0 { 36 }
    else if id <= 6 { 30 }
    else { 26 }
}

fn draw_connection(from: usize, to: usize, bright: bool) {
    let (x1, y1) = node_pos(from);
    let (x2, y2) = node_pos(to);
    let col = if bright { 0x404858FF } else { C_BORDER };
    // Draw a line using thick single-pixel segments
    // Use Bresenham-style: more vertical or horizontal
    let dx = (x2 - x1).abs();
    let dy = (y2 - y1).abs();
    if dx == 0 && dy == 0 { return; }
    if dx >= dy {
        let steps = dx.max(1);
        for i in 0..=steps {
            let x = x1 + (x2 - x1) * i / steps;
            let y = y1 + (y2 - y1) * i / steps;
            fill(x as u32, y as u32, 2, 2, col);
        }
    } else {
        let steps = dy.max(1);
        for i in 0..=steps {
            let x = x1 + (x2 - x1) * i / steps;
            let y = y1 + (y2 - y1) * i / steps;
            fill(x as u32, y as u32, 2, 2, col);
        }
    }
}

fn draw_node_box(id: usize, selected: bool) {
    let (cx, cy) = node_pos(id);
    let nw = node_w(id);
    let nh = node_h(id);
    let x = (cx as u32).saturating_sub(nw / 2);
    let y = (cy as u32).saturating_sub(nh / 2);
    let col = node_color(id);
    let bg = if selected { 0x1C2D4EFF } else { C_CARD };
    fill(x, y, nw, nh, bg);
    let border_col = if selected { C_SEL } else { col };
    border(x - 1, y - 1, nw + 2, nh + 2, border_col);
    let label = node_label(id);
    let max_c = (nw - 8) as usize / CHAR_W as usize;
    let d = if label.len() > max_c { &label[..max_c] } else { label };
    let ty = y + (nh - 12) / 2;
    let tx = x + (nw - d.len() as u32 * CHAR_W) / 2;
    text(tx, ty, if selected { C_SEL } else { col }, d);
}

fn draw(selected: usize) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Mind Map");
    text(140, 16, C_HINT, "VyomaOS — Architecture, Apps, Security, Performance, UI, Future");
    text(800, 16, C_HINT, "Tab:next  ↑↓:in-branch  R:root  Ctrl+C:exit");

    // Draw all connections first (underneath nodes)
    for b in 1..=6 {
        let bright = selected == 0 || selected == b || (selected >= 7 && (selected - 7) / 3 == b - 1);
        draw_connection(0, b, bright);
        for l in 0..3 {
            let leaf_id = 7 + (b - 1) * 3 + l;
            let leaf_bright = selected == b || selected == leaf_id;
            draw_connection(b, leaf_id, leaf_bright);
        }
    }

    // Draw all nodes
    for id in 0..TOTAL_NODES {
        draw_node_box(id, id == selected);
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let level = node_level(selected);
    let parent_label = if selected == 0 {
        String::new()
    } else if selected <= 6 {
        format!("  |  parent: VyomaOS")
    } else {
        let branch = (selected - 7) / 3;
        format!("  |  parent: {}", BRANCH_NAMES[branch])
    };
    text(8, sb_y + 6, C_HINT, &format!("{}  |  level: {}{}",
        node_label(selected), level, parent_label));
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut selected = 0usize;

    println!("@supervisor: raise mind-map");
    let _ = io::stdout().flush();
    draw(selected);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "r" | "R" => { selected = 0; }
            "\t" => { selected = (selected + 1) % TOTAL_NODES; }
            "\x1b[B" | "\x1b[C" => {
                // Down/Right: go deeper (root→branch, branch→leaf)
                if selected == 0 {
                    selected = 1; // first branch
                } else if selected >= 1 && selected <= 6 {
                    selected = leaf_base(selected); // first leaf of this branch
                }
            }
            "\x1b[A" | "\x1b[D" => {
                // Up/Left: go shallower (leaf→branch, branch→root)
                if selected >= 7 {
                    let branch = (selected - 7) / 3;
                    selected = branch + 1;
                } else if selected >= 1 {
                    selected = 0;
                }
            }
            "\x1b[5~" => {
                // PgUp: previous sibling
                if selected >= 1 && selected <= 6 {
                    if selected > 1 { selected -= 1; }
                } else if selected >= 7 {
                    let leaf_in_branch = (selected - 7) % 3;
                    if leaf_in_branch > 0 { selected -= 1; }
                }
            }
            "\x1b[6~" => {
                // PgDn: next sibling
                if selected >= 1 && selected <= 6 {
                    if selected < 6 { selected += 1; }
                } else if selected >= 7 {
                    let leaf_in_branch = (selected - 7) % 3;
                    if leaf_in_branch < 2 { selected += 1; }
                }
            }
            _ => {}
        }
        draw(selected);
    }
}
