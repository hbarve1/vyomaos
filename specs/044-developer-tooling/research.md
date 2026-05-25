# Research: VyomaOS Developer Tooling (vyoma CLI)

**Feature**: `specs/044-developer-tooling/spec.md`
**Date**: 2026-05-25
**Branch**: `044-developer-tooling`

---

## Decision 1: Wire Protocol

**Decision**: NDJSON (newline-delimited JSON) over TCP, with a `type` discriminator field in every message.

**Rationale**:
- Human-readable during development (can `telnet localhost 9090` for ad-hoc debugging)
- No external schema dependency; serde_json handles encoding/decoding
- Line-oriented: each request/response is one `\n`-terminated line; trivial to stream
- Trivially extensible: add a new `type` value without a schema migration
- Matches the heartbeat format already established in `docs/observability.md`

**Protocol shape**:
```json
// request
{"type":"ps"}
{"type":"logs","app":"notes"}
{"type":"push","name":"notes","size":12345,"sha256":"abc..."}
{"type":"exec","app":"pong","msg":"hello"}
{"type":"heartbeat_stream"}

// response (one or more lines per request)
{"type":"ps_row","name":"notes","status":"running","uptime_s":42,"restarts":0}
{"type":"ps_done"}
{"type":"log_line","app":"notes","line":"hello world"}
{"type":"push_ack","state":"transferring"}
{"type":"push_result","ok":true,"message":"committed to slot B"}
{"type":"exec_reply","reply":"pong"}
{"type":"heartbeat","module":"notes","uptime_s":42,"mem_kb":128,"status":"healthy","last_error":""}
{"type":"error","code":"NOT_FOUND","message":"app 'foo' is not running"}
```

**Alternatives considered**:
- Binary framing (length-prefix + msgpack): Lower overhead but opaque; debugging requires tooling
- gRPC/protobuf: Requires protoc toolchain, heavy dependency for a 10 MB CLI
- Plain text commands: Already used for supervisor IPC internally, but not structured enough for CLI parsing

---

## Decision 2: Supervisor Management Server

**Decision**: New `supervisor/src/mgmt_server.rs` using `std::net::TcpListener` (no tokio dependency). One thread per client connection, sharing the existing `Arc<Mutex<AppRegistry>>`.

**Rationale**:
- Supervisor is single-binary musl with zero async runtime; adding tokio would increase binary by ~2 MB and complicate the build
- At most 1–2 concurrent CLI clients (developer workflow); per-thread cost is negligible
- `TcpListener::accept()` loop with `thread::spawn` is a dozen lines; matches supervisor's existing threading model (one thread per app)
- `AppRegistry` is already `Arc<Mutex<...>>`; management handlers acquire the lock for read-only queries and release immediately

**Listener address**: `0.0.0.0:9090` by default, configurable via `boot.toml` `[management] port = 9090`

**Thread model**:
```
main thread: spawns MgmtServer thread
MgmtServer thread: TcpListener::bind → loop { accept → spawn handler thread }
handler thread: read NDJSON line → dispatch → write NDJSON response → close (or stream for logs/monitor)
```

**Alternatives considered**:
- Expose management over the existing `/dev/ttyS0` serial: Mixes operator console with CLI; can't multiplex
- Unix domain socket inside QEMU: Would require guest-to-host socket bridge; TCP is simpler for both local and remote VMs
- tokio async: Heavy; not worth it for 1–2 concurrent connections

---

## Decision 3: QEMU Socket Transport

**Decision**: Second serial port mapped to a Unix domain socket on the host: `qemu -chardev socket,id=mgmt,path=/tmp/vyoma-mgmt.sock,server=on,wait=off -device virtio-serial -chardev ... -serial chardev:mgmt`

Inside the VM, this appears as `/dev/ttyS1`. The management server listens on the same NDJSON protocol over this serial device in addition to TCP 9090.

**Rationale**:
- Zero kernel changes: virtio-serial is already in the kernel config
- The socket path is configurable via Makefile / QEMU invocation
- CLI selects transport via `--socket <path>` flag or `VYOMA_SOCKET` env var
- Useful for local development before the VM has a routable IP

**Alternatives considered**:
- QEMU monitor protocol: Requires QEMU-specific commands; not portable to real hardware
- QEMU GDB stub: Debugging only; not suitable for IPC
- 9P virtio file transfer only: Cannot stream logs or monitor in real time

---

## Decision 4: CLI Binary Dependencies

**Decision**: `clap` v4 (minimal features, `derive`) + `crossterm` (not ratatui) + `std::net` (not tokio) + `indicatif` + `serde_json` + `serde`.

**Rationale**:
- `clap v4` with `features = ["derive", "cargo"]` only: ~250 KB contribution to binary
- `crossterm`: Direct terminal control for `vyoma monitor` dashboard; ratatui adds ~150 KB of widget abstractions we don't need
- `std::net::TcpStream` + `BufReader<TcpStream>`: Sufficient for NDJSON line-by-line reading; no async needed
- `indicatif`: Progress spinner for `vyoma push` health-check wait (10-second progress updates per FR-007)
- `serde_json` + `serde derive`: Required for NDJSON encoding/decoding

**Estimated binary sizes** (musl static, stripped, x86-64):
- clap + serde_json + serde: ~600 KB
- crossterm: ~180 KB
- indicatif: ~120 KB
- application code: ~150 KB
- **Total**: ~1.05 MB — well within the 10 MB SC-008 limit

**Alternatives considered**:
- ratatui: Adds ~150 KB + required crossterm anyway; overkill for a 5-column table
- tokio: +2 MB; not needed for sequential CLI operations
- ureq/reqwest: Not needed; CLI communicates directly over TCP, not HTTP

---

## Decision 5: CLI Project Structure

**Decision**: Standalone Rust workspace at `cli/` in the repository root.

```
cli/
├── Cargo.toml          # workspace member
├── src/
│   ├── main.rs         # clap CLI entry point (≤ 200 lines)
│   ├── connection.rs   # VyomaConnection (TCP + socket transport)
│   ├── commands/
│   │   ├── ps.rs
│   │   ├── logs.rs
│   │   ├── push.rs
│   │   ├── monitor.rs
│   │   └── exec.rs
│   └── protocol.rs     # NDJSON message types (serde structs)
```

**Rationale**:
- Keeps CLI outside the supervisor workspace (different target triple: `x86_64-unknown-linux-musl` host vs `wasm32-wasip2` apps)
- Clean separation: supervisor owns the server, CLI owns the client
- Each command file stays under 500 lines
- `protocol.rs` is the single source of truth for message types used by both the supervisor server and the CLI client

**Alternatives considered**:
- In-repo tool via `cargo install --path cli`: Works, but placing in workspace root avoids polluting supervisor's Cargo.lock
- Separate git repository: Overkill; CLI and supervisor must stay in sync on protocol changes

---

## Decision 6: OTA Push Flow

**Decision**: `vyoma push` sends a `{"type":"push_start","name":"foo","size":N,"sha256":"..."}` header, then streams the binary as raw bytes over the same TCP connection, then waits for streaming `push_ack` status lines until `push_result`.

**Supervisor side**: The management handler writes the binary to a temp file under `/data/ota/`, then calls the existing `OtaManager::initiate_update()` which:
1. Copies to slot B
2. Starts health check window (configurable seconds, default 30)
3. Returns health check events as streaming NDJSON lines

**Rationale**:
- Reuses existing OTA A/B slot mechanism from `supervisor/src/ota/`
- Binary transfer over the same connection avoids a second connection or shared filesystem
- Streaming progress lines satisfy FR-007 (status line every ≤10 seconds)

---

## Resolved Unknowns Summary

| Unknown | Resolution |
|---------|-----------|
| Wire protocol | NDJSON over TCP/socket, `type` discriminator |
| Supervisor server | `std::net::TcpListener`, one thread per client, port 9090 |
| QEMU socket transport | `-chardev socket` → `/dev/ttyS1` inside VM |
| CLI dependencies | clap v4 + crossterm + indicatif + serde_json, ~1.05 MB |
| CLI project layout | `cli/` workspace member, commands split by file |
| OTA push binary transfer | Inline binary stream after JSON header, existing OtaManager |
