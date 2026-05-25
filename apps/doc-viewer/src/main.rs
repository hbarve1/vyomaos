// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! doc-viewer — dual-mode document viewer
//!
//! Same WASM binary adapts to the platform profile at runtime:
//!
//!   desktop-full profile  → display module wired → VYOMA_DRAW rendering
//!   server-headless profile → only net wired     → HTTP serving on port 8081
//!
//! Mode detection: check the VYOMA_PLATFORM environment variable.
//! The supervisor sets VYOMA_PLATFORM=<profile-name> when launching apps.
//! If the variable is absent or unknown, fall back to attempting a socket bind
//! to determine whether network is available (server mode) or not (display mode).

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};

// ── Display constants ──────────────────────────────────────────────────────────
const W: u32 = 960;
const H: u32 = 700;
const BG: u32 = 0x1E1E2EFF;
const WHITE: u32 = 0xFFFFFFFF;
const BLUE: u32 = 0x89B4FAFF;
const GRAY: u32 = 0x6C7086FF;
const HTTP_PORT: u16 = 8081;

// ── Sample document content ────────────────────────────────────────────────────
const DOC_TITLE: &str = "VyomaOS — Universal Modular OS";
const DOC_LINES: &[&str] = &[
    "A WASM-first, capability-secure modular operating system.",
    "",
    "Key Features:",
    "  - Write once, run everywhere (wasm32-wasip2)",
    "  - Capability-declared manifests enforce isolation",
    "  - Same binary on desktop (GUI) and server (HTTP)",
    "  - Deterministic builds across all architectures",
    "",
    "Platform Profiles:",
    "  desktop-full:     VYOMA_DRAW rendering, full display stack",
    "  server-headless:  HTTP serving, no display subsystem",
    "",
    "User Story 3: Enterprise cross-platform deployment",
    "  doc-viewer.wasm runs identically on both profiles,",
    "  activating the appropriate output channel automatically.",
];

// ── Mode detection ─────────────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum Mode {
    Display,
    Http,
}

fn detect_mode() -> Mode {
    // Primary: check VYOMA_PLATFORM env var set by the supervisor at launch.
    if let Ok(platform) = std::env::var("VYOMA_PLATFORM") {
        if platform.contains("server") || platform.contains("headless") {
            return Mode::Http;
        }
        if platform.contains("desktop") || platform.contains("mobile") {
            return Mode::Display;
        }
    }

    // Fallback: attempt to bind the HTTP port.
    // The supervisor only wires WASI sockets when network=true AND the profile
    // includes the "net" module.  On a desktop profile without network wiring,
    // TcpListener::bind will fail with an error, indicating display mode.
    match TcpListener::bind(format!("0.0.0.0:{HTTP_PORT}")) {
        Ok(listener) => {
            // Network is wired → server mode.  Drop and re-bind in run_http.
            drop(listener);
            Mode::Http
        }
        Err(_) => Mode::Display,
    }
}

// ── Display mode ───────────────────────────────────────────────────────────────

fn flush_display() {
    println!("VYOMA_DRAW:flush");
    let _ = std::io::stdout().flush();
}

fn run_display() {
    // Draw document using VYOMA_DRAW protocol.
    println!("VYOMA_DRAW:fill_rect:0,20,{W},{H},{BG}");
    println!("VYOMA_DRAW:draw_text:16,28,{BLUE:#010x},m,{DOC_TITLE}");
    println!("VYOMA_DRAW:fill_rect:0,48,{W},2,{GRAY:#010x}");

    let mut y = 60u32;
    for line in DOC_LINES {
        if !line.is_empty() {
            println!("VYOMA_DRAW:draw_text:16,{y},{WHITE:#010x},m,{line}");
        }
        y += 18;
    }

    flush_display();
    eprintln!("doc-viewer: display mode active — rendered via VYOMA_DRAW");
}

// ── HTTP mode ──────────────────────────────────────────────────────────────────

fn doc_html() -> String {
    let mut rows = String::new();
    for line in DOC_LINES {
        if line.is_empty() {
            rows.push_str("<br>\n");
        } else {
            rows.push_str(&format!("<p>{line}</p>\n"));
        }
    }
    format!(
        "<!DOCTYPE html><html><head>\
         <title>{DOC_TITLE}</title>\
         <style>body{{font-family:monospace;background:#1e1e2e;color:#fff;padding:2em}}\
         h1{{color:#89b4fa}}p{{margin:0.2em 0}}</style>\
         </head><body><h1>{DOC_TITLE}</h1>\n{rows}</body></html>"
    )
}

fn handle_http(mut stream: TcpStream) {
    let mut reader = BufReader::new(&stream);
    let mut req_line = String::new();
    if reader.read_line(&mut req_line).is_err() {
        return;
    }
    // Drain remaining request headers.
    for line in reader.lines() {
        match line {
            Ok(l) if l.is_empty() => break,
            Ok(_) => {}
            Err(_) => return,
        }
    }

    let path = req_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, content_type, body) = match path {
        "/health" => (
            "200 OK",
            "application/json",
            r#"{"status":"ok","mode":"server-headless","app":"doc-viewer"}"#.to_string(),
        ),
        "/raw" => (
            "200 OK",
            "text/plain; charset=utf-8",
            DOC_LINES.join("\n"),
        ),
        _ => (
            "200 OK",
            "text/html; charset=utf-8",
            doc_html(),
        ),
    };

    let _ = write!(
        stream,
        "HTTP/1.0 {status}\r\nContent-Type: {content_type}\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn run_http() {
    let addr = format!("0.0.0.0:{HTTP_PORT}");
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("doc-viewer: HTTP bind failed on {addr}: {e}");
            return;
        }
    };
    eprintln!("doc-viewer: HTTP mode — listening on {addr}");

    for stream in listener.incoming() {
        match stream {
            Ok(s) => handle_http(s),
            Err(e) => eprintln!("doc-viewer: accept error: {e}"),
        }
    }
}

// ── Entry point ────────────────────────────────────────────────────────────────

fn main() {
    let mode = detect_mode();
    eprintln!("doc-viewer: detected mode = {mode:?}");

    match mode {
        Mode::Display => run_display(),
        Mode::Http => run_http(),
    }
}
