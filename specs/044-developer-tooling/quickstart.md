# Quickstart: VyomaOS Developer Tooling (vyoma CLI)

**Feature**: `specs/044-developer-tooling/spec.md`
**Date**: 2026-05-25

---

## Prerequisites

- VyomaOS running in QEMU with management server enabled (port 9090 forwarded to host)
- Rust toolchain on host (`cargo build` for CLI)
- A compiled `.wasm` app binary to push

---

## Install the CLI

```bash
cd cli/
cargo build --release
# Binary at: cli/target/release/vyoma
# Optionally add to PATH:
export PATH="$PWD/cli/target/release:$PATH"
```

---

## Boot VyomaOS with Port Forwarding

```bash
make run-net
# Supervisor management server listens on :9090 inside VM
# QEMU forwards host:9090 → guest:9090
```

Alternatively, for GUI + networking:
```bash
make run-gui-net
```

---

## Test the Connection

```bash
vyoma ps
```

Expected output:
```
NAME          STATUS     UPTIME       RESTARTS
notes         running    0h 00m 12s   0
shell         running    0h 00m 11s   0
http-server   running    0h 00m 11s   0
```

---

## Deploy a New App Version

```bash
# Build updated app
cd apps/notes
cargo build --target wasm32-wasip2 --release
cd ../..

# Push to running VM
vyoma push apps/notes/target/wasm32-wasip2/release/notes.wasm
```

Expected output:
```
Pushing notes.wasm (8.4 KB) → notes
[0s] Transferring...
[1s] Transfer complete. Health check starting...
[11s] Health check in progress (10s elapsed)... status: healthy
[30s] SUCCESS: notes committed to slot B
```

With SHA-256 verification:
```bash
SHA=$(sha256sum apps/notes/target/wasm32-wasip2/release/notes.wasm | awk '{print $1}')
vyoma push apps/notes/target/wasm32-wasip2/release/notes.wasm --sha256 "$SHA"
```

---

## Stream App Logs

```bash
vyoma logs notes
```

Expected output (live stream):
```
[notes] starting up
[notes] initialized state
[notes] waiting for input...
```

Press Ctrl+C to stop.

---

## Monitor All Apps

```bash
vyoma monitor
```

Expected output (refreshes every 2 seconds):
```
VyomaOS Monitor — localhost:9090                    2026-05-25 18:42:01

  NAME          STATUS     UPTIME       MEM (KB)   LAST HB
  notes         healthy    0h 00m 42s   128        0s ago
  shell         healthy    0h 00m 41s   96         1s ago
  http-server   healthy    0h 00m 41s   192        1s ago

Press q or Ctrl+C to exit
```

---

## Send an IPC Message

```bash
vyoma exec pong "hello"
```

Expected output:
```
pong
```

With timeout override:
```bash
vyoma exec slow-app "compute" --timeout 30
```

---

## Connect via QEMU Socket (Local Dev)

Instead of TCP port forwarding, connect via the QEMU Unix socket (no port forwarding required):

```bash
# Boot with management socket enabled:
make run QEMU_EXTRA="-chardev socket,id=mgmt,path=/tmp/vyoma-mgmt.sock,server=on,wait=off -serial chardev:mgmt"

# Use the socket:
vyoma --socket /tmp/vyoma-mgmt.sock ps
vyoma --socket /tmp/vyoma-mgmt.sock logs notes
```

---

## Environment Variables

All global flags can be set via environment variables for convenience:

```bash
export VYOMA_HOST=192.168.1.100
export VYOMA_PORT=9090
# or
export VYOMA_SOCKET=/tmp/vyoma-mgmt.sock

# Now all commands use these settings:
vyoma ps
vyoma logs notes
```

---

## Independent Test Scenarios

### User Story 1 — OTA Push (P1)

1. Build v1 of an app that prints "hello v1" on stdout
2. Boot VyomaOS with v1 loaded; verify `vyoma ps` shows it running
3. Modify app to print "hello v2"; rebuild
4. Run `vyoma push v2.wasm`; verify it commits (exit 0)
5. Run `vyoma logs <app>`; verify "hello v2" appears

### User Story 2 — Logs & ps (P2)

1. Run a WASM app that prints a timestamped line every second
2. Run `vyoma logs <app>`; verify lines appear within 2 seconds
3. Run `vyoma ps`; verify all running apps appear with uptime and restart count

### User Story 3 — Monitor Dashboard (P3)

1. Boot VyomaOS with 3+ apps
2. Run `vyoma monitor`; verify all apps appear
3. Force a watchdog timeout on one app (or kill it manually via `@supervisor: kill`)
4. Verify dashboard reflects the status change within the next 2-second refresh

### User Story 4 — exec (P4)

1. Run the `pong` app
2. Run `vyoma exec pong "hello"`; verify reply appears
3. Run `vyoma exec nonexistent-app "hi"`; verify non-zero exit and error message
