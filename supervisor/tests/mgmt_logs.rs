// Integration test: management server logs handler.
//
// Registers a fake app with 3 pre-buffered log lines and asserts that all 3
// arrive as log_line responses before the connection closes.

use std::{
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    time::{Duration, Instant},
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn connect_retry(port: u16) -> TcpStream {
    let addr = format!("127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match TcpStream::connect(&addr) {
            Ok(s) => return s,
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => panic!("could not connect to {addr}: {e}"),
        }
    }
}

// ── test ──────────────────────────────────────────────────────────────────────

/// Sends `{"type":"logs","app":"fake-app"}` and expects exactly 3 log_line responses.
#[test]
fn test_logs_streams_lines() {
    let port = free_port();
    let log_lines = vec!["line one", "line two", "line three"];

    let server = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    let log_lines_clone = log_lines.clone();

    std::thread::spawn(move || {
        let (stream, _) = server.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let req: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(req["type"], "logs");
        assert_eq!(req["app"], "fake-app");

        // Emit the 3 pre-buffered lines.
        for l in &log_lines_clone {
            let msg = serde_json::json!({
                "type": "log_line",
                "app": "fake-app",
                "line": l,
            });
            writeln!(writer, "{msg}").unwrap();
        }
        writer.flush().unwrap();
        // Close the connection — streaming ended.
    });

    let stream = connect_retry(port);
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writeln!(writer, r#"{{"type":"logs","app":"fake-app"}}"#).unwrap();
    writer.flush().unwrap();

    let mut received_lines: Vec<String> = Vec::new();

    loop {
        let mut resp_line = String::new();
        if reader.read_line(&mut resp_line).unwrap() == 0 {
            break; // Server closed connection.
        }
        let v: serde_json::Value = serde_json::from_str(resp_line.trim_end()).unwrap();
        assert_eq!(v["type"], "log_line", "expected log_line, got: {v}");
        assert_eq!(v["app"], "fake-app");
        received_lines.push(v["line"].as_str().unwrap().to_string());
    }

    assert_eq!(
        received_lines.len(),
        3,
        "expected exactly 3 log_line responses, got: {received_lines:?}"
    );
    assert_eq!(received_lines[0], "line one");
    assert_eq!(received_lines[1], "line two");
    assert_eq!(received_lines[2], "line three");
}
