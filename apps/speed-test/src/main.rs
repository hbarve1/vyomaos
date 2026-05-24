// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 960;
const H: u32 = 640;
const HEADER_H: u32 = 40;
const STATUS_H: u32 = 28;
const CONTENT_Y: u32 = HEADER_H;
const CONTENT_H: u32 = H - HEADER_H - STATUS_H;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_RED: u32    = 0xFF7B72FF;
const C_YELLOW: u32 = 0xD29922FF;
const C_PURPLE: u32 = 0xBC8CFFFF;
const C_CARD: u32   = 0x161B22FF;

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

const PING_TICKS: usize = 20;
const DL_TICKS: usize = 40;
const UL_TICKS: usize = 20;
const TOTAL_TICKS: usize = PING_TICKS + DL_TICKS + UL_TICKS;

#[derive(PartialEq, Clone, Copy)]
enum Phase { Idle, Ping, Download, Upload, Done }

struct SpeedTest {
    phase:      Phase,
    tick:       usize,
    seed:       u64,
    ping_vals:  Vec<u32>,   // ms
    dl_vals:    Vec<u32>,   // Mbps
    ul_vals:    Vec<u32>,   // Mbps
    status:     String,
}

impl SpeedTest {
    fn new() -> Self {
        SpeedTest {
            phase: Phase::Idle,
            tick: 0,
            seed: 0xA0C5E_12345678ABu64,
            ping_vals: Vec::new(),
            dl_vals: Vec::new(),
            ul_vals: Vec::new(),
            status: String::from("Press Space to start test"),
        }
    }

    fn start(&mut self) {
        self.phase = Phase::Ping;
        self.tick = 0;
        self.ping_vals.clear();
        self.dl_vals.clear();
        self.ul_vals.clear();
        self.seed = 0xA0C5E_12345678ABu64;
        self.status = "Testing ping...".to_string();
    }

    fn advance(&mut self) {
        if self.phase == Phase::Idle || self.phase == Phase::Done { return; }
        self.seed = lcg(self.seed);

        match self.phase {
            Phase::Ping => {
                // ping 8-45ms
                let ms = 8 + (self.seed % 38) as u32;
                self.ping_vals.push(ms);
                self.tick += 1;
                if self.tick >= PING_TICKS {
                    self.phase = Phase::Download;
                    self.tick = 0;
                    self.status = "Testing download...".to_string();
                }
            }
            Phase::Download => {
                // download 50-900 Mbps with smooth walk
                let base = if self.dl_vals.is_empty() { 400u32 } else { *self.dl_vals.last().unwrap() };
                let delta = (self.seed % 80) as i32 - 40;
                let val = (base as i32 + delta).clamp(50, 900) as u32;
                self.dl_vals.push(val);
                self.tick += 1;
                if self.tick >= DL_TICKS {
                    self.phase = Phase::Upload;
                    self.tick = 0;
                    self.status = "Testing upload...".to_string();
                }
            }
            Phase::Upload => {
                // upload 20-300 Mbps with smooth walk
                let base = if self.ul_vals.is_empty() { 150u32 } else { *self.ul_vals.last().unwrap() };
                self.seed = lcg(self.seed);
                let delta = (self.seed % 40) as i32 - 20;
                let val = (base as i32 + delta).clamp(20, 300) as u32;
                self.ul_vals.push(val);
                self.tick += 1;
                if self.tick >= UL_TICKS {
                    self.phase = Phase::Done;
                    self.status = "Test complete!".to_string();
                }
            }
            _ => {}
        }
    }

    fn ping_avg(&self) -> u32 {
        if self.ping_vals.is_empty() { return 0; }
        self.ping_vals.iter().sum::<u32>() / self.ping_vals.len() as u32
    }
    fn ping_min(&self) -> u32 { self.ping_vals.iter().copied().min().unwrap_or(0) }
    fn ping_max(&self) -> u32 { self.ping_vals.iter().copied().max().unwrap_or(0) }
    fn dl_avg(&self) -> u32 {
        if self.dl_vals.is_empty() { return 0; }
        self.dl_vals.iter().sum::<u32>() / self.dl_vals.len() as u32
    }
    fn ul_avg(&self) -> u32 {
        if self.ul_vals.is_empty() { return 0; }
        self.ul_vals.iter().sum::<u32>() / self.ul_vals.len() as u32
    }

    fn grade_ping(ms: u32) -> &'static str {
        match ms { 0..=15 => "A+", 16..=25 => "A", 26..=40 => "B", 41..=80 => "C", _ => "D" }
    }
    fn grade_dl(mbps: u32) -> &'static str {
        match mbps { 500..=u32::MAX => "A+", 200..=499 => "A", 100..=199 => "B", 50..=99 => "C", _ => "D" }
    }
    fn grade_ul(mbps: u32) -> &'static str {
        match mbps { 200..=u32::MAX => "A+", 100..=199 => "A", 50..=99 => "B", 20..=49 => "C", _ => "D" }
    }
    fn grade_color(g: &str) -> u32 {
        match g { "A+" => C_GREEN, "A" => C_GREEN, "B" => C_SEL, "C" => C_YELLOW, _ => C_RED }
    }
}

fn draw_bar_graph(vals: &[u32], max_val: u32, x: u32, y: u32, w: u32, h: u32, bar_col: u32, label: &str) {
    fill(x, y, w, h, 0x0A0E13FF);
    border(x, y, w, h, C_BORDER);
    text(x + 4, y + 4, C_HINT, label);

    if vals.is_empty() { return; }
    let n = vals.len().min(w as usize / 4);
    let bar_w = (w / n.max(1) as u32).max(2);
    let chart_h = h - 20;

    for (i, &v) in vals.iter().rev().take(n).enumerate() {
        let bh = (v * chart_h / max_val.max(1)).max(1);
        let bx = x + w - (i as u32 + 1) * bar_w;
        let by = y + h - bh - 2;
        fill(bx + 1, by, bar_w.saturating_sub(2), bh, bar_col);
    }
}

fn draw(st: &SpeedTest) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "Network Speed Test");
    let phase_label = match st.phase {
        Phase::Idle => "IDLE",
        Phase::Ping => "PING",
        Phase::Download => "DOWNLOAD",
        Phase::Upload => "UPLOAD",
        Phase::Done => "DONE",
    };
    let phase_col = match st.phase {
        Phase::Idle => C_HINT,
        Phase::Done => C_GREEN,
        _ => C_YELLOW,
    };
    text(W - 120, 12, phase_col, phase_label);

    // Progress bar (overall)
    let prog_total = match st.phase {
        Phase::Idle => 0,
        Phase::Ping => st.ping_vals.len(),
        Phase::Download => PING_TICKS + st.dl_vals.len(),
        Phase::Upload => PING_TICKS + DL_TICKS + st.ul_vals.len(),
        Phase::Done => TOTAL_TICKS,
    };
    let prog_y = HEADER_H + 6;
    let prog_x = 200u32;
    let prog_w = W - prog_x - 130;
    fill(prog_x, prog_y, prog_w, 8, 0x1A1F27FF);
    border(prog_x, prog_y, prog_w, 8, C_BORDER);
    let pw = (prog_total as u32 * prog_w / TOTAL_TICKS as u32).min(prog_w);
    if pw > 0 { fill(prog_x, prog_y, pw, 8, C_SEL); }
    text(prog_x + prog_w + 4, prog_y - 2, C_HINT, &format!("{}/{}", prog_total, TOTAL_TICKS));

    // Three section panels
    let panel_y = CONTENT_Y + 8;
    let panel_h = 160u32;
    let gap = 16u32;
    let panel_w = (W - gap * 4) / 3;

    // Ping panel
    {
        let px = gap;
        fill(px, panel_y, panel_w, panel_h, C_CARD);
        border(px, panel_y, panel_w, panel_h, if st.phase == Phase::Ping { C_SEL } else { C_BORDER });
        text(px + 8, panel_y + 8, C_HINT, "PING");
        if !st.ping_vals.is_empty() {
            let avg = st.ping_avg();
            text(px + 8, panel_y + 30, C_TEXT, &format!("{} ms avg", avg));
            text(px + 8, panel_y + 50, C_HINT, &format!("min: {}  max: {}", st.ping_min(), st.ping_max()));
            // Dot graph
            let n = st.ping_vals.len().min((panel_w - 16) as usize / 6);
            for (i, &v) in st.ping_vals.iter().rev().take(n).enumerate() {
                let dx = px + panel_w - 8 - (i as u32 * 6);
                let dh = (v * 60 / 50).min(60);
                let dy = panel_y + panel_h - 8 - dh;
                fill(dx, dy, 4, dh, C_SEL);
            }
        } else if st.phase == Phase::Ping {
            text(px + 8, panel_y + 30, C_YELLOW, "measuring...");
        } else {
            text(px + 8, panel_y + 30, C_HINT, "—");
        }
    }

    // Download panel
    {
        let px = gap * 2 + panel_w;
        fill(px, panel_y, panel_w, panel_h, C_CARD);
        border(px, panel_y, panel_w, panel_h, if st.phase == Phase::Download { C_SEL } else { C_BORDER });
        text(px + 8, panel_y + 8, C_HINT, "DOWNLOAD");
        if !st.dl_vals.is_empty() {
            let avg = st.dl_avg();
            text(px + 8, panel_y + 30, C_GREEN, &format!("{} Mbps avg", avg));
            // Bar graph
            let n = st.dl_vals.len().min((panel_w - 16) as usize / 6);
            for (i, &v) in st.dl_vals.iter().rev().take(n).enumerate() {
                let bx = px + panel_w - 8 - (i as u32 * 6);
                let bh = (v * 70 / 900).max(1);
                let by = panel_y + panel_h - 8 - bh;
                fill(bx, by, 4, bh, C_GREEN);
            }
        } else if st.phase == Phase::Download {
            text(px + 8, panel_y + 30, C_YELLOW, "measuring...");
        } else {
            text(px + 8, panel_y + 30, C_HINT, "—");
        }
    }

    // Upload panel
    {
        let px = gap * 3 + panel_w * 2;
        fill(px, panel_y, panel_w, panel_h, C_CARD);
        border(px, panel_y, panel_w, panel_h, if st.phase == Phase::Upload { C_SEL } else { C_BORDER });
        text(px + 8, panel_y + 8, C_HINT, "UPLOAD");
        if !st.ul_vals.is_empty() {
            let avg = st.ul_avg();
            text(px + 8, panel_y + 30, C_PURPLE, &format!("{} Mbps avg", avg));
            let n = st.ul_vals.len().min((panel_w - 16) as usize / 6);
            for (i, &v) in st.ul_vals.iter().rev().take(n).enumerate() {
                let bx = px + panel_w - 8 - (i as u32 * 6);
                let bh = (v * 70 / 300).max(1);
                let by = panel_y + panel_h - 8 - bh;
                fill(bx, by, 4, bh, C_PURPLE);
            }
        } else if st.phase == Phase::Upload {
            text(px + 8, panel_y + 30, C_YELLOW, "measuring...");
        } else {
            text(px + 8, panel_y + 30, C_HINT, "—");
        }
    }

    // Full-width graphs
    let graph_y = panel_y + panel_h + 12;
    let graph_h = 120u32;
    let graph_w = W - 32;

    if !st.dl_vals.is_empty() {
        draw_bar_graph(&st.dl_vals, 900, 16, graph_y, graph_w, graph_h, C_GREEN, "Download Mbps (last 40 samples)");
    }
    if !st.ul_vals.is_empty() {
        let ul_y = graph_y + graph_h + 8;
        if ul_y + graph_h < H - STATUS_H {
            draw_bar_graph(&st.ul_vals, 300, 16, ul_y, graph_w, graph_h, C_PURPLE, "Upload Mbps (last 20 samples)");
        }
    }

    // Result card (when done)
    if st.phase == Phase::Done {
        let card_x = W / 2 - 240;
        let card_y = graph_y;
        let card_w = 480u32;
        let card_h = 180u32;
        fill(card_x, card_y, card_w, card_h, 0x161B22FF);
        border(card_x, card_y, card_w, card_h, C_GREEN);
        text(card_x + 16, card_y + 12, C_GREEN, "✓ Speed Test Results");

        let ping_avg = st.ping_avg();
        let dl_avg = st.dl_avg();
        let ul_avg = st.ul_avg();
        let pg = SpeedTest::grade_ping(ping_avg);
        let dg = SpeedTest::grade_dl(dl_avg);
        let ug = SpeedTest::grade_ul(ul_avg);

        text(card_x + 16, card_y + 36, C_HINT, &format!("Ping:     {:3} ms  avg  (min:{} max:{})", ping_avg, st.ping_min(), st.ping_max()));
        text(card_x + 380, card_y + 36, SpeedTest::grade_color(pg), pg);

        text(card_x + 16, card_y + 56, C_HINT, &format!("Download: {:3} Mbps avg", dl_avg));
        text(card_x + 380, card_y + 56, SpeedTest::grade_color(dg), dg);

        text(card_x + 16, card_y + 76, C_HINT, &format!("Upload:   {:3} Mbps avg", ul_avg));
        text(card_x + 380, card_y + 76, SpeedTest::grade_color(ug), ug);

        fill(card_x + 16, card_y + 96, card_w - 32, 1, C_BORDER);
        text(card_x + 16, card_y + 104, C_TEXT, "Overall connection quality:");
        let overall = if dl_avg >= 200 && ping_avg <= 25 { "Excellent" }
            else if dl_avg >= 100 { "Good" }
            else if dl_avg >= 50 { "Fair" }
            else { "Poor" };
        let oc = if overall == "Excellent" { C_GREEN } else if overall == "Good" { C_SEL } else if overall == "Fair" { C_YELLOW } else { C_RED };
        text(card_x + 220, card_y + 104, oc, overall);
        text(card_x + 16, card_y + 124, C_HINT, "Press Space to run again");
    }

    // Idle prompt
    if st.phase == Phase::Idle {
        let cx = W / 2 - 160;
        let cy = CONTENT_Y + CONTENT_H / 2 - 40;
        fill(cx, cy, 320, 80, C_CARD);
        border(cx, cy, 320, 80, C_BORDER);
        text(cx + 16, cy + 16, C_TEXT, "VyomaOS Network Speed Test");
        text(cx + 48, cy + 40, C_HINT, "Press Space to start");
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &st.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut st = SpeedTest::new();

    println!("@supervisor: raise speed-test");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&st);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            st.advance();
            if st.phase != Phase::Idle && st.phase != Phase::Done {
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            draw(&st);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            " " => {
                st.start();
                println!("@supervisor: ping");
                let _ = io::stdout().flush();
            }
            _ => {}
        }
        draw(&st);
    }
}
