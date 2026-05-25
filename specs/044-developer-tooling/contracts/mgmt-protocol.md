# Contract: VyomaOS Management TCP Protocol

**Feature**: `specs/044-developer-tooling/spec.md`
**Date**: 2026-05-25

This document defines the wire protocol between the `vyoma` CLI (client) and the VyomaOS supervisor management server (server). Both sides must implement this contract exactly.

---

## Transport

### TCP

- **Default port**: `9090`
- **Bind address**: `0.0.0.0` (accepts local and remote connections)
- **TLS**: None (v1; authentication is out of scope per spec Assumptions)
- **Concurrency**: Server accepts unlimited concurrent connections; each connection handled on a dedicated thread

### QEMU Unix Socket

- **Socket path**: `/tmp/vyoma-mgmt.sock` (host-side), mapped to `/dev/ttyS1` inside VM
- **Protocol**: Same NDJSON as TCP
- **QEMU flags**: `-chardev socket,id=mgmt,path=/tmp/vyoma-mgmt.sock,server=on,wait=off -device virtio-serial -chardev ... -serial chardev:mgmt`

---

## Encoding

- **Format**: NDJSON — one JSON object per line, terminated with `\n`
- **Character set**: UTF-8
- **Max line length**: 65,536 bytes (to accommodate large error messages and binary metadata)
- **Framing**: Lines are delimited by `\n` (LF). `\r\n` (CRLF) is accepted on input, emitted as `\n` on output

---

## Session Lifecycle

```
Client connects
  → sends one REQUEST line
  → reads zero or more RESPONSE lines until:
      (a) a terminal response is received (ps_done, push_result, exec_reply, error), OR
      (b) an open-ended stream (logs, heartbeat_stream) until client disconnects
Client disconnects (or server closes connection after terminal response for non-streaming commands)
```

For streaming commands (`logs`, `heartbeat_stream`), the server streams indefinitely until the client closes the TCP connection.

---

## Request Messages (CLI → Supervisor)

### `ps` — List running apps

```json
{"type":"ps"}
```

**Response**: Zero or more `ps_row` lines, followed by exactly one `ps_done`.

---

### `logs` — Stream app logs

```json
{"type":"logs","app":"notes"}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `app` | string | yes | App name to stream logs from |

**Response**: Stream of `log_line` messages until client disconnects. If app is not found, server sends `error` with code `NOT_FOUND` and closes.

---

### `push_start` — Begin OTA push

```json
{"type":"push_start","name":"notes","size":12345,"sha256":"abc123..."}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `name` | string | yes | Target app name |
| `size` | u64 | yes | Binary size in bytes |
| `sha256` | string | no | Expected SHA-256 hex digest |

**Immediately after** sending `push_start`, client streams exactly `size` bytes of raw binary data (not JSON). Server reads `size` bytes, then resumes NDJSON line parsing.

**Response**: Stream of `push_ack` lines, followed by exactly one `push_result`. Server closes after `push_result`.

---

### `heartbeat_stream` — Subscribe to heartbeat events

```json
{"type":"heartbeat_stream"}
```

**Response**: Continuous stream of `heartbeat` messages until client disconnects.

---

### `exec` — Send IPC message to app

```json
{"type":"exec","app":"pong","msg":"hello","timeout_ms":5000}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `app` | string | yes | Target app name |
| `msg` | string | yes | IPC message payload |
| `timeout_ms` | u64 | no | Milliseconds to wait for reply (default: 5000) |

**Response**: Exactly one `exec_reply` or `error`.

---

## Response Messages (Supervisor → CLI)

### `ps_row`

```json
{"type":"ps_row","name":"notes","status":"running","uptime_s":42,"restarts":0,"mem_kb":128}
```

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | App name |
| `status` | string | `"running"` \| `"stopped"` \| `"degraded"` \| `"unhealthy"` |
| `uptime_s` | u64 | Seconds since last start |
| `restarts` | u32 | Total restart count |
| `mem_kb` | u64 | Memory usage (0 if not available) |

### `ps_done`

```json
{"type":"ps_done"}
```

Terminal response for `ps`. Server closes connection after sending.

### `log_line`

```json
{"type":"log_line","app":"notes","line":"hello world"}
```

| Field | Type | Description |
|-------|------|-------------|
| `app` | string | Source app name |
| `line` | string | One line of output (without trailing newline) |

### `push_ack`

```json
{"type":"push_ack","state":"transferring","elapsed_s":0}
{"type":"push_ack","state":"health_check","elapsed_s":1,"health_status":"healthy"}
```

| Field | Type | Description |
|-------|------|-------------|
| `state` | string | `"transferring"` \| `"health_check"` |
| `elapsed_s` | u64 | Seconds since push began |
| `health_status` | string? | Present during `health_check`: `"healthy"` \| `"degraded"` |

Server must emit at least one `push_ack` per 10 seconds during the health check window (FR-007).

### `push_result`

```json
{"type":"push_result","ok":true,"message":"committed to slot B"}
{"type":"push_result","ok":false,"message":"health check failed, rolled back to slot A"}
```

| Field | Type | Description |
|-------|------|-------------|
| `ok` | bool | `true` = committed, `false` = rolled back or error |
| `message` | string | Human-readable outcome |

Terminal response for `push_start`. Server closes connection after sending.

### `heartbeat`

```json
{"type":"heartbeat","module":"notes","uptime_s":42,"mem_kb":128,"status":"healthy","last_error":""}
```

| Field | Type | Description |
|-------|------|-------------|
| `module` | string | App name |
| `uptime_s` | u64 | App uptime in seconds |
| `mem_kb` | u64 | Memory usage in kilobytes |
| `status` | string | `"healthy"` \| `"degraded"` \| `"unhealthy"` |
| `last_error` | string | Error string or empty string |

### `exec_reply`

```json
{"type":"exec_reply","reply":"pong response payload"}
```

| Field | Type | Description |
|-------|------|-------------|
| `reply` | string | App's IPC reply message |

Terminal response for `exec`.

### `error`

```json
{"type":"error","code":"NOT_FOUND","message":"app 'foo' is not running"}
```

| Field | Type | Description |
|-------|------|-------------|
| `code` | string | Machine-readable error code (see codes table) |
| `message` | string | Human-readable description |

**Error codes**:
| Code | HTTP analogy | Meaning |
|------|-------------|---------|
| `NOT_FOUND` | 404 | Named app is not running |
| `OTA_IN_PROGRESS` | 409 | Another push is already active |
| `HASH_MISMATCH` | 422 | Binary SHA-256 does not match expectation |
| `DISK_FULL` | 507 | No space for slot B binary |
| `INVALID_WASM` | 422 | Binary failed WASM parse validation |
| `TIMEOUT` | 408 | `exec` timed out waiting for app reply |
| `INTERNAL` | 500 | Unexpected supervisor-side error |

Terminal response for any command. Server closes connection after sending.

---

## Versioning

Protocol version is not included in v1 messages. Future versions may add a `{"type":"hello","version":2}` handshake. Implementations MUST ignore unknown `type` values rather than failing.
