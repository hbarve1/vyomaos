use std::io::{self, BufRead, Write};

const W: u32 = 960;
const H: u32 = 720;
const HEADER_H: u32 = 40;
const STATUS_H: u32 = 24;
const CONTENT_Y: u32 = HEADER_H;
const CONTENT_H: u32 = H - HEADER_H - STATUS_H;
const LEFT_W: u32 = 580;
const LOGO_X: u32 = LEFT_W + 16;
const LINE_H: u32 = 18;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
const C_PURPLE: u32 = 0xBC8CFFFF;
const C_RED: u32    = 0xFF7B72FF;
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

struct Section {
    title: &'static str,
    icon:  &'static str,
    color: u32,
    rows:  &'static [(&'static str, &'static str)],
}

const SECTIONS: &[Section] = &[
    Section {
        title: "Operating System",
        icon:  "OS",
        color: C_ORANGE,
        rows: &[
            ("Name",       "VyomaOS 1.0.0"),
            ("Kernel",     "Linux 5.10 (allnoconfig + virtio/9P/DRM)"),
            ("Runtime",    "Wasmtime 43.0.0 (WASI Preview 2)"),
            ("Supervisor", "Rust PID 1 · static musl · 697 KB"),
            ("Init",       "/init → supervisor (no sysv/systemd)"),
            ("Uptime",     "00:03:42 since boot"),
            ("Boot time",  "< 5 seconds in QEMU"),
        ],
    },
    Section {
        title: "Processor",
        icon:  "CPU",
        color: C_GREEN,
        rows: &[
            ("Architecture", "x86_64"),
            ("Model",        "QEMU Virtual CPU v2.5+"),
            ("Cores",        "1 vCPU"),
            ("Speed",        "1000 MIPS (emulated)"),
            ("Instruction",  "WASI Preview 2 (apps run as WASM)"),
            ("Compiler",     "rustc 1.87 → wasm32-wasip2"),
            ("Seccomp",      "BPF denylist applied to all Wasmtime children"),
        ],
    },
    Section {
        title: "Memory",
        icon:  "MEM",
        color: C_SEL,
        rows: &[
            ("Total RAM",  "256 MB"),
            ("Used",       "~42 MB (supervisor + Wasmtime + apps)"),
            ("Free",       "~214 MB"),
            ("Supervisor", "~2 MB resident"),
            ("Wasmtime",   "~30 MB per instance"),
            ("App avg",    "1–10 KB WASM binary + heap"),
            ("Allocator",  "musl malloc (supervisor) / WASM linear memory (apps)"),
        ],
    },
    Section {
        title: "Display",
        icon:  "DSP",
        color: C_PURPLE,
        rows: &[
            ("Resolution",  "1440 × 900 px"),
            ("Driver",      "virtio-gpu (DRM/KMS)"),
            ("Bit depth",   "32 bpp (BGRA)"),
            ("Framebuffer", "Double-buffered (back → mmap blit on flush)"),
            ("Protocol",    "VYOMA_DRAW: text commands over stdout"),
            ("Font",        "8×16 bitmap (95 printable ASCII chars)"),
            ("Refresh",     "~60 FPS (driven by app ping-pong)"),
        ],
    },
    Section {
        title: "Storage",
        icon:  "DSK",
        color: C_YELLOW,
        rows: &[
            ("/apps",       "tmpfs 18 MB — initramfs (WASM binaries + rootfs)"),
            ("/data",       "ext4 64 MB — virtio-blk persistent data disk"),
            ("/data/logs",  "App stdout logs (ring buffer per app)"),
            ("9P mount",    "virtio-9p → /data (host data/ directory)"),
            ("Format",      "ext4 (mkfs.ext4 at first boot)"),
            ("Persistence", "Survives VM reboots via virtio-blk"),
            ("Compression", "initramfs.cpio.gz (gzip, ~18 MB)"),
        ],
    },
    Section {
        title: "Network",
        icon:  "NET",
        color: C_GREEN,
        rows: &[
            ("Interface",  "virtio-net (e1000 emulated)"),
            ("IP Address", "10.0.0.2 (QEMU user networking)"),
            ("Gateway",    "10.0.0.1 (QEMU host NAT)"),
            ("HTTP Server","localhost:8080 (http-server WASM app)"),
            ("DNS",        "TCP to 8.8.8.8:53 via supervisor IPC"),
            ("Port Forward","host:8080 → guest:8080 (-netdev user)"),
            ("Sockets",    "WASI sockets (network=true apps only)"),
        ],
    },
    Section {
        title: "Security",
        icon:  "SEC",
        color: C_RED,
        rows: &[
            ("Model",      "Capability-based (vyoma.toml per-app)"),
            ("Sandbox",    "WASI Preview 2 import restriction"),
            ("Seccomp",    "BPF denylist on all Wasmtime children"),
            ("Namespaces", "PID + mount (unshare) per app (P27)"),
            ("Integrity",  "SHA-256 verify on wasm_sha256 field (P28)"),
            ("IPC",        "Supervisor-brokered, no direct app-to-app pipes"),
            ("LSM",        "None (capability model replaces SELinux/AppArmor)"),
        ],
    },
];

const LOGO: &[&str] = &[
    "  ____  ____  ____  ____  __  ",
    " / /\\ \\/ /\\ \\/ /\\ \\/ /\\ \\/ /  ",
    "/ /  \\ V /  \\ V /  \\ V /  \\ V /",
    "\\ \\   \\ /    \\ /    \\ /    \\ /  ",
    " \\ \\   V      V      V      V   ",
    "  \\ \\  | VYOMA OS     v1.0 |   ",
    "   \\ \\ | WASM-first   Linux |  ",
    "    \\_\\| Rust PID1  x86_64 |   ",
    "       +---------------------+  ",
];

const LOGO2: &[&str] = &[
    r" __   __                      ",
    r"|  | / /_  ____  _____ _____ ",
    r"|  |/ /\ \/ /  \/ /|  V  \\  ",
    r"|    <  \  / /\  / | |_| |/  ",
    r"|__|\__\ \/_/  \_/  |___,_\  ",
    r"",
    r"  VyomaOS  v1.0.0  x86_64   ",
    r"  WASM-first  Capability OS  ",
    r"  Linux 5.10  Wasmtime 43.0  ",
    r"  Rust PID1   Musl static    ",
];

struct App {
    scroll:  usize,
    status:  String,
}

fn section_line_count(s: &Section) -> u32 {
    2 + s.rows.len() as u32 + 1 // title + rows + gap
}

fn all_lines() -> Vec<(u32, String, u32)> { // (color, text, indent)
    let mut lines = Vec::new();
    for sec in SECTIONS {
        lines.push((sec.color, format!("▸ {}", sec.title), 0));
        lines.push((C_BORDER, "─".repeat(60), 0));
        for (k, v) in sec.rows {
            lines.push((C_HINT, format!("{:<20} {}", k, v), 8));
        }
        lines.push((0, String::new(), 0));
    }
    lines
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "System Information");
    text(240, 12, C_HINT, "VyomaOS 1.0.0 · x86_64 · Linux 5.10 · Wasmtime 43.0.0");

    // Left: info sections
    fill(0, CONTENT_Y, LEFT_W, CONTENT_H, C_BG);

    let lines = all_lines();
    let vis = (CONTENT_H / LINE_H) as usize;
    let scroll = app.scroll.min(lines.len().saturating_sub(vis));
    let start = scroll;
    let end = (start + vis).min(lines.len());

    for (vi, (col, txt, indent)) in lines[start..end].iter().enumerate() {
        let ly = CONTENT_Y + vi as u32 * LINE_H;
        if txt.is_empty() { continue; }
        let c = if *col == 0 { C_HINT } else { *col };
        let x_off = *indent as u32;
        // Truncate to fit
        let max_chars = ((LEFT_W - 16 - x_off) / 8) as usize;
        let shown = if txt.len() > max_chars { &txt[..max_chars] } else { txt.as_str() };
        // Color key differently from value
        if *indent == 8 {
            if let Some(pos) = shown.find("  ") {
                let key = &shown[..pos + 2];
                let val = &shown[pos + 2..];
                text(16 + x_off, ly + 2, C_HINT, key);
                text(16 + x_off + key.len() as u32 * 8, ly + 2, C_TEXT, val);
            } else {
                text(16 + x_off, ly + 2, c, shown);
            }
        } else {
            text(16, ly + 2, c, shown);
        }
    }

    // Scroll indicator
    if lines.len() > vis {
        let track_h = CONTENT_H;
        let thumb_h = (vis as u32 * track_h / lines.len() as u32).max(4);
        let thumb_y = CONTENT_Y + (scroll as u32 * (track_h - thumb_h)) / (lines.len() as u32 - vis as u32).max(1);
        fill(LEFT_W - 4, CONTENT_Y, 4, track_h, C_HEADER);
        fill(LEFT_W - 4, thumb_y, 4, thumb_h, C_HINT);
    }

    // Divider
    fill(LEFT_W, CONTENT_Y, 1, CONTENT_H, C_BORDER);

    // Right: ASCII logo
    fill(LEFT_W + 1, CONTENT_Y, W - LEFT_W - 1, CONTENT_H, C_CARD);

    text(LOGO_X, CONTENT_Y + 12, C_ORANGE, "VyomaOS");
    text(LOGO_X + 72, CONTENT_Y + 12, C_HINT, "v1.0.0");

    for (i, row) in LOGO2.iter().enumerate() {
        let ly = CONTENT_Y + 40 + i as u32 * 14;
        text(LOGO_X, ly, C_SEL, row);
    }

    // Build info box
    let bx = LOGO_X;
    let by = CONTENT_Y + 200;
    let bw = W - LEFT_W - 24;
    let bh = 180u32;
    fill(bx, by, bw, bh, 0x0D1117FF);
    border(bx, by, bw, bh, C_BORDER);
    text(bx + 8, by + 8, C_HINT, "Build");
    fill(bx, by + 24, bw, 1, C_BORDER);

    let build_rows: &[(&str, &str, u32)] = &[
        ("Rust",    "1.87.0 stable",   C_ORANGE),
        ("Target",  "wasm32-wasip2",   C_GREEN),
        ("Kernel",  "5.10.x allnoconfig", C_SEL),
        ("BusyBox", "1.35.0 static",   C_TEXT),
        ("musl",    "1.2.4",           C_TEXT),
        ("Docker",  "vyomaos-builder", C_PURPLE),
        ("Git",     "develop branch",  C_YELLOW),
    ];
    for (i, (k, v, vc)) in build_rows.iter().enumerate() {
        let ry = by + 32 + i as u32 * LINE_H;
        text(bx + 8, ry, C_HINT, &format!("{:<10}", k));
        text(bx + 88, ry, *vc, v);
    }

    // Stats box
    let sx = LOGO_X;
    let sy = by + bh + 8;
    let sw = W - LEFT_W - 24;
    let sh_box = CONTENT_Y + CONTENT_H - sy - 4;
    if sh_box > 40 {
        fill(sx, sy, sw, sh_box, 0x0D1117FF);
        border(sx, sy, sw, sh_box, C_BORDER);
        text(sx + 8, sy + 8, C_HINT, "App Ecosystem");
        fill(sx, sy + 24, sw, 1, C_BORDER);
        text(sx + 8, sy + 32, C_TEXT, "Total apps:    166+");
        text(sx + 8, sy + 50, C_TEXT, "Games:         ~30");
        text(sx + 8, sy + 68, C_TEXT, "Productivity:  ~50");
        text(sx + 8, sy + 86, C_TEXT, "System tools:  ~40");
        text(sx + 8, sy + 104, C_TEXT, "Media & viz:   ~46");
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 6, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App {
        scroll: 0,
        status: String::from("↑↓/PgUp/PgDn=scroll  R=refresh  Ctrl+C=quit"),
    };

    println!("@supervisor: raise system-info");
    let _ = io::stdout().flush();
    draw(&app);

    let lines = all_lines();

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        let vis = (CONTENT_H / LINE_H) as usize;

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => { app.scroll = app.scroll.saturating_sub(1); }
            "\x1b[B" => { app.scroll = (app.scroll + 1).min(lines.len().saturating_sub(vis)); }
            "\x1b[5~" => { app.scroll = app.scroll.saturating_sub(vis); }
            "\x1b[6~" => { app.scroll = (app.scroll + vis).min(lines.len().saturating_sub(vis)); }
            "\x1b[H" => { app.scroll = 0; }
            "\x1b[F" => { app.scroll = lines.len().saturating_sub(vis); }
            "r" | "R" => { app.status = "Refreshed.".to_string(); }
            _ => {}
        }
        draw(&app);
    }
}
