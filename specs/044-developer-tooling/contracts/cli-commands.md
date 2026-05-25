# Contract: vyoma CLI Command Interface

**Feature**: `specs/044-developer-tooling/spec.md`
**Date**: 2026-05-25

This document defines the public interface contract for the `vyoma` CLI binary. Every command, flag, environment variable, exit code, and output format described here must be implemented exactly as specified.

---

## Global Flags (all commands)

| Flag | Short | Env Var | Default | Description |
|------|-------|---------|---------|-------------|
| `--host <addr>` | — | `VYOMA_HOST` | `localhost` | VyomaOS instance hostname or IP |
| `--port <n>` | — | `VYOMA_PORT` | `9090` | TCP port for management server |
| `--socket <path>` | — | `VYOMA_SOCKET` | — | QEMU Unix socket path (overrides TCP) |

When `--socket` is provided (or `VYOMA_SOCKET` is set), TCP flags are ignored.

---

## `vyoma ps`

**Purpose**: List all running apps on the connected VM.

**Usage**: `vyoma [global-flags] ps`

**Output** (stdout, tab-aligned columns):
```
NAME          STATUS     UPTIME       RESTARTS
notes         running    2h 14m 32s   0
shell         running    2h 14m 31s   0
http-server   degraded   45m 12s      3
```

Column definitions:
- `NAME`: app name (string, max 20 chars in display)
- `STATUS`: `running` | `stopped` | `degraded` | `unhealthy`
- `UPTIME`: human-readable duration (`Xd Xh Xm Xs`, omit zero-value leading units)
- `RESTARTS`: unsigned integer

**Exit codes**:
- `0`: success (list printed, even if empty)
- `1`: connection failure
- `2`: protocol error

**Timing**: Must complete within 1 second for up to 20 apps (SC-003).

---

## `vyoma logs <app>`

**Purpose**: Stream live stdout/stderr from the named app to the host terminal.

**Usage**: `vyoma [global-flags] logs <app>`

**Positional arguments**:
- `<app>`: app name string (required)

**Additional flags**:
| Flag | Default | Description |
|------|---------|-------------|
| `--follow` / `-f` | true | Follow log stream (default behavior, flag reserved for future `--no-follow`) |

**Output** (stdout, one line per log event):
```
[notes] hello world
[notes] another line
```

Prefix format: `[<app-name>] ` (with a trailing space after `]`).

**Behavior on app restart**: Continues streaming without CLI restart (FR-003). Prints `[notes] --- app restarted ---` when a restart is detected (uptime reset in heartbeat).

**Exit codes**:
- `0`: user pressed Ctrl+C (clean exit)
- `1`: connection failure
- `3`: named app not found (error to stderr, immediate exit)

**Timing**: Each line must appear within 2 seconds of emission (SC-002).

---

## `vyoma push <path>`

**Purpose**: Transfer a WASM binary to the VM and trigger an OTA A/B slot update.

**Usage**: `vyoma [global-flags] push <path> [--name <app>] [--sha256 <hash>]`

**Positional arguments**:
- `<path>`: local path to the `.wasm` file (required)

**Additional flags**:
| Flag | Default | Description |
|------|---------|-------------|
| `--name <app>` | filename without extension | Target app name |
| `--sha256 <hash>` | — | Expected SHA-256 hex digest for binary verification |

**Output** (stdout, progress lines):
```
Pushing my-app.wasm (12.3 KB) → notes
[0s] Transferring...
[1s] Transfer complete. Health check starting...
[11s] Health check in progress (10s elapsed)... status: healthy
[21s] Health check in progress (20s elapsed)... status: healthy
[28s] SUCCESS: notes committed to slot B
```

On rollback:
```
[28s] ROLLBACK: notes reverted to slot A (health check failed)
```

**Exit codes**:
- `0`: binary committed to slot B (health check passed)
- `1`: connection failure
- `2`: transfer rejected (hash mismatch, disk full, invalid WASM) — reason on stderr
- `3`: health check failed, rollback occurred — message on stderr

**Timing**: First progress line must appear within 5 seconds of invocation (SC-001).

---

## `vyoma monitor`

**Purpose**: Live terminal dashboard showing all app heartbeats, refreshed every 2 seconds.

**Usage**: `vyoma [global-flags] monitor`

**Output** (full-screen terminal, clears and redraws every 2 seconds):
```
VyomaOS Monitor — localhost:9090                    2026-05-25 18:42:01

  NAME          STATUS     UPTIME       MEM (KB)   LAST HB
  notes         healthy    2h 14m 32s   128        0s ago
  shell         healthy    2h 14m 31s   96         1s ago
  http-server   degraded   45m 12s      256        3s ago
  geo-quiz      unhealthy  2h 10m 00s   0          45s ago

Press q or Ctrl+C to exit
```

Column definitions:
- `NAME`: app name
- `STATUS`: `healthy` | `degraded` | `unhealthy` (from heartbeat `status` field)
- `UPTIME`: human-readable duration
- `MEM (KB)`: memory in kilobytes from heartbeat `mem_kb`
- `LAST HB`: seconds since last heartbeat received (e.g., `0s ago`, `5s ago`)

**Status coloring** (crossterm, if terminal supports color):
- `healthy` → green
- `degraded` → yellow
- `unhealthy` → red

**Terminal cleanup**: On exit (`q` or Ctrl+C), restore terminal cursor and clear the dashboard (FR-008 requirement to restore prior state).

**Exit codes**:
- `0`: clean exit via `q` or Ctrl+C
- `1`: connection failure

**Timing**: Status change reflected within 4 seconds (SC-004): 2s refresh + ≤2s network latency.

---

## `vyoma exec <app> "<message>"`

**Purpose**: Send an IPC message to a running app and print the reply.

**Usage**: `vyoma [global-flags] exec <app> <message> [--timeout <secs>]`

**Positional arguments**:
- `<app>`: target app name (required)
- `<message>`: IPC message string (required, quoted if it contains spaces)

**Additional flags**:
| Flag | Default | Description |
|------|---------|-------------|
| `--timeout <secs>` | `5` | Seconds to wait for app reply |

**Output** (stdout):
```
pong
```
(raw reply string from the app, with a trailing newline)

**Exit codes**:
- `0`: reply received and printed
- `1`: connection failure
- `3`: named app not found
- `4`: timeout — no reply received within `--timeout` seconds

**Timing**: Reply printed within 2 seconds for synchronously responding apps (SC-005).

---

## Exit Code Summary

| Code | Meaning |
|------|---------|
| `0` | Success |
| `1` | Connection / transport failure |
| `2` | Protocol or validation error (hash mismatch, disk full, invalid WASM) |
| `3` | App not found |
| `4` | Timeout |

All error messages go to **stderr**. Stdout is reserved for structured output.
