// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! WebSocket client: handshake, framing (RFC 6455), send/recv text frames.

use std::io::{Read, Write};
use std::net::TcpStream;

/// An active WebSocket connection wrapping a TCP stream.
#[allow(dead_code)]
pub struct WsConnection {
    pub stream: TcpStream,
    pub host: String,
    pub path: String,
    /// True once a close frame has been sent or received.
    pub closed: bool,
}

/// Parse a `ws://host:port/path` URL into (host, port, path).
pub fn parse_ws_url(url: &str) -> Result<(String, u16, String), String> {
    let rest = url.strip_prefix("ws://")
        .ok_or_else(|| "URL must start with ws://".to_string())?;

    let (hostport, path) = if let Some(idx) = rest.find('/') {
        (&rest[..idx], rest[idx..].to_string())
    } else {
        (rest, "/".to_string())
    };

    let (host, port) = if let Some(colon) = hostport.rfind(':') {
        let p: u16 = hostport[colon + 1..]
            .parse()
            .map_err(|_| format!("invalid port in '{hostport}'"))?;
        (hostport[..colon].to_string(), p)
    } else {
        (hostport.to_string(), 80)
    };

    Ok((host, port, path))
}

/// Generate a deterministic but unique-enough WebSocket key from the host+path.
/// In production this should be random; we use a simple hash for no-std friendliness.
fn generate_ws_key(host: &str, path: &str) -> String {
    // Simple deterministic 16-byte key encoded as base64.
    // RFC 6455 requires a base64-encoded 16-byte nonce.
    let mut key_bytes = [0u8; 16];
    let seed = host.as_bytes().iter().chain(path.as_bytes().iter());
    for (i, &b) in seed.enumerate() {
        key_bytes[i % 16] ^= b.wrapping_add(i as u8);
    }
    // Mix in a timestamp for uniqueness across calls.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    for i in 0..8 {
        key_bytes[i] ^= ((ts >> (i * 8)) & 0xFF) as u8;
    }
    base64_encode(&key_bytes)
}

/// Minimal base64 encoder (no padding issues for 16 bytes -> 24 chars).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as u32;
        let b1 = if i + 1 < data.len() { data[i + 1] as u32 } else { 0 };
        let b2 = if i + 2 < data.len() { data[i + 2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        out.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if i + 1 < data.len() {
            out.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if i + 2 < data.len() {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

/// Open a WebSocket connection to `ws://host:port/path`.
///
/// Performs the HTTP upgrade handshake and verifies a `101 Switching Protocols`
/// response before returning the connection handle.
pub fn ws_connect(url: &str) -> Result<WsConnection, String> {
    let (host, port, path) = parse_ws_url(url)?;
    let ws_key = generate_ws_key(&host, &path);

    let host_header = if port == 80 {
        host.clone()
    } else {
        format!("{host}:{port}")
    };

    let request = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: {host_header}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: {ws_key}\r\n\
         Sec-WebSocket-Version: 13\r\n\
         \r\n"
    );

    let mut stream = TcpStream::connect((&*host, port))
        .map_err(|e| format!("ws connect {host}:{port}: {e}"))?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).ok();

    stream.write_all(request.as_bytes())
        .map_err(|e| format!("ws handshake write: {e}"))?;

    // Read the HTTP response (headers end at \r\n\r\n).
    let mut hdr_buf = Vec::with_capacity(512);
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(1) => {
                hdr_buf.push(byte[0]);
                if hdr_buf.len() >= 4
                    && hdr_buf[hdr_buf.len() - 4..] == *b"\r\n\r\n"
                {
                    break;
                }
                if hdr_buf.len() > 4096 {
                    return Err("ws handshake response too large".to_string());
                }
            }
            Ok(_) => return Err("ws handshake: connection closed".to_string()),
            Err(e) => return Err(format!("ws handshake read: {e}")),
        }
    }

    let hdr_str = String::from_utf8_lossy(&hdr_buf);
    let status_line = hdr_str.lines().next().unwrap_or("");
    let status_code: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if status_code != 101 {
        return Err(format!("ws handshake: expected 101, got {status_code}"));
    }

    // Switch to a longer timeout for data transfer.
    stream.set_read_timeout(Some(std::time::Duration::from_millis(200))).ok();

    Ok(WsConnection {
        stream,
        host,
        path,
        closed: false,
    })
}

/// Send a text frame over the WebSocket connection.
///
/// Text frames use opcode 0x1 with FIN bit set and client-to-server masking
/// as required by RFC 6455 section 5.3.
pub fn ws_send(conn: &mut WsConnection, text: &str) -> Result<(), String> {
    if conn.closed {
        return Err("connection closed".to_string());
    }
    let frame = build_text_frame(text);
    conn.stream.write_all(&frame)
        .map_err(|e| format!("ws send: {e}"))
}

/// Read one text frame from the WebSocket connection.
///
/// Returns `Ok(Some(text))` for a text message, `Ok(None)` if no data is
/// available within the read timeout, or signals a close via `Err`.
pub fn ws_recv(conn: &mut WsConnection) -> Result<Option<String>, String> {
    if conn.closed {
        return Err("connection closed".to_string());
    }

    // Read first 2 bytes (FIN + opcode + mask bit + payload len).
    let mut header = [0u8; 2];
    match conn.stream.read_exact(&mut header) {
        Ok(()) => {}
        Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(None),
        Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => return Ok(None),
        Err(e) => return Err(format!("ws recv header: {e}")),
    }

    let opcode = header[0] & 0x0F;
    let masked = (header[1] & 0x80) != 0;
    let len0 = (header[1] & 0x7F) as u64;

    let payload_len: u64 = if len0 <= 125 {
        len0
    } else if len0 == 126 {
        let mut buf = [0u8; 2];
        conn.stream.read_exact(&mut buf)
            .map_err(|e| format!("ws recv len16: {e}"))?;
        u16::from_be_bytes(buf) as u64
    } else {
        let mut buf = [0u8; 8];
        conn.stream.read_exact(&mut buf)
            .map_err(|e| format!("ws recv len64: {e}"))?;
        u64::from_be_bytes(buf)
    };

    // Limit to 1 MB to prevent OOM.
    if payload_len > 1_048_576 {
        return Err("ws frame too large".to_string());
    }

    let mask_key = if masked {
        let mut mk = [0u8; 4];
        conn.stream.read_exact(&mut mk)
            .map_err(|e| format!("ws recv mask: {e}"))?;
        Some(mk)
    } else {
        None
    };

    let mut payload = vec![0u8; payload_len as usize];
    conn.stream.read_exact(&mut payload)
        .map_err(|e| format!("ws recv payload: {e}"))?;

    // Unmask if needed (server frames are typically unmasked).
    if let Some(mk) = mask_key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= mk[i % 4];
        }
    }

    match opcode {
        // 0x1 = text frame
        0x1 => {
            let text = String::from_utf8_lossy(&payload).to_string();
            Ok(Some(text))
        }
        // 0x8 = close frame
        0x8 => {
            conn.closed = true;
            Err("ws peer closed".to_string())
        }
        // 0x9 = ping -> send pong
        0x9 => {
            let pong = build_frame(0xA, &payload, true);
            let _ = conn.stream.write_all(&pong);
            Ok(None)
        }
        // 0xA = pong (ignore)
        0xA => Ok(None),
        _ => Ok(None),
    }
}

/// Send a close frame and mark the connection closed.
pub fn ws_close(conn: &mut WsConnection) -> Result<(), String> {
    if conn.closed {
        return Ok(());
    }
    let frame = build_frame(0x8, &[], true);
    let _ = conn.stream.write_all(&frame);
    conn.closed = true;
    Ok(())
}

/// Build a masked WebSocket frame with the given opcode and payload.
/// `mask` should be true for client-to-server frames (RFC 6455).
fn build_frame(opcode: u8, payload: &[u8], mask: bool) -> Vec<u8> {
    let mut frame = Vec::new();
    // FIN bit + opcode
    frame.push(0x80 | opcode);

    let len = payload.len();
    let mask_bit: u8 = if mask { 0x80 } else { 0x00 };

    if len <= 125 {
        frame.push(mask_bit | len as u8);
    } else if len <= 65535 {
        frame.push(mask_bit | 126);
        frame.extend_from_slice(&(len as u16).to_be_bytes());
    } else {
        frame.push(mask_bit | 127);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }

    if mask {
        // Generate a simple mask key from timestamp.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u32;
        let mask_key = ts.to_be_bytes();
        frame.extend_from_slice(&mask_key);

        for (i, &b) in payload.iter().enumerate() {
            frame.push(b ^ mask_key[i % 4]);
        }
    } else {
        frame.extend_from_slice(payload);
    }

    frame
}

/// Build a masked text frame (opcode 0x1).
fn build_text_frame(text: &str) -> Vec<u8> {
    build_frame(0x1, text.as_bytes(), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ws_url_basic() {
        let (host, port, path) = parse_ws_url("ws://example.com/chat").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 80);
        assert_eq!(path, "/chat");
    }

    #[test]
    fn parse_ws_url_custom_port() {
        let (host, port, path) = parse_ws_url("ws://localhost:9000/ws").unwrap();
        assert_eq!(host, "localhost");
        assert_eq!(port, 9000);
        assert_eq!(path, "/ws");
    }

    #[test]
    fn parse_ws_url_no_path() {
        let (host, port, path) = parse_ws_url("ws://host.io").unwrap();
        assert_eq!(host, "host.io");
        assert_eq!(port, 80);
        assert_eq!(path, "/");
    }

    #[test]
    fn parse_ws_url_invalid_scheme() {
        assert!(parse_ws_url("http://example.com").is_err());
    }

    #[test]
    fn parse_ws_url_invalid_port() {
        assert!(parse_ws_url("ws://example.com:abc/path").is_err());
    }

    #[test]
    fn frame_encode_small() {
        let frame = build_text_frame("Hi");
        // First byte: FIN + text opcode = 0x81
        assert_eq!(frame[0], 0x81);
        // Second byte: mask bit (0x80) + len 2
        assert_eq!(frame[1], 0x80 | 2);
        // 4 bytes mask key + 2 bytes masked payload = total 8 bytes
        assert_eq!(frame.len(), 2 + 4 + 2);
    }

    #[test]
    fn frame_encode_medium() {
        let text = "A".repeat(200);
        let frame = build_text_frame(&text);
        assert_eq!(frame[0], 0x81);
        // len > 125 -> uses 126 marker + 2-byte length
        assert_eq!(frame[1], 0x80 | 126);
        let ext_len = u16::from_be_bytes([frame[2], frame[3]]);
        assert_eq!(ext_len, 200);
        // 2 header + 2 ext len + 4 mask + 200 payload
        assert_eq!(frame.len(), 2 + 2 + 4 + 200);
    }

    #[test]
    fn frame_unmasked_roundtrip() {
        let payload = b"hello world";
        let frame = build_frame(0x1, payload, false);
        // No mask: FIN+opcode, len, payload
        assert_eq!(frame[0], 0x81);
        assert_eq!(frame[1], payload.len() as u8);
        assert_eq!(&frame[2..], payload);
    }

    #[test]
    fn frame_masked_payload_decodes() {
        let text = "test data";
        let frame = build_frame(0x1, text.as_bytes(), true);
        // Extract mask key and verify unmasking produces original text.
        let mask_key = &frame[2..6];
        let masked_payload = &frame[6..];
        let decoded: Vec<u8> = masked_payload.iter().enumerate()
            .map(|(i, &b)| b ^ mask_key[i % 4])
            .collect();
        assert_eq!(decoded, text.as_bytes());
    }

    #[test]
    fn base64_encode_known() {
        // "Hello" -> "SGVsbG8="
        assert_eq!(base64_encode(b"Hello"), "SGVsbG8=");
    }

    #[test]
    fn close_frame_structure() {
        let frame = build_frame(0x8, &[], true);
        assert_eq!(frame[0], 0x88); // FIN + close opcode
        assert_eq!(frame[1], 0x80); // mask bit, 0 length
        assert_eq!(frame.len(), 2 + 4); // header + mask key, no payload
    }
}
