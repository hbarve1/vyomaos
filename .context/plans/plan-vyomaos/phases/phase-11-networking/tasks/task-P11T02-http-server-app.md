# P11T02 — http-server WASM App

## Phase
Phase 11 — Networking

## Goal
Build `apps/http-server/` — a WASM app that accepts TCP connections on port 8080 and serves HTTP/1.0 responses. Uses only `std::io` and `std::net` (wasmtime maps WASI sockets to these via `-S tcplisten`). No external HTTP crate needed.

## Files to create

```
apps/http-server/Cargo.toml
apps/http-server/src/main.rs
apps/http-server/vyoma.toml
```

## Implementation

### `apps/http-server/vyoma.toml`

```toml
[app]
name    = "http-server"
version = "0.1.0"
wasm    = "http-server.wasm"

[capabilities]
stdio      = true
filesystem = true    # reads /data/boot_count.txt, boot_log.txt
network    = true    # grants -S tcplisten=0.0.0.0:8080
display    = false
```

### `apps/http-server/Cargo.toml`

```toml
[package]
name    = "http-server"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "http-server"
path = "src/main.rs"

[profile.release]
opt-level = "z"
strip     = true
```

### `apps/http-server/src/main.rs`

```rust
//! VyomaOS HTTP status server
//!
//! Serves a live status page at http://localhost:8080/.
//! Uses only std::net — wasmtime provides the WASI socket via -S tcplisten.

use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
};

fn main() {
    let listener = TcpListener::bind("0.0.0.0:8080")
        .expect("bind 0.0.0.0:8080");
    eprintln!("http-server: listening on 0.0.0.0:8080");

    for stream in listener.incoming() {
        match stream {
            Ok(s) => handle(s),
            Err(e) => eprintln!("http-server: accept error: {e}"),
        }
    }
}

fn handle(mut stream: TcpStream) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() { return; }
    // Drain remaining headers
    for line in reader.lines() {
        match line { Ok(l) if l.is_empty() => break, Ok(_) => {}, Err(_) => return }
    }

    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, body) = match path {
        "/health" => ("200 OK", r#"{"status":"ok","os":"VyomaOS"}"#.to_string()),
        "/apps"   => ("200 OK", apps_json()),
        _         => ("200 OK", status_html()),
    };

    let content_type = if path == "/health" || path == "/apps" {
        "application/json"
    } else {
        "text/html; charset=utf-8"
    };

    let _ = write!(stream,
        "HTTP/1.0 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn boot_count() -> u64 {
    std::fs::read_to_string("/data/boot_count.txt")
        .ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0)
}

fn boot_log() -> String {
    std::fs::read_to_string("/data/boot_log.txt").unwrap_or_default()
}

fn apps_json() -> String {
    let apps = ["hello-world","calculator","factorial","ping","pong","storage-demo","gui-demo","http-server"];
    let list = apps.iter().map(|a| format!(r#""{}""#, a)).collect::<Vec<_>>().join(",");
    format!(r#"{{"apps":[{list}]}}"#)
}

fn status_html() -> String {
    let count = boot_count();
    let log   = boot_log().replace('\n', "<br>");
    format!(r#"<!DOCTYPE html>
<html><head><meta charset="utf-8">
<title>VyomaOS Status</title>
<style>body{{font-family:monospace;background:#0d1117;color:#c9d1d9;padding:2rem}}
h1{{color:#58a6ff}}table{{border-collapse:collapse;width:100%}}
td,th{{border:1px solid #30363d;padding:8px 12px}}th{{background:#161b22}}
.ok{{color:#3fb950}}</style></head>
<body>
<h1>VyomaOS</h1>
<p>Boot count: <b>{count}</b></p>
<h2>Apps</h2>
<table><tr><th>App</th><th>Status</th></tr>
<tr><td>hello-world</td><td class="ok">done</td></tr>
<tr><td>calculator</td><td class="ok">done</td></tr>
<tr><td>factorial</td><td class="ok">done</td></tr>
<tr><td>ping</td><td class="ok">done</td></tr>
<tr><td>pong</td><td class="ok">done</td></tr>
<tr><td>storage-demo</td><td class="ok">done</td></tr>
<tr><td>gui-demo</td><td class="ok">done</td></tr>
<tr><td>http-server</td><td class="ok">running</td></tr>
</table>
<h2>Boot Log</h2><p>{log}</p>
</body></html>"#)
}
```

## Notes

- `restart = "always"` should be used in boot.toml for the http-server so it restarts if it crashes; however the current supervisor treats always/on-failure as never for IPC apps — a separate fix may be needed
- wasmtime `-S tcplisten=0.0.0.0:8080` is already wired in `spawn_app` when `caps.network = true`
- HTTP/1.0 (not 1.1) is intentional — no chunked encoding, keep-alive, or Host header required

## Verification

```sh
make run-net
# From host:
curl -s http://localhost:8080/         | grep "VyomaOS"
curl -s http://localhost:8080/health   | python3 -m json.tool
curl -s http://localhost:8080/apps     | python3 -m json.tool
```
