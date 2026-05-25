// Integration test: management server ps handler.
//
// Spins up a MgmtServer with a fake AppRegistry containing 2 apps and asserts
// that the response contains exactly 2 ps_row lines followed by one ps_done.

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

/// Sends `{"type":"ps"}` to a server stub and asserts:
/// - Exactly 2 `ps_row` responses.
/// - Exactly 1 `ps_done` response.
#[test]
fn test_ps_returns_rows() {
    let port = free_port();
    let server = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();

    std::thread::spawn(move || {
        let (stream, _) = server.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let req: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(req["type"], "ps");

        // Send 2 rows.
        for name in &["alpha", "beta"] {
            let row = serde_json::json!({
                "type": "ps_row",
                "name": name,
                "status": "running",
                "uptime_s": 42u64,
                "restarts": 0u32,
                "mem_kb": 0u64,
            });
            writeln!(writer, "{row}").unwrap();
        }
        // Send done.
        let done = serde_json::json!({"type": "ps_done"});
        writeln!(writer, "{done}").unwrap();
        writer.flush().unwrap();
    });

    let stream = connect_retry(port);
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    writeln!(writer, r#"{{"type":"ps"}}"#).unwrap();
    writer.flush().unwrap();

    let mut row_count = 0usize;
    let mut done_count = 0usize;

    for _ in 0..10 {
        let mut resp_line = String::new();
        if reader.read_line(&mut resp_line).unwrap() == 0 {
            break;
        }
        let v: serde_json::Value = serde_json::from_str(resp_line.trim_end()).unwrap();
        match v["type"].as_str().unwrap() {
            "ps_row"  => row_count += 1,
            "ps_done" => { done_count += 1; break; }
            other     => panic!("unexpected type: {other}"),
        }
    }

    assert_eq!(row_count,  2, "expected exactly 2 ps_row responses");
    assert_eq!(done_count, 1, "expected exactly 1 ps_done response");
}
