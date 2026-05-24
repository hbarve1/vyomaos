// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use std::io::{self, BufRead, Write};

const W: u32 = 1280;
const H: u32 = 800;
const HEADER_H: u32 = 56;
const FOOTER_H: u32 = 48;
const SLIDE_PAD: u32 = 60;
const TITLE_Y: u32 = HEADER_H + 32;
const BULLET_Y: u32 = TITLE_Y + 64;
const BULLET_GAP: u32 = 36;
const CHAR_W: u32 = 8;

const C_BG: u32      = 0x0D1117FF;
const C_HEADER: u32  = 0x21262DFF;
const C_BORDER: u32  = 0x30363DFF;
const C_TEXT: u32    = 0xE6EDF3FF;
const C_HINT: u32    = 0x6E7681FF;
const C_ORANGE: u32  = 0xFFA657FF;
const C_GREEN: u32   = 0x3FB950FF;
const C_SEL: u32     = 0x58A6FFFF;
const C_CARD: u32    = 0x161B22FF;
const C_YELLOW: u32  = 0xD29922FF;
const C_PURPLE: u32  = 0xBC8CFFFF;

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

struct Slide {
    title:   &'static str,
    color:   u32,
    bullets: &'static [&'static str],
}

const SLIDES: &[Slide] = &[
    Slide {
        title: "VyomaOS — A WASM-First Operating System",
        color: C_ORANGE,
        bullets: &[
            "Built from the ground up on a capability-secure WebAssembly foundation",
            "Every application is a wasm32-wasip2 binary executed by Wasmtime",
            "Linux kernel handles hardware only — supervisor handles everything else",
            "Goal: a lightweight, fully capable, macOS-like general-purpose OS",
            "Security by design: undeclared capabilities are not accessible",
        ],
    },
    Slide {
        title: "System Architecture",
        color: C_SEL,
        bullets: &[
            "Linux 5.10 kernel (allnoconfig, 2.3 MB) — drivers only, no syscall policy",
            "Rust supervisor (PID 1, 697 KB musl static) — scheduler, IPC, display",
            "Wasmtime 43.0.0 (WASI Preview 2) — WASM runtime with capability model",
            "WASM apps (1–10 KB each) — isolated, byte-identical across platforms",
            "Seccomp BPF denylist applied to all Wasmtime children",
        ],
    },
    Slide {
        title: "Capability-Secure App Model",
        color: C_GREEN,
        bullets: &[
            "Each app declares: stdio, filesystem, network, display, shell, mouse",
            "Supervisor wires only declared WASI imports — no filter layer needed",
            "No network interface → no network syscalls possible (not filtered, absent)",
            "vyoma.toml manifest: human-readable TOML, validated at startup",
            "Restart policies: never (one-shot tools), always (daemons)",
        ],
    },
    Slide {
        title: "Display System — VYOMA_DRAW Protocol",
        color: C_YELLOW,
        bullets: &[
            "Apps write VYOMA_DRAW: commands to stdout — parsed by supervisor",
            "fill_rect / draw_text / rect_border / flush — full pixel control",
            "Double-buffered compositor: all draws to back buffer, flush presents",
            "Window decorations, z-ordering, click-to-raise handled by supervisor",
            "8×16 bitmap font — deterministic, no font loading required",
        ],
    },
    Slide {
        title: "IPC Broker",
        color: C_PURPLE,
        bullets: &[
            "Apps write '@<target>: <message>' to stdout",
            "Supervisor intercepts, routes payload to target app's stdin",
            "Ping-pong pattern: @supervisor: ping → REPLY:pong drives timers",
            "Process management: ps-raw, kill, restart, raise, lower, focus",
            "Cross-app communication enables rich desktop experiences",
        ],
    },
    Slide {
        title: "App Ecosystem (150+ Apps)",
        color: C_ORANGE,
        bullets: &[
            "Desktop: Finder, Dock, Menu Bar, Spotlight, Mission Control, Spaces",
            "Productivity: Text Editor, Spreadsheet, Code Editor, Kanban Board, Notes",
            "Creative: Paint, Pixel Art, ASCII Art, Music Composer, Music Visualizer",
            "Games: Tetris, Snake, Chess, Minesweeper, Wordle, Breakout, Pong",
            "Dev Tools: Hex Editor, JSON Viewer, File Diff, Code Runner, Log Viewer",
        ],
    },
    Slide {
        title: "Persistent Storage",
        color: C_SEL,
        bullets: &[
            "Host data/ directory mounted via 9P virtio at /data inside VM",
            "Apps with filesystem=true can read/write /data persistently",
            "Files survive VM reboots — no re-initialization needed",
            "Package manager, settings, session state all stored in /data",
            "CSV, TOML, PPM, raw binary — no database dependency",
        ],
    },
    Slide {
        title: "Build System",
        color: C_GREEN,
        bullets: &[
            "Docker-based hermetic builds — identical output on any host",
            "Single Makefile: kernel → supervisor → WASM apps → rootfs → boot",
            "WASM binaries: byte-identical across builds (no linker variance)",
            "SHA-256 verification for Wasmtime and BusyBox downloads",
            "make build + make run-gui — full OS in under 5 seconds",
        ],
    },
    Slide {
        title: "Road to macOS-Like Desktop",
        color: C_YELLOW,
        bullets: &[
            "Menu Bar (top): focused app name, clock, system tray icons",
            "Dock (bottom): app icons, running indicators, click to launch",
            "Spotlight: instant search + launch overlay (Cmd+Space)",
            "Window Manager: tiling, z-order, resize, session save/restore",
            "Notifications, Context Menus, Drag-and-Drop, Clipboard History",
        ],
    },
    Slide {
        title: "Open Source & Community",
        color: C_PURPLE,
        bullets: &[
            "Apache 2.0 licensed — contributions welcome",
            "Built with Rust + WebAssembly — modern, memory-safe foundation",
            "Each app is a self-contained Rust binary — easy to contribute",
            "GitHub: github.com/hbarve1/vyomaos",
            "Join us in building the next generation of operating systems!",
        ],
    },
];

fn draw_slide(idx: usize, fullscreen: bool) {
    let slide = &SLIDES[idx];
    fill(0, 0, W, H, C_BG);

    if !fullscreen {
        fill(0, 0, W, HEADER_H, C_HEADER);
        fill(0, HEADER_H - 1, W, 1, C_BORDER);
        text(16, 18, slide.color, "VyomaOS Presentation");
        text(360, 18, C_HINT, &format!("Slide {} / {}", idx + 1, SLIDES.len()));
        text(560, 18, C_HINT, "← → : navigate   F : fullscreen   Ctrl+C : exit");
    }

    let content_y = if fullscreen { 24 } else { HEADER_H + 8 };
    let content_h = if fullscreen { H - 24 - FOOTER_H } else { H - HEADER_H - FOOTER_H - 8 };

    // Slide background panel
    fill(SLIDE_PAD, content_y, W - SLIDE_PAD * 2, content_h, C_CARD);
    border(SLIDE_PAD, content_y, W - SLIDE_PAD * 2, content_h, slide.color);

    // Title
    let title_x = SLIDE_PAD + 24;
    let ty = content_y + 32;
    // Draw title larger using spacing trick — repeat 3 times offset by 1px for bold effect
    text(title_x, ty,     slide.color, slide.title);
    text(title_x + 1, ty, slide.color, slide.title);

    // Divider
    fill(SLIDE_PAD + 24, ty + 28, W - (SLIDE_PAD + 24) * 2, 2, slide.color);

    // Bullets
    let max_bullet_w = (W - SLIDE_PAD * 2 - 64) as usize / CHAR_W as usize;
    for (i, bullet) in slide.bullets.iter().enumerate() {
        let by = ty + 44 + i as u32 * BULLET_GAP;
        // Bullet dot
        fill(title_x, by + 7, 6, 6, slide.color);
        // Text — truncate if too long
        let disp = if bullet.len() > max_bullet_w { &bullet[..max_bullet_w] } else { bullet };
        text(title_x + 14, by, C_TEXT, disp);
    }

    // Footer: progress bar + slide indicator
    let footer_y = H - FOOTER_H;
    fill(0, footer_y, W, FOOTER_H, C_HEADER);
    fill(0, footer_y, W, 1, C_BORDER);

    // Progress bar
    let bar_w = (W - 32) * (idx as u32 + 1) / SLIDES.len() as u32;
    fill(16, footer_y + 10, W - 32, 8, C_BORDER);
    fill(16, footer_y + 10, bar_w, 8, slide.color);

    // Slide counter
    let counter = format!("{} / {}", idx + 1, SLIDES.len());
    let cx = W - 8 - counter.len() as u32 * CHAR_W;
    text(cx, footer_y + 28, C_HINT, &counter);

    // Fullscreen toggle indicator
    let fs_label = if fullscreen { "[F] Exit fullscreen" } else { "[F] Fullscreen" };
    text(16, footer_y + 28, C_HINT, fs_label);

    flush();
}

fn main() {
    let stdin = io::stdin();
    let mut idx = 0usize;
    let mut fullscreen = false;

    println!("@supervisor: raise presentation");
    let _ = io::stdout().flush();
    draw_slide(idx, fullscreen);

    for line in stdin.lock().lines() {
        let raw = match line { Ok(l) => l, Err(_) => break };
        if raw.starts_with("REPLY:") { continue; }

        match raw.as_str() {
            "\x03" => { fill(0, 0, W, H, C_BG); flush(); std::process::exit(0); }
            "\x1b[C" | "\x1b[6~" => { // Right / PgDn
                if idx + 1 < SLIDES.len() { idx += 1; }
            }
            "\x1b[D" | "\x1b[5~" => { // Left / PgUp
                if idx > 0 { idx -= 1; }
            }
            "\x1b[H" => { idx = 0; }                    // Home
            "\x1b[F" => { idx = SLIDES.len() - 1; }     // End
            "f" | "F" => { fullscreen = !fullscreen; }
            _ => { continue; }
        }
        draw_slide(idx, fullscreen);
    }
}
