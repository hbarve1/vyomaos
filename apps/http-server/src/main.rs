//! VyomaOS HTTP status server
//!
//! Serves a live status page at http://localhost:8080/.
//! Uses only std::net — wasmtime provides the WASI socket via -S tcplisten.

use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
};

fn main() {
    let listener = TcpListener::bind("0.0.0.0:8080").expect("bind 0.0.0.0:8080");
    eprintln!("http-server: listening on 0.0.0.0:8080");

    for stream in listener.incoming() {
        match stream {
            Ok(s)  => handle(s),
            Err(e) => eprintln!("http-server: accept error: {e}"),
        }
    }
}

fn handle(mut stream: TcpStream) {
    let mut reader = BufReader::new(&stream);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() { return; }
    // Drain remaining request headers
    for line in reader.lines() {
        match line {
            Ok(l) if l.is_empty() => break,
            Ok(_)  => {}
            Err(_) => return,
        }
    }

    let path = request_line.split_whitespace().nth(1).unwrap_or("/");
    let (status, content_type, body) = match path {
        "/health" => (
            "200 OK",
            "application/json",
            r#"{"status":"ok","os":"VyomaOS"}"#.to_string(),
        ),
        "/apps" => (
            "200 OK",
            "application/json",
            apps_json(),
        ),
        _ => (
            "200 OK",
            "text/html; charset=utf-8",
            status_html(),
        ),
    };

    let _ = write!(
        stream,
        "HTTP/1.0 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

fn boot_count() -> u64 {
    std::fs::read_to_string("/data/boot_count.txt")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

fn boot_log() -> String {
    std::fs::read_to_string("/data/boot_log.txt").unwrap_or_default()
}

fn apps_json() -> String {
    let apps = [
        "hello-world", "calculator", "factorial",
        "ping", "pong", "storage-demo", "gui-demo", "http-server",
    ];
    let list = apps.iter()
        .map(|a| format!(r#""{}""#, a))
        .collect::<Vec<_>>()
        .join(",");
    format!(r#"{{"apps":[{list}]}}"#)
}

fn status_html() -> String {
    let count = boot_count();
    let log   = boot_log().replace('\n', "<br>");
    format!(r#"<!DOCTYPE html>
<html><head><meta charset="utf-8">
<title>VyomaOS Status</title>
<style>
body {{ font-family: monospace; background: #0d1117; color: #c9d1d9; padding: 2rem; }}
h1   {{ color: #58a6ff; margin-bottom: 0.25rem; }}
h2   {{ color: #8b949e; margin-top: 1.5rem; }}
p    {{ margin: 0.5rem 0; }}
table {{ border-collapse: collapse; width: 100%; max-width: 600px; }}
td, th {{ border: 1px solid #30363d; padding: 8px 12px; text-align: left; }}
th   {{ background: #161b22; color: #8b949e; }}
.ok  {{ color: #3fb950; }}
.tag {{ color: #58a6ff; font-size: 0.85em; }}
</style></head>
<body>
<h1>VyomaOS</h1>
<p class="tag">WASM-first OS &nbsp;|&nbsp; Linux 5.10 &nbsp;|&nbsp; Wasmtime WASI Preview 2</p>
<p>Boot count: <b>{count}</b></p>
<h2>Running Apps</h2>
<table>
<tr><th>App</th><th>Capabilities</th><th>Status</th></tr>
<tr><td>hello-world</td>   <td>stdio</td>                          <td class="ok">done</td></tr>
<tr><td>calculator</td>    <td>stdio</td>                          <td class="ok">done</td></tr>
<tr><td>factorial</td>     <td>stdio</td>                          <td class="ok">done</td></tr>
<tr><td>ping</td>          <td>stdio, ipc</td>                     <td class="ok">done</td></tr>
<tr><td>pong</td>          <td>stdio, ipc</td>                     <td class="ok">done</td></tr>
<tr><td>storage-demo</td>  <td>stdio, filesystem</td>              <td class="ok">done</td></tr>
<tr><td>gui-demo</td>      <td>stdio, filesystem, display</td>     <td class="ok">done</td></tr>
<tr><td>http-server</td>   <td>stdio, filesystem, network:8080</td><td class="ok">running</td></tr>
</table>
<h2>Boot Log</h2>
<p>{log}</p>
<h2>Endpoints</h2>
<table>
<tr><th>Path</th><th>Response</th></tr>
<tr><td>/</td>       <td>This page (HTML)</td></tr>
<tr><td>/health</td> <td>JSON health check</td></tr>
<tr><td>/apps</td>   <td>JSON app list</td></tr>
</table>
</body></html>"#)
}
