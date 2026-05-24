// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1200;
const H: u32 = 760;
const HEADER_H: u32 = 40;
const STATUS_H: u32 = 24;
const TOC_W: u32 = 200;
const READ_X: u32 = TOC_W + 1;
const READ_W: u32 = W - TOC_W - 1;
const CONTENT_Y: u32 = HEADER_H;
const CONTENT_H: u32 = H - HEADER_H - STATUS_H;
const LINE_H: u32 = 20;
const CHAR_W: u32 = 8;
const READ_CHARS: usize = ((READ_W - 32) / CHAR_W) as usize;

const C_BG: u32     = 0x0D1117FF;
const C_HEADER: u32 = 0x21262DFF;
const C_BORDER: u32 = 0x30363DFF;
const C_TEXT: u32   = 0xE6EDF3FF;
const C_HINT: u32   = 0x6E7681FF;
const C_ORANGE: u32 = 0xFFA657FF;
const C_SEL: u32    = 0x58A6FFFF;
const C_YELLOW: u32 = 0xD29922FF;
const C_GREEN: u32  = 0x3FB950FF;
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

struct Chapter {
    title: &'static str,
    paragraphs: &'static [&'static str],
}

const CHAPTERS: &[Chapter] = &[
    Chapter {
        title: "1. Introduction to VyomaOS",
        paragraphs: &[
            "VyomaOS is a WASM-first operating system with the long-term goal of becoming a lightweight but fully capable general-purpose OS built from the ground up on a capability-secure WebAssembly foundation.",
            "The core architecture is simple: every application is a wasm32-wasip2 binary executed by Wasmtime under a Rust PID 1 supervisor. The Linux kernel handles hardware only. Capabilities such as filesystem, network, display, and stdio are declared per-app in a manifest and enforced at runtime.",
            "The system stack from bottom to top is: Linux 5.10 kernel with allnoconfig (2.3 MB), then the Rust supervisor (PID 1, 697 KB static musl), then the Wasmtime runtime (WASI Preview 2), and finally the WASM apps themselves (1-10 KB each).",
            "VyomaOS boots in QEMU under 5 seconds to a Rust supervisor running multiple concurrent WASM apps with a live GUI dashboard, interactive shell, HTTP server, and real-time keyboard input.",
            "The project is organized into phased milestones. Each phase adds one coherent feature: a new supervisor capability, a new VYOMA_DRAW command, or a new WASM application. The phased approach keeps commits clean and bisectable.",
            "All compilation runs inside a hermetic Docker container. The Makefile orchestrates: kernel build, supervisor build, WASM app builds, rootfs creation, and QEMU boot. This ensures byte-identical outputs across all developer machines.",
            "The build artifacts are: out/bzImage (Linux kernel, 2.3 MB), out/initramfs.cpio.gz (compressed rootfs, 18 MB with Wasmtime plus BusyBox plus supervisor), and out/disk.img (ext4 data disk, 64 MB, created once and persisting across reboots).",
            "Applications are launched on demand via the interactive shell. The shell can run any app by name, focus an existing window, resize it, or send it supervisor commands. Boot-time apps are declared in boot.toml; the rest launch on demand.",
            "VyomaOS is not a microkernel in the traditional sense. The Linux kernel still handles hardware interrupts, memory management, and process scheduling. What VyomaOS provides is a capability-secure application layer on top of a minimal Linux base.",
            "The long-term vision is a macOS-like desktop experience: Menu Bar, Dock, Spotlight, Mission Control, Finder, and Notification Center — all implemented as WASM apps communicating through the supervisor IPC broker.",
        ],
    },
    Chapter {
        title: "2. WASM Security Model",
        paragraphs: &[
            "The VyomaOS security model is capability-based by design. An app can only access resources it explicitly declares in its vyoma.toml manifest. If an app does not declare network = true, there is no network interface available to it — not filtered, simply not wired up.",
            "Each WASM app runs inside a Wasmtime instance. Wasmtime enforces the WASI interface boundary: the only syscalls available to the app are the ones imported from the WASI preview2 interface, and only those imports that the supervisor provides.",
            "The supervisor reads each app's vyoma.toml at launch time and constructs a Wasmtime configuration that only wires up the declared capability imports. An app that declares filesystem = true gets a WASI filesystem mount at /data; one that does not gets no filesystem import at all.",
            "On top of WASI capability restriction, VyomaOS applies a seccomp BPF denylist to all Wasmtime child processes. This is defense in depth: even if Wasmtime had a bug that allowed an escape from the WASI sandbox, the seccomp filter would block the malicious syscall.",
            "Phase 27 added PID and mount namespace isolation using Linux's unshare(CLONE_NEWNS | CLONE_NEWPID) in a pre_exec hook. Each Wasmtime child runs in its own PID namespace, so it cannot see or signal other processes.",
            "The signed bundle system (Phase 28) adds integrity verification. An app's vyoma.toml can include a wasm_sha256 field with the expected SHA-256 hash of the WASM binary. The supervisor verifies the hash before spawning Wasmtime and rejects mismatches.",
            "VyomaOS does not use AppArmor, SELinux, or any other mandatory access control framework. The capability model, seccomp filter, and namespace isolation provide sufficient security for the target use case without the complexity of a full MAC system.",
            "The IPC broker adds a layer of communication security. Apps cannot send arbitrary signals or shared memory to each other. All inter-app communication goes through the supervisor's IPC broker, which validates message format and routes to the correct destination.",
            "WASM bytecode is deterministic across builds and hosts. Unlike ELF binaries which vary by libc and architecture, a wasm32-wasip2 binary is byte-identical across all platforms that support the target. This enables reproducible deployments and hash-based integrity checks.",
            "Future security improvements include: per-app memory limits enforced by Wasmtime fuel and memory quotas, rate limiting on IPC messages, and a formal capability audit log that records every capability exercise for security analysis.",
        ],
    },
    Chapter {
        title: "3. Display Protocol",
        paragraphs: &[
            "The VYOMA_DRAW protocol is the heart of VyomaOS's display system. Every graphical app writes draw commands to stdout as text lines prefixed with VYOMA_DRAW:. The supervisor intercepts these lines and executes them against the framebuffer.",
            "The fundamental drawing primitives are: fill_rect (fill a rectangle with a solid color), draw_text (render a string using the embedded bitmap font), rect_border (draw a hollow rectangle border), clear_region (fill a region with the background color), and draw_text_wrap (render text with automatic line wrapping).",
            "Colors are specified as 32-bit RGBA values in hex format: 0xRRGGBBFF. The alpha byte is always 0xFF for opaque rendering. The color constants follow the GitHub dark theme palette: C_BG (0x0D1117FF), C_HEADER (0x21262DFF), C_BORDER (0x30363DFF), C_TEXT (0xE6EDF3FF), and accent colors.",
            "The text rendering system uses an embedded 8x16 bitmap font. Each character is 8 pixels wide and 16 pixels tall. The supervisor's font.rs module contains the complete font bitmap for all 95 printable ASCII characters. Text is rendered by looking up each character's 16-byte column bitmap and drawing it pixel by pixel.",
            "Phase 20 added multi-size text support. The draw_text command accepts a size field: 's' for 8x8, 'm' for 8x16, and 'l' for 16x32. The supervisor scales the bitmap font by 1x, 2x, or 4x as appropriate. All VyomaOS apps use size 'm' (8x16) as the standard text size.",
            "The double-buffered compositor (Phase 31) ensures tear-free rendering. All draw operations write to a back buffer in memory. The flush command atomically copies the back buffer to the memory-mapped framebuffer. Apps never write directly to the visible framebuffer.",
            "Window decorations (Phase 32) are painted by the supervisor on top of app content at flush time. Each app's window gets a title bar showing the app name and a close button. Clicking the close button sends VYOMA_SYSTEM:window_event:close to the app's stdin.",
            "The display driver opens /dev/fb0 or the virtio-gpu framebuffer at startup. It uses the FBIOGET_VSCREENINFO ioctl to determine the screen resolution and communicates this to display apps via VYOMA_SYSTEM:screen:<w>,<h> on launch.",
            "Z-ordering (Phase 33) allows windows to overlap. A global Z_ORDER list tracks the stacking order. Click-to-raise brings the clicked window to the front. Apps can request z-order changes via @supervisor: raise or @supervisor: lower commands.",
            "The screenshot capability (Phase 51) reads the back buffer and writes it as a PPM P6 file. The BGRA framebuffer pixel format is converted to RGB during the write. Screenshots are stored in /data/screenshot.ppm by default.",
        ],
    },
    Chapter {
        title: "4. IPC Broker Design",
        paragraphs: &[
            "The VyomaOS IPC broker is the supervisor's message routing subsystem. Every inter-app and app-to-supervisor message passes through the broker. No direct communication between apps is possible; all messages are mediated.",
            "The message format is simple: an app writes a line beginning with @ followed by the target name, a colon, a space, and the message payload. For example: @supervisor: ping. The supervisor intercepts this line from the app's stdout and routes it to the appropriate handler.",
            "The supervisor is the primary IPC target. It handles dozens of commands: process management (ps, kill, restart), window management (raise, lower, focus, resize), system control (shutdown, reboot, wallpaper), and infrastructure (notify, clipboard-set, download).",
            "Apps can also send messages to each other. If app A writes @appB: hello, the supervisor looks up appB in the running process table, finds its stdin pipe, and writes the message there. The receiving app reads it from stdin like any other input line.",
            "The ping-pong pattern is the standard way for apps to drive periodic updates. An app sends @supervisor: ping and receives REPLY:pong on its next stdin read. This round trip takes roughly one event loop iteration and drives animations, live data feeds, and UI refreshes.",
            "Replies from the supervisor follow the format REPLY:<data>. For example, @supervisor: ps-raw returns REPLY:ps-raw:<json> with the process table. Apps parse the reply by matching the prefix and extracting the payload after the colon.",
            "The IPC broker is thread-safe. Each app runs in its own thread with a dedicated stdin/stdout pipe pair. The supervisor's main thread processes keyboard events and routes them to the focused app. IPC replies are written from the command handler thread with a mutex-protected pipe write.",
            "Phase 44 added DNS resolution via the IPC broker: @supervisor: dns-resolve <host> triggers a TCP DNS query to 8.8.8.8:53, parses the A record response, and replies with REPLY:dns <host> <ip>. This gives WASM apps network name resolution without direct socket access.",
            "Phase 49 added background download via the broker: @supervisor: download <url> <dest> spawns a background thread that performs an HTTP GET, writes the response to /data/<dest>, and sends progress updates as REPLY:download-progress:<n>/<total> messages.",
            "Future IPC improvements include: typed message schemas with validation, message queuing for apps that are slow to respond, broadcast messages to multiple subscribers, and a debug mode that logs all IPC traffic to /data/ipc.log for troubleshooting.",
        ],
    },
    Chapter {
        title: "5. Future Roadmap",
        paragraphs: &[
            "The immediate next milestone for VyomaOS is a macOS-like desktop experience. This means building the full shell layer as composable WASM apps: a persistent Menu Bar at the top, a Dock at the bottom, a Spotlight search overlay, and a Mission Control bird's-eye view.",
            "The Menu Bar app (already shipped as Phase 72) sits at y=0 with h=28 and no window chrome. It polls ps-raw every ping-pong tick to show the focused app's name, displays a live clock, and has placeholder wifi and volume indicators.",
            "The Dock (Phase 73) sits at the bottom of the screen with nine app icons. Running apps are indicated by a dot under their icon. Number keys 1-9 launch or focus apps. The Dock queries ps-raw to determine which apps are running.",
            "Spotlight (Phase 74) is a 600x400 overlay that appears on demand. It searches a hard-coded list of 31 apps with case-insensitive substring matching. Pressing Enter launches the selected app via @supervisor: run.",
            "Mission Control (Phase 77) is a full-screen overlay showing all running apps as thumbnail cards. Arrow keys navigate the grid; Enter focuses the selected app. It demonstrates that complex window management is achievable as a pure WASM app.",
            "The next hardware target after x86 QEMU is ARM64. The Rust supervisor already compiles for aarch64-unknown-linux-musl. The main work is recompiling the Linux kernel for ARM64 and adding a QEMU aarch64 boot target to the Makefile.",
            "Real hardware support requires replacing the virtio drivers with actual hardware drivers. The display driver needs to work with real framebuffers via DRM. The input driver needs to read from /dev/input/event* instead of /dev/tty0.",
            "A package manager UI (beyond the current command-line pkg-install) would let users browse, install, and remove apps from a graphical interface. Phase 56 shipped a basic App Store UI that queries pkg-list and calls pkg-install on Enter.",
            "Accessibility features are planned for a future phase: high-contrast mode, screen magnifier, keyboard navigation for all apps, and TTS output via a speaker driver stub. These would be implemented as supervisor capabilities and per-app opt-in features.",
            "The ultimate goal is a production-quality minimal OS that can run on a Raspberry Pi or similar ARM SBC, boot in under 3 seconds, run 50+ WASM apps concurrently within 256 MB RAM, and provide a macOS-like user experience through composable capability-secure WASM applications.",
        ],
    },
];

fn wrap_text(s: &str, max_chars: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let words: Vec<&str> = s.split_whitespace().collect();
    let mut line = String::new();
    for word in &words {
        if line.len() + word.len() + 1 > max_chars {
            if !line.is_empty() { lines.push(line.clone()); line.clear(); }
        }
        if !line.is_empty() { line.push(' '); }
        line.push_str(word);
    }
    if !line.is_empty() { lines.push(line); }
    lines
}

struct App {
    chapter:  usize,
    scroll:   usize,
    bookmark: Option<(usize, usize)>,
    lines:    Vec<(u32, String)>, // (color, text)
    status:   String,
}

impl App {
    fn new() -> Self {
        let mut a = App {
            chapter: 0,
            scroll: 0,
            bookmark: None,
            lines: Vec::new(),
            status: String::from("←→=chapter  ↑↓/PgUp/PgDn=scroll  B=bookmark  G=goto bookmark  Ctrl+C=quit"),
        };
        a.build_lines();
        a
    }

    fn build_lines(&mut self) {
        self.lines.clear();
        let ch = &CHAPTERS[self.chapter];
        // Title line
        self.lines.push((C_ORANGE, ch.title.to_string()));
        self.lines.push((C_BORDER, "─".repeat(READ_CHARS.min(60))));
        self.lines.push((0, String::new())); // blank
        for para in ch.paragraphs {
            let wrapped = wrap_text(para, READ_CHARS);
            for wl in wrapped {
                self.lines.push((C_TEXT, wl));
            }
            self.lines.push((0, String::new())); // blank between paragraphs
        }
    }

    fn visible_lines(&self) -> usize {
        (CONTENT_H / LINE_H) as usize
    }
}

fn draw(app: &App) {
    fill(0, 0, W, H, C_BG);

    // Header
    fill(0, 0, W, HEADER_H, C_HEADER);
    fill(0, HEADER_H - 1, W, 1, C_BORDER);
    text(16, 12, C_ORANGE, "E-Book Reader");
    text(180, 12, C_HINT, &format!("Chapter {}/{}", app.chapter + 1, CHAPTERS.len()));
    if let Some((bc, _)) = app.bookmark {
        text(320, 12, C_YELLOW, &format!("★ Bookmark: Ch.{}", bc + 1));
    }

    // TOC panel
    fill(0, CONTENT_Y, TOC_W, CONTENT_H, C_CARD);
    fill(TOC_W, CONTENT_Y, 1, CONTENT_H, C_BORDER);
    text(8, CONTENT_Y + 4, C_HINT, "Contents");
    fill(0, CONTENT_Y + LINE_H, TOC_W, 1, C_BORDER);

    for (i, ch) in CHAPTERS.iter().enumerate() {
        let ty = CONTENT_Y + LINE_H + 2 + i as u32 * (LINE_H + 4);
        let short_title = if ch.title.len() > 22 { &ch.title[..22] } else { ch.title };
        if i == app.chapter {
            fill(0, ty - 2, TOC_W, LINE_H + 2, 0x1C2D4EFF);
            text(6, ty, C_SEL, short_title);
        } else {
            text(6, ty, C_HINT, short_title);
        }
        if let Some((bc, _)) = app.bookmark {
            if bc == i {
                text(TOC_W - 16, ty, C_YELLOW, "★");
            }
        }
    }

    // Reading pane
    fill(READ_X, CONTENT_Y, READ_W, CONTENT_H, C_BG);
    let vis = app.visible_lines();
    let scroll = app.scroll.min(app.lines.len().saturating_sub(vis));
    let start = scroll;
    let end = (start + vis).min(app.lines.len());

    for (vi, (col, line_text)) in app.lines[start..end].iter().enumerate() {
        let ly = CONTENT_Y + vi as u32 * LINE_H;
        if line_text.is_empty() { continue; }
        let c = if *col == 0 { C_TEXT } else { *col };
        text(READ_X + 16, ly + 2, c, line_text);
    }

    // Scroll indicator
    if app.lines.len() > vis {
        let track_h = CONTENT_H;
        let thumb_h = (vis as u32 * track_h / app.lines.len() as u32).max(4);
        let thumb_y = CONTENT_Y + (scroll as u32 * (track_h - thumb_h)) / (app.lines.len() as u32 - vis as u32).max(1);
        fill(W - 6, CONTENT_Y, 6, track_h, C_HEADER);
        fill(W - 6, thumb_y, 6, thumb_h, C_HINT);
    }

    // Status bar
    let sb_y = H - STATUS_H;
    fill(0, sb_y, W, STATUS_H, C_HEADER);
    fill(0, sb_y, W, 1, C_BORDER);
    let pct = if app.lines.len() > vis {
        (scroll * 100) / (app.lines.len() - vis).max(1)
    } else { 100 };
    text(8, sb_y + 4, C_HINT, &format!("{} · line {}/{} · {}% · {}", app.status,
        scroll + 1, app.lines.len(), pct, CHAPTERS[app.chapter].title));

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut app = App::new();

    println!("@supervisor: raise ebook");
    let _ = io::stdout().flush();
    draw(&app);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        let vis = app.visible_lines();
        let total = app.lines.len();

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[A" => {
                app.scroll = app.scroll.saturating_sub(1);
            }
            "\x1b[B" => {
                app.scroll = (app.scroll + 1).min(total.saturating_sub(vis));
            }
            "\x1b[5~" => {
                app.scroll = app.scroll.saturating_sub(vis);
            }
            "\x1b[6~" => {
                app.scroll = (app.scroll + vis).min(total.saturating_sub(vis));
            }
            "\x1b[H" => { app.scroll = 0; }
            "\x1b[F" => { app.scroll = total.saturating_sub(vis); }
            "\x1b[D" => {
                if app.chapter > 0 {
                    app.chapter -= 1;
                    app.scroll = 0;
                    app.build_lines();
                    app.status = format!("← {}", CHAPTERS[app.chapter].title);
                }
            }
            "\x1b[C" => {
                if app.chapter + 1 < CHAPTERS.len() {
                    app.chapter += 1;
                    app.scroll = 0;
                    app.build_lines();
                    app.status = format!("→ {}", CHAPTERS[app.chapter].title);
                }
            }
            "b" | "B" => {
                if app.bookmark == Some((app.chapter, app.scroll)) {
                    app.bookmark = None;
                    app.status = "Bookmark removed.".to_string();
                } else {
                    app.bookmark = Some((app.chapter, app.scroll));
                    app.status = format!("Bookmarked Ch.{} line {}", app.chapter + 1, app.scroll + 1);
                }
            }
            "g" | "G" => {
                if let Some((bc, bs)) = app.bookmark {
                    if bc != app.chapter {
                        app.chapter = bc;
                        app.build_lines();
                    }
                    app.scroll = bs.min(app.lines.len().saturating_sub(vis));
                    app.status = format!("Jumped to bookmark: Ch.{} line {}", bc + 1, bs + 1);
                } else {
                    app.status = "No bookmark set. Press B to bookmark current position.".to_string();
                }
            }
            _ => {}
        }
        draw(&app);
    }
}
