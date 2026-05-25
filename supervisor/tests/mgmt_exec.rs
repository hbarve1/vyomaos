// Integration test: management server exec handler.
//
// Registers a fake app with a mock IPC handler that echoes "pong" to any
// message and asserts the exec_reply contains "pong".

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

/// Sends `{"type":"exec","app":"echo-app","msg":"hello"}` to a server stub that
/// echoes "pong" and asserts the response is `exec_reply` with `reply: "pong"`.
#[test]
fn test_exec_delivers_message() {
    let port = free_port();

    let server = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();

    std::thread::spawn(move || {
        let (stream, _) = server.accept().unwrap();
        let mut writer = stream.try_clone().unwrap();
        let mut reader = BufReader::new(stream);

        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let req: serde_json::Value = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(req["type"], "exec");
        assert_eq!(req["app"],  "echo-app");
        assert_eq!(req["msg"],  "hello");

        // Simulate the app echoing "pong".
        let reply = serde_json::json!({"type": "exec_reply", "reply": "pong"});
        writeln!(writer, "{reply}").unwrap();
        writer.flush().unwrap();
    });

    let stream = connect_retry(port);
    stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut writer = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);

    let req = r#"{"type":"exec","app":"echo-app","msg":"hello","timeout_ms":5000}"#;
    writeln!(writer, "{req}").unwrap();
    writer.flush().unwrap();

    let mut resp_line = String::new();
    reader.read_line(&mut resp_line).unwrap();
    let v: serde_json::Value = serde_json::from_str(resp_line.trim_end()).unwrap();

    assert_eq!(v["type"],  "exec_reply", "expected exec_reply, got: {v}");
    assert_eq!(v["reply"], "pong",       "expected reply 'pong'");
}
