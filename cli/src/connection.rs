// VyomaConnection — TCP and Unix socket transport for the vyoma CLI.
//
// Abstracts both transports behind a uniform BufReader/Write interface.
// Implements exponential backoff retry for TCP connection refused.

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpStream, SocketAddr, ToSocketAddrs},
    path::Path,
    time::{Duration, Instant},
};

use crate::protocol::{MgmtRequest, MgmtResponse};

// ── VyomaConnection ───────────────────────────────────────────────────────────

/// An active connection to the VyomaOS supervisor management server.
pub struct VyomaConnection {
    pub reader: BufReader<Box<dyn Read + Send>>,
    pub writer: Box<dyn Write + Send>,
}

impl VyomaConnection {
    /// Connect to the supervisor over TCP with exponential backoff retry.
    ///
    /// On `ConnectionRefused` sleeps 100 ms → 200 ms → 400 ms → 800 ms →
    /// 1600 ms, then gives up (total ≤ 10 s).  All other errors fail immediately.
    pub fn connect(host: &str, port: u16) -> anyhow::Result<Self> {
        let addr_str = format!("{host}:{port}");
        let addr: SocketAddr = addr_str
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| anyhow::anyhow!("could not resolve {addr_str}"))?;

        eprintln!("Connecting to {addr_str}...");

        let mut delay_ms: u64 = 100;
        let deadline = Instant::now() + Duration::from_secs(10);

        loop {
            match TcpStream::connect_timeout(&addr, Duration::from_secs(10)) {
                Ok(stream) => {
                    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
                    let writer: Box<dyn Write + Send> =
                        Box::new(stream.try_clone()?);
                    let reader = BufReader::new(Box::new(stream) as Box<dyn Read + Send>);
                    return Ok(VyomaConnection { reader, writer });
                }
                Err(e) if e.kind() == io::ErrorKind::ConnectionRefused => {
                    if Instant::now() >= deadline {
                        return Err(anyhow::anyhow!(
                            "connection refused after retries: {e}"
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(delay_ms));
                    delay_ms = (delay_ms * 2).min(1600);
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("connect error: {e}"));
                }
            }
        }
    }

    /// Connect to the supervisor over a Unix domain socket.
    ///
    /// Used when `--socket` flag or `VYOMA_SOCKET` env var is set.
    #[cfg(unix)]
    pub fn connect_socket(path: &Path) -> anyhow::Result<Self> {
        use std::os::unix::net::UnixStream;
        let stream = UnixStream::connect(path)
            .map_err(|e| anyhow::anyhow!("socket connect {}: {e}", path.display()))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let writer: Box<dyn Write + Send> = Box::new(stream.try_clone()?);
        let reader = BufReader::new(Box::new(stream) as Box<dyn Read + Send>);
        Ok(VyomaConnection { reader, writer })
    }

    /// Fallback for non-Unix platforms: always returns an error.
    #[cfg(not(unix))]
    pub fn connect_socket(_path: &Path) -> anyhow::Result<Self> {
        Err(anyhow::anyhow!("Unix socket transport is not supported on this platform"))
    }

    /// Serialize `req` as an NDJSON line and write it to the server.
    pub fn send(&mut self, req: &MgmtRequest) -> anyhow::Result<()> {
        let line = serde_json::to_string(req)?;
        self.writer.write_all(line.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    /// Read one NDJSON line from the server and deserialize it.
    pub fn recv(&mut self) -> anyhow::Result<MgmtResponse> {
        let mut line = String::new();
        let n = self.reader.read_line(&mut line)?;
        if n == 0 {
            return Err(anyhow::anyhow!("connection closed by server"));
        }
        let resp: MgmtResponse = serde_json::from_str(line.trim_end())
            .map_err(|e| anyhow::anyhow!("protocol error: {e}: {}", line.trim_end()))?;
        Ok(resp)
    }

    /// Write raw bytes directly to the connection (used for binary push data).
    pub fn write_raw(&mut self, data: &[u8]) -> anyhow::Result<()> {
        self.writer.write_all(data)?;
        self.writer.flush()?;
        Ok(())
    }
}
