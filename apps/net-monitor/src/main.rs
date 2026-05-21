use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 48;
const STATUS_H: u32 = 28;
const CHAR_W: u32 = 8;

const STATS_W: u32 = 200;
const GRAPH_H: u32 = 180;
const PACKET_LIST_H: u32 = 240;
const GRAPH_Y: u32 = HEADER_H + 4;
const PACKET_Y: u32 = H - STATUS_H - PACKET_LIST_H;
const GRAPH_W: u32 = W - STATS_W - 8;

const GRAPH_SAMPLES: usize = 60;
const MAX_PACKETS: usize = 20;
const LINE_H: u32 = 18;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_RED: u32     = 0xFF7B72FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD: u32    = 0x161B22FF;
const C_YELLOW: u32  = 0xD29922FF;
const C_PURPLE: u32  = 0xBC8CFFFF;
const C_GRID: u32    = 0x1C2128FF;

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

fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

#[derive(Clone, Copy, PartialEq)]
enum Proto { Tcp, Udp, Icmp, Http, Dns }

impl Proto {
    fn name(self) -> &'static str {
        match self { Proto::Tcp=>"TCP", Proto::Udp=>"UDP", Proto::Icmp=>"ICMP", Proto::Http=>"HTTP", Proto::Dns=>"DNS" }
    }
    fn color(self) -> u32 {
        match self { Proto::Tcp=>C_SEL, Proto::Udp=>C_GREEN, Proto::Icmp=>C_YELLOW, Proto::Http=>C_ORANGE, Proto::Dns=>C_PURPLE }
    }
    fn from_idx(i: u64) -> Proto {
        match i % 5 { 0=>Proto::Tcp, 1=>Proto::Udp, 2=>Proto::Icmp, 3=>Proto::Http, _=>Proto::Dns }
    }
}

#[derive(Clone)]
struct Packet {
    proto:    Proto,
    src_ip:   [u8; 4],
    dst_ip:   [u8; 4],
    src_port: u16,
    dst_port: u16,
    size:     u32, // bytes
}

struct Monitor {
    packets:      Vec<Packet>,
    throughput:   [u32; GRAPH_SAMPLES], // KB/s per sample
    sample_idx:   usize,
    total_bytes:  u64,
    total_pkts:   u64,
    tick:         u64,
    seed:         u64,
    paused:       bool,
    filter:       [bool; 5], // TCP UDP ICMP HTTP DNS
    proto_counts: [u64; 5],
}

impl Monitor {
    fn new() -> Self {
        Monitor {
            packets: Vec::new(),
            throughput: [0; GRAPH_SAMPLES],
            sample_idx: 0,
            total_bytes: 0,
            total_pkts: 0,
            tick: 0,
            seed: 0xC0DE_ABCDEF123456,
            paused: false,
            filter: [true; 5],
            proto_counts: [0; 5],
        }
    }

    fn gen_packet(&mut self) -> Packet {
        self.seed = lcg(self.seed);
        let proto = Proto::from_idx(self.seed);

        self.seed = lcg(self.seed);
        let src_ip = [10, 0, 0, (self.seed % 254 + 1) as u8];
        self.seed = lcg(self.seed);
        let dst_ip = match proto {
            Proto::Http | Proto::Dns => [8, 8, (self.seed % 8 + 4) as u8, (self.seed % 254 + 1) as u8],
            _ => [192, 168, 1, (self.seed % 10 + 1) as u8],
        };
        self.seed = lcg(self.seed);
        let src_port = (self.seed % 60000 + 1024) as u16;
        let dst_port = match proto {
            Proto::Http  => 80,
            Proto::Dns   => 53,
            Proto::Tcp   => 443,
            Proto::Udp   => 5353,
            Proto::Icmp  => 0,
        };
        self.seed = lcg(self.seed);
        let size = match proto {
            Proto::Icmp => (self.seed % 56 + 28) as u32,
            Proto::Dns  => (self.seed % 200 + 50) as u32,
            Proto::Http => (self.seed % 1400 + 200) as u32,
            _           => (self.seed % 1460 + 40) as u32,
        };
        Packet { proto, src_ip, dst_ip, src_port, dst_port, size }
    }

    fn tick(&mut self) {
        if self.paused { return; }
        self.tick += 1;
        self.seed = lcg(self.seed);
        let n = (self.seed % 4 + 1) as usize; // 1-4 packets per tick
        let mut tick_bytes = 0u32;
        for _ in 0..n {
            let pkt = self.gen_packet();
            tick_bytes += pkt.size;
            self.total_bytes += pkt.size as u64;
            self.total_pkts += 1;
            let pi = pkt.proto as usize;
            self.proto_counts[pi] += 1;
            if self.packets.len() >= MAX_PACKETS { self.packets.remove(0); }
            self.packets.push(pkt);
        }
        // Update throughput graph every 4 ticks
        if self.tick % 4 == 0 {
            self.throughput[self.sample_idx] = tick_bytes / 4; // rough KB/s
            self.sample_idx = (self.sample_idx + 1) % GRAPH_SAMPLES;
        }
    }

    fn reset(&mut self) {
        self.packets.clear();
        self.throughput = [0; GRAPH_SAMPLES];
        self.total_bytes = 0;
        self.total_pkts = 0;
        self.proto_counts = [0; 5];
        self.sample_idx = 0;
    }
}

fn fmt_ip(ip: [u8; 4]) -> String {
    format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3])
}

fn fmt_bytes(b: u64) -> String {
    if b < 1024 { format!("{}B", b) }
    else if b < 1024*1024 { format!("{}.{}KB", b/1024, (b%1024)*10/1024) }
    else { format!("{}.{}MB", b/(1024*1024), (b%(1024*1024))*10/(1024*1024)) }
}

fn draw(m: &Monitor) {
    fill(0, 0, W, H, C_BG);
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 16, C_ORANGE, "Network Monitor");
    let status = if m.paused { "PAUSED" } else { "LIVE" };
    let sc = if m.paused { C_RED } else { C_GREEN };
    text(220, 16, sc, status);
    text(310, 16, C_HINT, "P:pause  R:reset  t/u/i/h/d:filter  Ctrl+C:exit");

    // ── Throughput graph ──────────────────────────────────────────────────
    fill(0, GRAPH_Y, GRAPH_W, GRAPH_H + 8, C_CARD);
    text(8, GRAPH_Y + 2, C_HINT, "Throughput (bytes/tick):");

    let graph_max = m.throughput.iter().copied().max().unwrap_or(1).max(100);
    let graph_top = GRAPH_Y + 20;
    let graph_bottom = GRAPH_Y + GRAPH_H;
    let sample_w = GRAPH_W / GRAPH_SAMPLES as u32;

    // Grid lines
    for g in 0..=4 {
        let gy = graph_top + (GRAPH_H - 20) * g / 4;
        fill(0, gy, GRAPH_W, 1, C_GRID);
        let label = format!("{}", graph_max * (4 - g) / 4);
        text(2, gy - 6, C_HINT, &label);
    }

    // Bars
    for i in 0..GRAPH_SAMPLES {
        let idx = (m.sample_idx + i) % GRAPH_SAMPLES;
        let val = m.throughput[idx];
        let bar_h = ((val as u64 * (GRAPH_H - 22) as u64) / graph_max as u64) as u32;
        if bar_h == 0 { continue; }
        let bx = i as u32 * sample_w;
        let by = graph_bottom - bar_h;
        fill(bx, by, sample_w.saturating_sub(1).max(1), bar_h, C_SEL);
    }
    fill(0, graph_bottom, GRAPH_W, 1, C_BORDER);

    // ── Stats panel ───────────────────────────────────────────────────────
    let sx = GRAPH_W + 8;
    fill(sx, GRAPH_Y, STATS_W - 8, H - HEADER_H - STATUS_H, C_CARD);
    text(sx + 8, GRAPH_Y + 4, C_HINT, "Statistics:");
    text(sx + 8, GRAPH_Y + 22, C_TEXT, &format!("Total pkts: {}", m.total_pkts));
    text(sx + 8, GRAPH_Y + 40, C_TEXT, &format!("Total data: {}", fmt_bytes(m.total_bytes)));
    text(sx + 8, GRAPH_Y + 58, C_TEXT, &format!("Tick: {}", m.tick));

    text(sx + 8, GRAPH_Y + 84, C_HINT, "By protocol:");
    let protos = [Proto::Tcp, Proto::Udp, Proto::Icmp, Proto::Http, Proto::Dns];
    for (i, proto) in protos.iter().enumerate() {
        let py = GRAPH_Y + 100 + i as u32 * 22;
        let pi = *proto as usize;
        let enabled = m.filter[pi];
        let col = if enabled { proto.color() } else { C_HINT };
        text(sx + 8, py, col, &format!("{}: {}", proto.name(), m.proto_counts[pi]));
    }

    text(sx + 8, GRAPH_Y + 216, C_HINT, "Filter (t/u/i/h/d):");
    for (i, proto) in protos.iter().enumerate() {
        let px = sx + 8 + i as u32 * 36;
        let col = if m.filter[*proto as usize] { proto.color() } else { C_HINT };
        text(px, GRAPH_Y + 234, col, proto.name());
    }

    // ── Packet list ───────────────────────────────────────────────────────
    fill(0, PACKET_Y - 20, W, 20, C_HEADER);
    fill(0, PACKET_Y - 20, W, 1, C_BORDER);
    text(8, PACKET_Y - 14, C_HINT, "Recent Packets:");

    let visible_pkts = (PACKET_LIST_H / LINE_H) as usize;
    let start = m.packets.len().saturating_sub(visible_pkts);
    for (i, pkt) in m.packets[start..].iter().enumerate() {
        if !m.filter[pkt.proto as usize] { continue; }
        let py = PACKET_Y + i as u32 * LINE_H + 2;
        let src = fmt_ip(pkt.src_ip);
        let dst = fmt_ip(pkt.dst_ip);
        let port_str = if pkt.dst_port > 0 { format!(":{}", pkt.dst_port) } else { String::new() };
        let line = format!("{:<4} {}:{} → {}{}  {}",
            pkt.proto.name(), src, pkt.src_port, dst, port_str, fmt_bytes(pkt.size as u64));
        let max_c = (GRAPH_W - 8) as usize / CHAR_W as usize;
        let d = if line.len() > max_c { &line[..max_c] } else { &line };
        text(8, py, pkt.proto.color(), d);
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &format!("{} packets  {}  uptime: {} ticks",
        m.total_pkts, fmt_bytes(m.total_bytes), m.tick));
    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut mon = Monitor::new();

    println!("@supervisor: raise net-monitor");
    let _ = io::stdout().flush();
    println!("@supervisor: ping");
    let _ = io::stdout().flush();
    draw(&mon);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        if raw.starts_with("REPLY:") {
            mon.tick();
            println!("@supervisor: ping");
            let _ = io::stdout().flush();
            draw(&mon);
            continue;
        }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "p" | "P" => { mon.paused = !mon.paused; }
            "r" | "R" => { mon.reset(); }
            "c" | "C" => { mon.packets.clear(); }
            "t" => { mon.filter[0] = !mon.filter[0]; }
            "u" => { mon.filter[1] = !mon.filter[1]; }
            "i" => { mon.filter[2] = !mon.filter[2]; }
            "h" => { mon.filter[3] = !mon.filter[3]; }
            "d" => { mon.filter[4] = !mon.filter[4]; }
            _ => {}
        }
        draw(&mon);
    }
}
