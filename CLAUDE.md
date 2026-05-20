# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What is VyomaOS?

A **WASM-first operating system** with the long-term goal of becoming a lightweight but fully capable general-purpose OS built from the ground up on a capability-secure WebAssembly foundation.

**Core architecture**: Every application is a `wasm32-wasip2` binary executed by Wasmtime under a Rust PID 1 supervisor. The Linux kernel handles hardware only. Capabilities (filesystem, network, display, stdio) are declared per-app in a manifest and enforced at runtime.

Current state (Phase 17): Boots in QEMU under 5 seconds to a Rust supervisor running 10 concurrent WASM apps with a live GUI dashboard, interactive shell, HTTP server, and real-time keyboard input.

## High-Level Architecture

### System Stack

```
Linux 5.10 kernel (allnoconfig, 2.3 MB)
  ↓
Rust supervisor (PID 1, 697 KB static musl)
  ├─ Manifest parser (TOML capabilities)
  ├─ Concurrent scheduler (one thread per app)
  ├─ IPC broker (route @<app>: messages)
  ├─ Framebuffer driver (DRM/virtio-gpu + VYOMA_DRAW protocol)
  ├─ TTY input router (raw mode, per-keypress dispatch)
  └─ Process manager (ps, kill, restart, reload, log)
      ↓
Wasmtime runtime (WASI Preview 2)
  ↓
WASM apps (wasm32-wasip2 binaries, 1–10 KB each)
```

### Key Components

**Supervisor** (`supervisor/src/main.rs`):
- Parses `/etc/vyoma/boot.toml` at startup
- Reads app manifests (vyoma.toml) to determine capabilities
- Spawns one wasmtime child process per app with only declared WASI imports
- Routes app lifecycle events, IPC messages, keyboard input
- Applies seccomp BPF denylist to all wasmtime children

**App Model** (`apps/*/vyoma.toml`):
- Every WASM app declares capabilities: stdio, filesystem, network, display, shell, mouse
- Capabilities not declared are not wired up (no filtering layer needed)
- Restart policies: `never` (one-shot), `always` (restart on exit)

**Display System** (`supervisor/src/display.rs`):
- Opens `/dev/fb0` (or virtio-gpu framebuffer)
- Parses `VYOMA_DRAW:` protocol from app stdout
- Commands: `fill_rect`, `draw_text`, `flush`
- 8×16 bitmap font rendering (`supervisor/src/font.rs`)

**IPC Broker**:
- Apps write `@<app>: <message>` to stdout
- Supervisor intercepts, routes to target app stdin
- Enables ping/pong, shell commands, cross-app communication

**Persistent Storage**:
- Host `data/` directory mounted via 9P virtio at `/data` in VM
- Apps with `filesystem = true` access `/data`
- Files survive VM reboots
- Package manager persists via `/data/installed.txt`

### Build System

**Docker-based hermetic builds** (all compilation in container):
- Makefile orchestrates: kernel build → supervisor → WASM apps → rootfs → QEMU boot
- Base image: Ubuntu 22.04 with kernel build deps, musl-tools, Rust (stable)
- Targets: `x86_64-unknown-linux-musl` (supervisor), `wasm32-wasip2` (apps)
- SHA-256 verification for Wasmtime (43.0.0) and BusyBox (1.35.0) downloads

**Build artifacts**:
- `out/bzImage`: Linux kernel (2.3 MB)
- `out/initramfs.cpio.gz`: Compressed rootfs (18 MB) with Wasmtime + BusyBox + supervisor
- `out/disk.img`: ext4 data disk (64 MB, created once, persists across reboots)
- `supervisor/target/x86_64-unknown-linux-musl/release/supervisor`: Binary
- `apps/*/target/wasm32-wasip2/release/*.wasm`: App binaries

## Common Commands

### Build

```bash
make build              # Full build: kernel + supervisor + apps + rootfs + disk
make kernel             # Just compile Linux kernel
make supervisor         # Compile supervisor (Rust)
make apps               # Compile all WASM apps
make rootfs             # Create initramfs from supervisor + apps + busybox
```

### Run

```bash
make run                # Boot headless (serial console only)
make run-gui DISPLAY_BACKEND=cocoa   # macOS: virtio-gpu display
make run-gui DISPLAY_BACKEND=sdl     # Linux: virtio-gpu display
make run-net            # Headless + virtio-net (port 8080 forwarded)
make run-gui-net        # Display + networking
```

### Development

```bash
make shell              # Open bash inside builder container (for manual cargo builds)
make clean              # Remove out/ directory
make clean-image        # Remove Docker builder image
make image              # Rebuild Docker image (if Dockerfile changes)
```

### Supervisor Development

```bash
cd supervisor
cargo build --target x86_64-unknown-linux-musl --release
cd ..
make rootfs && make run
```

### App Development (single app)

```bash
cd apps/my-app
cargo build --target wasm32-wasip2 --release
cd ../..
make rootfs && make run
```

### Debugging in QEMU

**Inside the VM** (at supervisor console):
```
ps                      # List running apps
log <name>              # Print app logs
logf <name>             # Tail app logs (follow mode)
kill <name>             # Terminate app
restart <name>          # Restart app
@supervisor: list       # List apps (IPC command)
@supervisor: focus <name>   # Switch keyboard focus
```

**Manifest validation**: Supervisor prints capability errors at startup if vyoma.toml is invalid (unknown fields, parse errors).

## Creating a New App

1. Create directory: `mkdir apps/my-app`
2. Populate `Cargo.toml`:
   ```toml
   [package]
   name    = "my-app"
   version = "0.1.0"
   edition = "2021"
   
   [[bin]]
   name = "my-app"
   path = "src/main.rs"
   
   [dependencies]
   # Keep minimal for small binary size
   ```
3. Create `src/main.rs` with your code
4. Create `vyoma.toml`:
   ```toml
   [app]
   name    = "my-app"
   version = "0.1.0"
   wasm    = "my-app.wasm"
   
   [capabilities]
   stdio = true
   # Add others as needed: filesystem, network, display, shell, mouse
   ```
5. Build: `cargo build --target wasm32-wasip2 --release`
6. Update `rootfs.sh` to include your .wasm binary in initramfs (copy `hello-world` pattern)

## App Manifest Capabilities

Every `vyoma.toml` declares capabilities; undeclared capabilities are not accessible:

| Capability | Effect |
|-----------|--------|
| `stdio` | App inherits supervisor stdin/stdout (can write to console, receive keyboard input) |
| `filesystem` | App mounts `/data` (9P, persistent storage on host) |
| `network` | App gets WASI sockets support (TCP, port 8080 default) |
| `display` | App accesses `/dev/fb0` (framebuffer) + can use `VYOMA_DRAW:` protocol |
| `shell` | App can issue `@supervisor:` commands (process management) |
| `mouse` | App receives `VYOMA_INPUT:mouse:` events when cursor is in window |
| `watchdog_secs` | Supervisor kills app if silent for N seconds (0 = disabled) |

## Display Protocol (VYOMA_DRAW)

Apps write commands to stdout in format `VYOMA_DRAW:<cmd>`:

```
VYOMA_DRAW:fill_rect:x:y:w:h:r:g:b
VYOMA_DRAW:draw_text:x:y:r:g:b:text
VYOMA_DRAW:flush
```

Example (Rust):
```rust
println!("VYOMA_DRAW:fill_rect:0:0:100:100:255:0:0");  // Red rect
println!("VYOMA_DRAW:draw_text:10:10:255:255:255:Hello");  // White text
println!("VYOMA_DRAW:flush");  // Render to framebuffer
```

## IPC Message Format

Apps communicate via supervisor IPC broker. Format: `@<target_app>: <message>`

Example (send message from one app to another):
```rust
println!("@pong: hello from ping");  // Send to pong app
```

Receiving app reads stdin line-by-line; supervisor strips `@sender:` prefix before delivery.

## File Structure

- `supervisor/`: Rust PID 1, manages app lifecycle + IPC + display
- `apps/`: WASM applications (ping, pong, calculator, shell, http-server, gui-demo, etc.)
- `base/`: Kernel config + rootfs build scripts
- `docker/`: Dockerfile for hermetic build environment
- `docs/`: Manifest schema, comparison matrix, design docs for future phases
- `.context/plans/`: Phased implementation roadmap (P01–P17 complete, P18+ planned)

## Key Design Decisions

**Capability-secure by default**: Supervisor does not filter app syscalls; it only wires up the WASI imports declared in vyoma.toml. This means:
- If an app doesn't declare `network = true`, there is no network interface to syscall
- If an app doesn't declare `filesystem = true`, there is no `/data` mount to access
- No separate filtering layer (seccomp, AppArmor, SELinux) needed

**Deterministic binary output**: WASM bytecode is byte-identical across builds and hosts (unlike ELF binaries which vary by libc/architecture). This enables reproducible deployments.

**Minimal kernel**: Linux kernel compiled with allnoconfig + only drivers VyomaOS actually uses (virtio, 9P, DRM, fbcon). No networking stack, no filesystem drivers beyond 9P, no USB. Supervisor handles all high-level policy.

**Supervisor-side IPC**: Apps don't directly communicate; supervisor brokers all messages. This centralizes routing logic and enables future debugging/monitoring features.

## Testing Strategy

No formal test suite yet. Manual testing via `make run` / `make run-gui`.

**Smoke test**: Boot and verify supervisor starts all apps without manifest errors:
```bash
make build && make run-gui 2>&1 | tee boot.log
# Check boot.log for "app started: <name>" messages
```

## Phase Overview

| Phase | Feature | Status |
|-------|---------|--------|
| P01–P08 | Kernel, supervisor, manifest model, IPC, seccomp, storage | ✅ complete |
| P09–P10 | Display (DRM/virtio-gpu) + bitmap font | ✅ complete |
| P11–P12 | Networking (HTTP server) + interactive shell + keyboard routing | ✅ complete |
| P13–P17 | Process management, package manager, persistent logs, real-time TTY | ✅ complete |
| P18+ | Future phases (font scaling, windowing, mouse input, watchdog) | 📋 planned |

See `.context/plans/plan-vyomaos/` for detailed phase specs and task breakdowns.

## Troubleshooting

**Docker build fails**: Run `make image` first to build the vyomaos-builder Docker image.

**Supervisor won't start**: Check boot.toml syntax (rootfs.sh generates it). Verify manifest files exist at paths listed in boot.toml.

**GUI window is black**: 
- Ensure app has `display = true` in vyoma.toml
- Check app calls `VYOMA_DRAW:flush` after drawing
- Look for parse errors in QEMU serial console

**App exits immediately**: Check logs via `make shell` and inspecting `/work/out/initramfs.cpio.gz` contents, or add debug output to supervisor stderr.

**IPC messages not routing**: Format is `@<app>: <message>`. Target app must be running. No newlines allowed in payload.

**Networking not working**: Requires `network = true` in app's vyoma.toml AND boot with `make run-net` / `make run-gui-net`.

## References

- **README.md**: Project overview, vision, roadmap
- **docs/vyoma-manifest-schema.md**: Detailed manifest format and examples
- **docs/comparison-matrix.md**: VyomaOS vs Alpine, Flatcar, MirageOS positioning
- **supervisor/src/main.rs**: Manifest parsing, app scheduler, IPC broker logic
- **.context/plans/**: Implementation roadmap (phases, tasks, design specs)

