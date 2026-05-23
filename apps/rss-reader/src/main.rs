use std::io::{self, BufRead, Write};

const W: u32 = 1280;
const H: u32 = 760;
const HEADER_H: u32 = 40;
const TAB_H: u32 = 28;
const STATUS_H: u32 = 24;
const FEED_W: u32 = 220;
const CONTENT_X: u32 = FEED_W + 1;
const CONTENT_W: u32 = W - FEED_W - 1;
const CONTENT_Y: u32 = HEADER_H + TAB_H;
const CONTENT_H: u32 = H - HEADER_H - TAB_H - STATUS_H;
const LINE_H: u32 = 18;
const DETAIL_SPLIT: u32 = CONTENT_H / 2; // article list takes top half
const CHAR_W: u32 = 8;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_GREEN: u32  = 0x3FB950FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
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

#[derive(Clone)]
struct Article {
    title:   &'static str,
    source:  &'static str,
    date:    &'static str,
    summary: &'static str,
}

const FEED_NAMES: [&str; 3] = ["VyomaOS News", "Dev Blog", "Tech Headlines"];

const ARTICLES_0: &[Article] = &[
    Article { title: "VyomaOS reaches 160 apps milestone", source: "VyomaOS Blog", date: "2026-05-21", summary: "The VyomaOS project has reached a remarkable milestone with over 160 WASM apps now shipping in the default distribution. The apps range from productivity tools to games and system utilities, all running securely under the capability-based model." },
    Article { title: "New display protocol: rect_border and text_wrap", source: "Dev Notes", date: "2026-05-20", summary: "Phase 23 introduced two new VYOMA_DRAW commands: rect_border for drawing hollow rectangles and draw_text_wrap for automatic line-wrapping in TUI panels. These make building rich UIs significantly easier." },
    Article { title: "Photo Editor now supports PPM crop and rotate", source: "Release Notes", date: "2026-05-19", summary: "The photo-editor app (P158) supports crop, resize, rotate 90°, brightness, and contrast adjustments on PPM P6 images stored in /data. Save your results as edited.ppm." },
    Article { title: "IRC client adds simulated multi-channel chat", source: "App Updates", date: "2026-05-18", summary: "The new IRC client app (P157) simulates a three-channel IRC environment with LCG-driven bots posting messages in real-time. Join channels with /join, send messages, and navigate history with arrow keys." },
    Article { title: "File Archiver supports custom .tar format", source: "Dev Blog", date: "2026-05-17", summary: "P155 ships a file archiver with a simple custom archive format: FILE:<name>:<size> header + raw bytes. The two-panel UI lets you manage multiple archives stored in /data." },
    Article { title: "System Logger tracks 200-entry ring buffer", source: "System", date: "2026-05-16", summary: "The syslog app (P156) maintains a rolling 200-entry ring buffer of simulated kernel and app log messages across 7 subsystems and 5 severity levels. Filter by level, pause, export to /data/syslog.txt." },
    Article { title: "VyomaOS kernel stays under 3MB", source: "Kernel Team", date: "2026-05-15", summary: "The Linux kernel for VyomaOS remains under 3MB compressed thanks to allnoconfig plus only the virtio, 9P, DRM, and fbcon drivers. No networking stack, USB, or filesystem drivers beyond 9P." },
    Article { title: "Wasmtime 43.0.0 now the default runtime", source: "Infrastructure", date: "2026-05-14", summary: "VyomaOS has upgraded to Wasmtime 43.0.0 which ships a static musl binary under 700KB. WASI Preview 2 component model is fully supported for all apps." },
    Article { title: "Drawing App v2 adds 16-color palette", source: "App Updates", date: "2026-05-13", summary: "draw2 (P154) ships a 300×200 canvas at 3px per pixel with 16 colors, brush/eraser/fill/line tools, 10-step undo, and PPM save/load. Run-length encoding keeps draw calls under 200 per frame." },
    Article { title: "Spreadsheet v2 supports circular reference detection", source: "App Updates", date: "2026-05-12", summary: "spreadsheet2 (P153) has a 20×15 grid with =SUM/AVG/MIN/MAX/COUNT formulas, cell refs, arithmetic, and safe circular reference detection that shows #CIRC instead of looping forever." },
];

const ARTICLES_1: &[Article] = &[
    Article { title: "Building a WASM OS from scratch: lessons learned", source: "hbarve1", date: "2026-05-21", summary: "After 160+ phases of building VyomaOS, the key insights are: capability-based security requires no filtering layer, WASM bytecode is deterministic across builds, and a minimal kernel is surprisingly easy to maintain once you have the right config." },
    Article { title: "Why ping-pong IPC is the right model for WASM apps", source: "Architecture Notes", date: "2026-05-20", summary: "VyomaOS apps use a simple request-reply IPC: send @supervisor: ping, receive REPLY:pong, redraw. This drives all animation, live data feeds, and UI updates without threads or async runtime." },
    Article { title: "Rust + WASM: the perfect embedded UI stack", source: "hbarve1", date: "2026-05-19", summary: "Using Rust compiled to wasm32-wasip2 for UI apps gives you zero-cost abstractions, no GC pauses, and tiny binaries (1-10KB) while the VYOMA_DRAW protocol handles all rendering via a simple text protocol over stdout." },
    Article { title: "LCG random: good enough for WASM apps", source: "Dev Blog", date: "2026-05-18", summary: "The LCG (Linear Congruential Generator) with multiplier 6364136223846793005 and addend 1442695040888963407 is used throughout VyomaOS apps for simulation, shuffling, and procedural generation without needing std::collections." },
    Article { title: "Hermetic Docker builds: never break the build", source: "CI/CD Notes", date: "2026-05-17", summary: "All VyomaOS compilation runs inside a vyomaos-builder Docker container. This ensures byte-identical outputs across developer machines and CI. The Makefile orchestrates kernel → supervisor → apps → rootfs → QEMU." },
    Article { title: "VYOMA_DRAW protocol design decisions", source: "hbarve1", date: "2026-05-16", summary: "The display protocol is deliberately text-based over stdout so apps can be tested with just cat or a terminal. Commands are fill_rect, draw_text (with size m), rect_border, clear_region, draw_text_wrap, and flush." },
    Article { title: "Phase planning: 160 apps in 160 phases", source: "Roadmap", date: "2026-05-15", summary: "Each VyomaOS phase adds exactly one coherent feature. Two phases per batch, one batch per session. At 2 per session × 5 sessions per day, the OS can gain 10 new apps daily while keeping commits clean and bisectable." },
    Article { title: "PPM P6: the simplest image format for WASM", source: "Media Notes", date: "2026-05-14", summary: "PPM P6 (binary portable pixmap) is a trivial format: magic 'P6', width, height, maxval as ASCII, then raw RGB bytes. No compression, no color spaces, no metadata — perfect for WASM apps that need image I/O without dependencies." },
    Article { title: "seccomp BPF: adding kernel-level sandboxing", source: "Security", date: "2026-05-13", summary: "Phase 8 added a seccomp BPF denylist to all Wasmtime child processes. Combined with capability-based WASI imports and PID namespaces (Phase 27), VyomaOS achieves defense in depth without a separate LSM like SELinux." },
    Article { title: "Static musl builds: the secret to 697KB supervisor", source: "Build System", date: "2026-05-12", summary: "The VyomaOS supervisor is compiled with x86_64-unknown-linux-musl target and strip=true, producing a 697KB static binary with no dynamic library dependencies. PID 1 requires no shared libs to be available at root." },
];

const ARTICLES_2: &[Article] = &[
    Article { title: "WebAssembly Component Model reaches maturity", source: "W3C Blog", date: "2026-05-21", summary: "The WASM Component Model (WASI Preview 2) has reached maturity with broad runtime support. Key features: typed interfaces, capability-based imports, and composable components that can be linked at deploy time." },
    Article { title: "Rust 2024 edition: new async and lifetime rules", source: "Rust Blog", date: "2026-05-20", summary: "The Rust 2024 edition ships improved async trait support, new lifetime capture rules that eliminate many explicit annotations, and a more ergonomic error handling story with try blocks stabilized." },
    Article { title: "Linux 6.x brings RISC-V Vector Extension support", source: "LWN.net", date: "2026-05-19", summary: "Linux 6.x adds production-quality support for the RISC-V Vector Extension, enabling SIMD operations on RISC-V hardware. This paves the way for VyomaOS to potentially target RISC-V boards." },
    Article { title: "Wasmtime 43.0 performance: 40% faster startup", source: "Bytecode Alliance", date: "2026-05-18", summary: "Wasmtime 43.0 delivers a 40% improvement in cold startup time for WASI Preview 2 modules through improved compilation caching and a leaner runtime initialization path. Binary size also dropped 15%." },
    Article { title: "musl libc 1.2.5 release: improved POSIX compliance", source: "musl.libc.org", date: "2026-05-17", summary: "musl libc 1.2.5 ships better POSIX compliance for process management, improved pthread support, and several security fixes. The static binary size remains under 800KB for most configurations." },
    Article { title: "BusyBox 1.36 adds new applets and security fixes", source: "BusyBox", date: "2026-05-16", summary: "BusyBox 1.36 adds several new applets, hardens existing ones against path traversal and buffer overflow, and improves ash shell compatibility. The single binary remains under 2MB for the default config." },
    Article { title: "virtio-gpu: the standard for QEMU display backends", source: "QEMU Blog", date: "2026-05-15", summary: "virtio-gpu is now the recommended display backend for QEMU VMs, offering better performance than VGA emulation and supporting resolution changes via VIRTIO_GPU_CMD_RESOURCE_FLUSH without kernel driver changes." },
    Article { title: "9P protocol: simple network filesystem for VMs", source: "Plan 9 From Bell Labs", date: "2026-05-14", summary: "The 9P (Plan 9 Filesystem Protocol) over virtio-9p is the cleanest way to share a host directory with a QEMU guest. Zero configuration, no NFS dependencies, and good performance for development use cases." },
    Article { title: "DRM/KMS: the right way to drive framebuffers in Linux", source: "DRI Devel", date: "2026-05-13", summary: "Direct Rendering Manager (DRM) with Kernel Mode Setting (KMS) is the modern Linux API for framebuffer access. VyomaOS uses DRM with a simple linear framebuffer layout, avoiding the complexity of Mesa and OpenGL." },
    Article { title: "TOML: the configuration language for modern systems", source: "TOML Spec", date: "2026-05-12", summary: "TOML (Tom's Obvious Minimal Language) is used throughout VyomaOS for app manifests (vyoma.toml), boot configuration (boot.toml), and user settings (/data/settings.toml). Its strict typing prevents silent misconfiguration." },
];

const ALL_ARTICLES: [&[Article]; 3] = [ARTICLES_0, ARTICLES_1, ARTICLES_2];

enum View { List, Detail }

struct App {
    feed:    usize,
    sel:     usize,
    view:    View,
    order:   [Vec<usize>; 3],
    scroll:  [usize; 3],
    seed:    u64,
    status:  String,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            feed: 0,
            sel: 0,
            view: View::List,
            order: [
                (0..10).collect(),
                (0..10).collect(),
                (0..10).collect(),
            ],
            scroll: [0; 3],
            seed: 0xA0C5E_12345678ABu64,
            status: String::from("Tab=switch feed  ↑↓=navigate  Enter=detail  R=refresh  Esc=back  Ctrl+C=quit"),
        };
        a
    }

    fn shuffle(&mut self) {
        for feed in 0..3 {
            let n = ALL_ARTICLES[feed].len();
            for i in (1..n).rev() {
                self.seed = lcg(self.seed);
                let j = (self.seed as usize) % (i + 1);
                self.order[feed].swap(i, j);
            }
        }
        self.status = "Feed refreshed.".to_string();
    }

    fn current_articles(&self) -> Vec<&Article> {
        let articles = ALL_ARTICLES[self.feed];
        self.order[self.feed].iter().map(|&i| &articles[i]).collect()
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "RSS Reader");
    text(140, 12, C_HINT, &format!("— {} articles", ALL_ARTICLES[app.feed].len()));

    // Feed tabs
    fill(0, HEADER_H, W, TAB_H, 0x161B22FF);
    fill(0, HEADER_H + TAB_H - 1, W, 1, C_BORDER);
    for (i, name) in FEED_NAMES.iter().enumerate() {
        let tx = 12 + i as u32 * 280;
        if i == app.feed {
            fill(tx - 4, HEADER_H, 272, TAB_H - 1, C_HEADER);
            text(tx, HEADER_H + 7, C_SEL, name);
        } else {
            text(tx, HEADER_H + 7, C_HINT, name);
        }
    }

    // Left: feed panel (vertical feed list)
    fill(0, CONTENT_Y, FEED_W, CONTENT_H, 0x161B22FF);
    fill(FEED_W, CONTENT_Y, 1, CONTENT_H, C_BORDER);
    text(8, CONTENT_Y + 4, C_HINT, "Feeds");
    fill(0, CONTENT_Y + LINE_H + 2, FEED_W, 1, C_BORDER);
    for (i, name) in FEED_NAMES.iter().enumerate() {
        let fy = CONTENT_Y + LINE_H + 4 + i as u32 * LINE_H * 2;
        let cnt = ALL_ARTICLES[i].len();
        if i == app.feed {
            fill(0, fy, FEED_W, LINE_H, 0x1C2D4EFF);
            text(8, fy + 2, C_SEL, name);
        } else {
            text(8, fy + 2, C_TEXT, name);
        }
        text(8, fy + LINE_H, C_HINT, &format!("  {} articles", cnt));
    }

    // Right: article list + detail
    let articles = app.current_articles();
    let list_lines = (DETAIL_SPLIT / LINE_H) as usize;
    let scroll = app.scroll[app.feed].min(articles.len().saturating_sub(list_lines));
    let start = scroll;
    let end = (start + list_lines).min(articles.len());

    fill(CONTENT_X, CONTENT_Y, CONTENT_W, DETAIL_SPLIT, C_BG);
    fill(CONTENT_X, CONTENT_Y + DETAIL_SPLIT, CONTENT_W, 1, C_BORDER);

    for (vi, art) in articles[start..end].iter().enumerate() {
        let idx = start + vi;
        let ly = CONTENT_Y + vi as u32 * LINE_H;
        let is_sel = idx == app.sel;
        if is_sel {
            fill(CONTENT_X, ly, CONTENT_W, LINE_H, 0x1C2D4EFF);
        }
        let max_title = (CONTENT_W / 8 - 24) as usize;
        let title = if art.title.len() > max_title { &art.title[..max_title] } else { art.title };
        let tc = if is_sel { C_SEL } else { C_TEXT };
        text(CONTENT_X + 8, ly + 2, tc, title);
        let meta = format!("{} · {}", art.source, art.date);
        text(CONTENT_X + CONTENT_W - meta.len() as u32 * 8 - 8, ly + 2, C_HINT, &meta);
    }

    // Detail pane (bottom half)
    let detail_y = CONTENT_Y + DETAIL_SPLIT + 2;
    fill(CONTENT_X, detail_y, CONTENT_W, CONTENT_H - DETAIL_SPLIT - 2, C_BG);

    if app.sel < articles.len() {
        let art = articles[app.sel];
        text(CONTENT_X + 8, detail_y + 4, C_ORANGE, art.title);
        text(CONTENT_X + 8, detail_y + 4 + LINE_H, C_HINT, &format!("{} · {} · {}", art.source, art.date, FEED_NAMES[app.feed]));
        fill(CONTENT_X + 8, detail_y + 4 + LINE_H * 2, CONTENT_W - 16, 1, C_BORDER);

        // Word-wrap summary manually
        let words: Vec<&str> = art.summary.split_whitespace().collect();
        let max_chars = ((CONTENT_W - 24) / 8) as usize;
        let mut line = String::new();
        let mut row = 0u32;
        for word in &words {
            if line.len() + word.len() + 1 > max_chars {
                text(CONTENT_X + 8, detail_y + 8 + LINE_H * 3 + row * LINE_H, C_TEXT, &line);
                line.clear();
                row += 1;
                if detail_y + 8 + LINE_H * 3 + (row + 1) * LINE_H > H - STATUS_H { break; }
            }
            if !line.is_empty() { line.push(' '); }
            line.push_str(word);
        }
        if !line.is_empty() {
            text(CONTENT_X + 8, detail_y + 8 + LINE_H * 3 + row * LINE_H, C_TEXT, &line);
        }
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    text(8, sb_y + 4, C_HINT, &app.status);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise rss-reader");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\t" => {
                app.feed = (app.feed + 1) % FEED_NAMES.len();
                app.sel = 0;
                app.status = format!("Switched to {}", FEED_NAMES[app.feed]);
            }
            "\x1b[A" => {
                if app.sel > 0 { app.sel -= 1; }
                let list_lines = (DETAIL_SPLIT / LINE_H) as usize;
                if app.sel < app.scroll[app.feed] { app.scroll[app.feed] = app.sel; }
            }
            "\x1b[B" => {
                let n = ALL_ARTICLES[app.feed].len();
                if app.sel + 1 < n { app.sel += 1; }
                let list_lines = (DETAIL_SPLIT / LINE_H) as usize;
                if app.sel >= app.scroll[app.feed] + list_lines {
                    app.scroll[app.feed] = app.sel + 1 - list_lines;
                }
            }
            "\x1b[5~" => {
                let n = ALL_ARTICLES[app.feed].len();
                let list_lines = (DETAIL_SPLIT / LINE_H) as usize;
                app.scroll[app.feed] = app.scroll[app.feed].saturating_sub(list_lines);
                app.sel = app.sel.saturating_sub(list_lines).min(n.saturating_sub(1));
            }
            "\x1b[6~" => {
                let n = ALL_ARTICLES[app.feed].len();
                let list_lines = (DETAIL_SPLIT / LINE_H) as usize;
                app.scroll[app.feed] = (app.scroll[app.feed] + list_lines).min(n.saturating_sub(list_lines));
                app.sel = (app.sel + list_lines).min(n.saturating_sub(1));
            }
            "" | "\x1b[C" => {
                app.view = View::Detail;
                if app.sel < ALL_ARTICLES[app.feed].len() {
                    let art = app.current_articles();
                    app.status = format!("Reading: {}", art[app.sel].title);
                }
            }
            "\x1b" => {
                app.view = View::List;
                app.status = "Back to list.".to_string();
            }
            "r" | "R" => {
                app.shuffle();
                app.sel = 0;
                app.scroll = [0; 3];
            }
            _ => {}
        }
        draw(&app);
    }
}
