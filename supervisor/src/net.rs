// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! DNS resolution, HTTP helpers, and DRM connector counting.

use std::fs;

/// Count DRM connectors visible under /sys/class/drm (P43).
pub fn count_drm_connectors() -> usize {
    fs::read_dir("/sys/class/drm")
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("card0-"))
                .count()
        })
        .unwrap_or(1)
        .max(1)
}

/// Synchronous DNS A-record lookup over TCP to 8.8.8.8:53 (P44).
pub fn dns_resolve_a(hostname: &str) -> Option<String> {
    use std::io::{Read, Write as _};
    use std::net::TcpStream;

    // Build DNS query packet
    let mut q: Vec<u8> = Vec::new();
    q.extend_from_slice(&[0x12, 0x34, 0x01, 0x00]); // ID + flags (RD)
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]); // counts
    for label in hostname.trim_end_matches('.').split('.') {
        let b = label.as_bytes();
        q.push(b.len() as u8);
        q.extend_from_slice(b);
    }
    q.push(0x00);                                // root label
    q.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]); // Type A, Class IN

    // TCP DNS: 2-byte big-endian length prefix
    let mut msg: Vec<u8> = vec![(q.len() >> 8) as u8, (q.len() & 0xFF) as u8];
    msg.extend_from_slice(&q);

    let mut stream = TcpStream::connect("8.8.8.8:53").ok()?;
    stream.set_read_timeout(Some(std::time::Duration::from_secs(4))).ok()?;
    stream.write_all(&msg).ok()?;

    // Read response length then body
    let mut lbuf = [0u8; 2];
    stream.read_exact(&mut lbuf).ok()?;
    let rlen = u16::from_be_bytes(lbuf) as usize;
    let mut resp = vec![0u8; rlen];
    stream.read_exact(&mut resp).ok()?;

    if resp.len() < 12 { return None; }
    let ancount = u16::from_be_bytes([resp[6], resp[7]]) as usize;
    if ancount == 0 { return None; }

    // Skip question section
    let mut pos = 12usize;
    pos = dns_skip_name(&resp, pos)?;
    pos += 4; // type + class

    // Parse first answer record
    pos = dns_skip_name(&resp, pos)?;
    if pos + 10 > resp.len() { return None; }
    let rtype  = u16::from_be_bytes([resp[pos],   resp[pos+1]]);
    pos += 8; // type(2) + class(2) + ttl(4)
    let rdlen  = u16::from_be_bytes([resp[pos], resp[pos+1]]) as usize;
    pos += 2;

    if rtype == 1 && rdlen == 4 && pos + 4 <= resp.len() {
        return Some(format!("{}.{}.{}.{}", resp[pos], resp[pos+1], resp[pos+2], resp[pos+3]));
    }
    None
}

/// Skip a DNS name at `pos` in `buf`, returning the position after it.
pub fn dns_skip_name(buf: &[u8], mut pos: usize) -> Option<usize> {
    loop {
        if pos >= buf.len() { return None; }
        let b = buf[pos] as usize;
        if b == 0                 { return Some(pos + 1); }
        if (b & 0xC0) == 0xC0    { return Some(pos + 2); } // compressed pointer
        pos += b + 1;
    }
}

/// Parse a URL into (scheme, host, port, path). Supports http:// and https://.
pub fn parse_url(url: &str) -> Result<(bool, String, u16, String), String> {
    let (is_https, rest) = if let Some(r) = url.strip_prefix("https://") {
        (true, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (false, r)
    } else {
        return Err("URL must start with http:// or https://".to_string());
    };

    let (hostport, path) = if let Some(idx) = rest.find('/') {
        (&rest[..idx], rest[idx..].to_string())
    } else {
        (rest, "/".to_string())
    };

    let default_port: u16 = if is_https { 443 } else { 80 };
    let (host, port) = if let Some(c) = hostport.rfind(':') {
        let p: u16 = hostport[c + 1..]
            .parse()
            .map_err(|_| format!("invalid port in '{hostport}'"))?;
        (hostport[..c].to_string(), p)
    } else {
        (hostport.to_string(), default_port)
    };

    Ok((is_https, host, port, path))
}

/// Minimal HTTP/1.1 GET over plain TCP or TLS (P30/P45). Returns the response body.
/// Supports both `http://` and `https://` URLs.
pub fn http_get(url: &str) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let (is_https, host, port, path) = parse_url(url)?;

    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n"
    );

    let mut raw = Vec::new();

    if is_https {
        raw = https_request(&host, port, req.as_bytes())?;
    } else {
        let mut stream = TcpStream::connect((&*host, port))
            .map_err(|e| format!("connect {host}:{port}: {e}"))?;
        stream.write_all(req.as_bytes())
            .map_err(|e| format!("request write: {e}"))?;
        stream.read_to_end(&mut raw)
            .map_err(|e| format!("response read: {e}"))?;
    }

    let sep = raw.windows(4).position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| "no HTTP header boundary in response".to_string())?;

    let status: u16 = std::str::from_utf8(&raw[..sep])
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("")
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    if status != 200 {
        return Err(format!("HTTP {status}"));
    }

    Ok(raw[sep + 4..].to_vec())
}

/// Perform a raw TLS-wrapped request, returning the full response bytes.
fn https_request(host: &str, port: u16, request: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Arc;

    let root_store = rustls::RootCertStore::from_iter(
        webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
    );

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    let server_name = rustls::pki_types::ServerName::try_from(host.to_string())
        .map_err(|e| format!("invalid server name '{host}': {e}"))?;

    let mut conn = rustls::ClientConnection::new(Arc::new(config), server_name)
        .map_err(|e| format!("TLS init: {e}"))?;

    let mut tcp = TcpStream::connect((&*host, port))
        .map_err(|e| format!("connect {host}:{port}: {e}"))?;
    tcp.set_read_timeout(Some(std::time::Duration::from_secs(10))).ok();

    let mut tls = rustls::Stream::new(&mut conn, &mut tcp);

    tls.write_all(request)
        .map_err(|e| format!("TLS write: {e}"))?;

    let mut raw = Vec::new();
    loop {
        let mut buf = [0u8; 8192];
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionAborted => break,
            Err(ref e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("TLS read: {e}")),
        }
    }

    Ok(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_url_http() {
        let (tls, host, port, path) = parse_url("http://example.com/foo").unwrap();
        assert!(!tls);
        assert_eq!(host, "example.com");
        assert_eq!(port, 80);
        assert_eq!(path, "/foo");
    }

    #[test]
    fn parse_url_https_default_port() {
        let (tls, host, port, path) = parse_url("https://example.com/bar").unwrap();
        assert!(tls);
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert_eq!(path, "/bar");
    }

    #[test]
    fn parse_url_https_custom_port() {
        let (tls, host, port, path) = parse_url("https://example.com:8443/baz").unwrap();
        assert!(tls);
        assert_eq!(host, "example.com");
        assert_eq!(port, 8443);
        assert_eq!(path, "/baz");
    }

    #[test]
    fn parse_url_no_path() {
        let (tls, host, port, path) = parse_url("https://example.com").unwrap();
        assert!(tls);
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert_eq!(path, "/");
    }

    #[test]
    fn parse_url_invalid_scheme() {
        assert!(parse_url("ftp://example.com").is_err());
    }

    #[test]
    fn parse_url_invalid_port() {
        assert!(parse_url("http://example.com:notaport/x").is_err());
    }
}
