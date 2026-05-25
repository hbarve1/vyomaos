# Data Model: VyomaOS Developer Tooling (vyoma CLI)

**Feature**: `specs/044-developer-tooling/spec.md`
**Date**: 2026-05-25

---

## Entities

### VyomaConnection

A handle to a connected VyomaOS management server. Abstracts both TCP and QEMU-socket transports behind a uniform read/write interface.

**Fields**:
| Field | Type | Description |
|-------|------|-------------|
| `transport` | `Transport` | Active transport variant (TCP or UnixSocket) |
| `reader` | `BufReader<Box<dyn Read>>` | Line-buffered reader for incoming NDJSON |
| `writer` | `Box<dyn Write>` | Writer for outgoing NDJSON requests |

**Transport enum**:
```rust
enum Transport {
    Tcp(TcpStream),
    Socket(UnixStream),
}
```

**State transitions**:
```
Disconnected → connect() → Connected → send_request() → AwaitingResponse
AwaitingResponse → read_line() → Connected (or Disconnected on EOF)
```

**Validation rules**:
- TCP connect timeout: 10 seconds max (spec edge-case: supervisor not yet started)
- Retry with exponential backoff for up to 10 seconds on connection refused
- CLI exits with non-zero code and stderr message if connection cannot be established

**Created by**: `VyomaConnection::connect(host, port)` or `VyomaConnection::connect_socket(path)`

---

### AppHandle

A snapshot of a running (or recently stopped) app as seen from the CLI. Populated from `ps_row` NDJSON responses and updated from `heartbeat` stream events.

**Fields**:
| Field | Type | Description |
|-------|------|-------------|
| `name` | `String` | App identifier (matches vyoma.toml `[app].name`) |
| `status` | `AppStatus` | Current lifecycle state |
| `uptime_s` | `u64` | Seconds since app was last started |
| `restarts` | `u32` | Total restart count since supervisor boot |
| `mem_kb` | `u64` | RSS memory usage in kilobytes (0 if unknown) |
| `last_heartbeat_s` | `Option<u64>` | Seconds since last heartbeat (None = never received) |
| `last_error` | `Option<String>` | Most recent error string from heartbeat (None = clean) |

**AppStatus enum**:
```rust
enum AppStatus {
    Running,
    Stopped,
    Degraded,   // heartbeat received but status field = "degraded"
    Unhealthy,  // missed heartbeat deadline
}
```

**Display representation** (for `vyoma ps`):
```
NAME          STATUS     UPTIME       RESTARTS
notes         running    2h 14m 32s   0
shell         running    2h 14m 31s   0
http-server   degraded   45m 12s      3
```

---

### HeartbeatStream

A continuous sequence of heartbeat JSON-line events received from the supervisor. Consumed by `vyoma monitor` to maintain a live `HashMap<String, AppHandle>`.

**Wire format** (one JSON line per heartbeat event):
```json
{"type":"heartbeat","module":"notes","uptime_s":42,"mem_kb":128,"status":"healthy","last_error":""}
```

**Fields** (per event):
| Field | Type | Description |
|-------|------|-------------|
| `module` | `String` | App name |
| `uptime_s` | `u64` | App uptime in seconds |
| `mem_kb` | `u64` | Memory usage |
| `status` | `String` | `"healthy"` \| `"degraded"` \| `"unhealthy"` |
| `last_error` | `String` | Error string or empty |

**Consumed by**: `HeartbeatStream::next_event()` in a loop inside `commands/monitor.rs`

**Behavior on reconnect**: If the TCP connection drops while streaming, `vyoma monitor` prints a reconnect notice and calls `VyomaConnection::connect()` again, then re-issues `{"type":"heartbeat_stream"}`.

---

### PushSession

Represents an in-flight OTA push operation. Created by `vyoma push`, lives for the duration of the health check window.

**Fields**:
| Field | Type | Description |
|-------|------|-------------|
| `app_name` | `String` | Target app identifier |
| `binary_path` | `PathBuf` | Local path to the .wasm file |
| `sha256` | `Option<String>` | Expected SHA-256 hash (from `--sha256` flag) |
| `start_time` | `Instant` | Time push command was issued |

**State machine**:
```
Pending → Transferring → HealthCheck → Committed (exit 0)
                                    → RolledBack (exit 1)
Pending → TransferError (exit 1)
HealthCheck → Timeout (exit 1, only if supervisor health check exceeds configured window)
```

**Progress output** (one line per ≤10 seconds per FR-007):
```
[0s] Transferring my-app.wasm (12.3 KB)...
[1s] Transfer complete. Health check starting...
[11s] Health check in progress (10s elapsed)... status: healthy
[21s] Health check in progress (20s elapsed)... status: healthy
[28s] SUCCESS: my-app committed to slot B
```

---

## NDJSON Message Type Catalog

All messages exchanged over the management TCP connection (wire format reference).

### CLI → Supervisor (Requests)

| `type` | Additional Fields | Description |
|--------|------------------|-------------|
| `ps` | — | List all running apps |
| `logs` | `app: String` | Stream log lines for named app |
| `push_start` | `name: String`, `size: u64`, `sha256: Option<String>` | Begin OTA push; binary follows inline |
| `heartbeat_stream` | — | Subscribe to heartbeat events |
| `exec` | `app: String`, `msg: String` | Send IPC message to app |

### Supervisor → CLI (Responses)

| `type` | Additional Fields | Description |
|--------|------------------|-------------|
| `ps_row` | `name`, `status`, `uptime_s`, `restarts`, `mem_kb` | One row per running app |
| `ps_done` | — | End of `ps` response |
| `log_line` | `app: String`, `line: String` | One log line from streaming logs |
| `push_ack` | `state: String` | Progress update: `"transferring"`, `"health_check"` |
| `push_result` | `ok: bool`, `message: String` | Final push outcome |
| `heartbeat` | `module`, `uptime_s`, `mem_kb`, `status`, `last_error` | One heartbeat event |
| `exec_reply` | `reply: String` | App's IPC response |
| `error` | `code: String`, `message: String` | Error response for any request |

### Error Codes

| `code` | Meaning |
|--------|---------|
| `NOT_FOUND` | Named app is not running |
| `OTA_IN_PROGRESS` | Another push is already active |
| `HASH_MISMATCH` | Binary SHA-256 does not match `--sha256` argument |
| `DISK_FULL` | Supervisor rejected binary (no space for slot B) |
| `INVALID_WASM` | Binary failed WASM parse validation |
| `TIMEOUT` | `exec` command timed out waiting for app reply |
| `INTERNAL` | Supervisor-side unexpected error |
