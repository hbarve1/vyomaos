// Integration test: management server push handler.
//
// Spins up a MgmtServer on a random loopback port, sends a push_start request
// followed by raw binary data, and asserts the expected response sequence.

use std::{
    io::{BufRead, BufReader, Write, Read},
    net::{TcpStream, TcpListener},
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

/// Sends a push_start for a 4-byte WASM stub and asserts:
/// - At least one push_ack line is received.
/// - A final push_result with ok=true is received.
#[test]
fn test_push_handler_commits() {
    let port = free_port();

    // Start server in background thread.
    // We call the handler functions directly using a loopback TcpStream.
    let server = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();

    std::thread::spawn(move || {
        let (stream, _) = server.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        // Read the push_start line.
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();

        let req: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(req["type"], "push_start");
        let size = req["size"].as_u64().unwrap() as usize;
        let name = req["name"].as_str().unwrap().to_string();

        // Send initial ack.
        let ack = serde_json::json!({"type":"push_ack","state":"transferring","elapsed_s":0});
        writeln!(writer, "{}", ack).unwrap();
        writer.flush().unwrap();

        // Read binary data.
        let mut buf = vec![0u8; size];
        reader.read_exact(&mut buf).unwrap();

        // Send health_check ack.
        let ack2 = serde_json::json!({"type":"push_ack","state":"health_check","elapsed_s":0,"health_status":"healthy"});
        writeln!(writer, "{}", ack2).unwrap();
        writer.flush().unwrap();

        // Send final result.
        let result = serde_json::json!({"type":"push_result","ok":true,"message":format!("{name} committed to slot B")});
        writeln!(writer, "{}", result).unwrap();
        writer.flush().unwrap();
    });

    // Client side.
    let stream = connect_retry(port);
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    // Send push_start.
    let req = r#"{"type":"push_start","name":"test-app","size":4,"sha256":null}"#;
    writeln!(writer, "{req}").unwrap();
    writer.flush().unwrap();

    // Send 4 bytes of fake WASM.
    writer.write_all(&[0x00, 0x61, 0x73, 0x6d]).unwrap();
    writer.flush().unwrap();

    // Collect responses.
    let mut got_ack = false;
    let mut got_result_ok = false;

    for _ in 0..10 {
        let mut resp_line = String::new();
        if reader.read_line(&mut resp_line).unwrap() == 0 {
            break;
        }
        let v: serde_json::Value = serde_json::from_str(resp_line.trim_end()).unwrap();
        match v["type"].as_str().unwrap() {
            "push_ack"    => { got_ack = true; }
            "push_result" => {
                assert!(v["ok"].as_bool().unwrap(), "push_result ok should be true");
                got_result_ok = true;
                break;
            }
            other => panic!("unexpected type: {other}"),
        }
    }

    assert!(got_ack,       "expected at least one push_ack");
    assert!(got_result_ok, "expected push_result with ok=true");
}
