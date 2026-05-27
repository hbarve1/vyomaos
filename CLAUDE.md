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

### Multi-Platform Build

VyomaOS supports 6 platform profiles selectable via `PLATFORM=<name>`:

| Platform name | Target | Runtime | RAM floor |
|---------------|--------|---------|-----------|
| `mcu-minimal` | ARM Cortex-M4 MCU | wasm3 interpreter | 128 KB |
| `iot-edge` | ARM64 SBC (Raspberry Pi) | WAMR AOT | 4 MB |
| `robotics-rt` | ARM64 robot controller | WAMR AOT | 8 MB |
| `mobile` | ARM64 tablet/phone | Wasmtime JIT | 256 MB |
| `desktop-full` | x86-64 workstation (default) | Wasmtime JIT | 512 MB |
| `server-headless` | ARM64 / x86-64 server | Wasmtime JIT | 1 GB |

```bash
make build PLATFORM=desktop-full     # default, equivalent to plain `make build`
make build PLATFORM=iot-edge         # IoT/embedded ARM64 image
make build PLATFORM=server-headless  # headless server ARM64 image
make build PLATFORM=robotics-rt      # robotics ARM64 image
make build PLATFORM=mobile           # mobile ARM64 image with touchscreen
make build PLATFORM=mcu-minimal      # ARM Cortex-M MCU image (wasm3)
```

Per-platform QEMU invocations:
- `desktop-full` / default: uses `qemu-system-x86_64` with `-kernel out/bzImage` (existing `make run`, `make run-gui`)
- `iot-edge`, `robotics-rt`, `mobile`, `server-headless`: use `qemu-system-aarch64 -machine virt -cpu cortex-a72`
- `mcu-minimal`: uses `qemu-system-arm -machine mps2-an385 -cpu cortex-m3`

```bash
make run PLATFORM=iot-edge           # headless ARM64 QEMU boot
make run PLATFORM=server-headless    # headless server ARM64
```

### New Supervisor Subsystem Modules (spec-043)

The supervisor is organized into focused subsystems under `supervisor/src/`:

| Directory | Purpose |
|-----------|---------|
| `runtime/` | `WasmRuntime` trait + Wasmtime and wasm3 adapters |
| `hal/` | Hardware Abstraction Layer — GPIO, I2C, SPI, UART, ADC trait definitions |
| `profile/` | Platform profile loader — parses TOML files from `profile/profiles/` |
| `ota/` | A/B slot OTA update manager with health-check and rollback |
| `observability/` | Structured heartbeat emitter (JSON-line format) |
| `capability/` | Peripheral capability enforcer — `PeripheralRegistry` exclusive-access tracking |

Platform profile TOML files live in `supervisor/src/profile/profiles/` (one per platform).

### New Peripheral Capability Fields

Beyond `stdio`, `filesystem`, `network`, `display`, `shell`, `mouse`, apps on embedded/robotics platforms may also declare hardware peripheral capabilities in `vyoma.toml`:

| Field | Type | Effect |
|-------|------|--------|
| `touch` | `bool` | App receives `VYOMA_INPUT:touch:` events (mobile profile) |
| `gpio_pins` | `[u8]` | App gets exclusive access to the listed GPIO pin numbers |
| `i2c_bus` | `u8` | App gets access to the specified I2C bus number |
| `spi_bus` | `u8` | App gets access to the specified SPI bus number |
| `uart_port` | `u8` | App gets access to the specified UART port number |
| `adc_channel` | `u8` | App gets access to the specified ADC channel number |

Example with peripheral capabilities:
```toml
[capabilities]
stdio = true

[capabilities.gpio]
pins = [4, 17]
direction = "output"

[capabilities.i2c]
bus = 1
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

Full specification: [`docs/vyoma-draw-protocol.md`](docs/vyoma-draw-protocol.md)

Apps write line-oriented commands to stdout. Colors are packed `u32`: `(R<<24)|(G<<16)|(B<<8)|A` printed as decimal.

```
VYOMA_DRAW:fill_rect:<x>,<y>,<w>,<h>,<rgba>
VYOMA_DRAW:draw_text:<x>,<y>,<rgba>,<size>,<text>   # size: s=4×8  m=8×16  l=16×32
VYOMA_DRAW:draw_text_wrap:<x>,<y>,<max_w>,<rgba>,<size>,<text>
VYOMA_DRAW:flush
```

Example (Rust):
```rust
const WHITE: u32 = 0xFFFFFFFF;
const BG:    u32 = 0x1E1E2EFF;
println!("VYOMA_DRAW:fill_rect:0,20,960,700,{BG}");    // clear background
println!("VYOMA_DRAW:draw_text:8,24,{WHITE},m,Hello");  // medium font text
println!("VYOMA_DRAW:flush");                            // commit frame
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

## Code Size Rule

**No source file may exceed 500 lines.** When a file approaches or exceeds this limit, split it into focused submodules before adding more code. For the supervisor, this means separate files per subsystem (chrome, ipc_handlers, draw_cmd, mouse_input, etc.). For WASM apps, extract helper modules under `src/` when `main.rs` grows beyond 500 lines.

This rule applies to all `.rs` files across the repo (supervisor and apps). The `.tessl/` plugin cache is excluded.

## Key Design Decisions

**Capability-secure by default**: Supervisor does not filter app syscalls; it only wires up the WASI imports declared in vyoma.toml. This means:
- If an app doesn't declare `network = true`, there is no network interface to syscall
- If an app doesn't declare `filesystem = true`, there is no `/data` mount to access
- No separate filtering layer (seccomp, AppArmor, SELinux) needed

**Deterministic binary output**: WASM bytecode is byte-identical across builds and hosts (unlike ELF binaries which vary by libc/architecture). This enables reproducible deployments.

**Minimal kernel**: Linux kernel compiled with allnoconfig + only drivers VyomaOS actually uses (virtio, 9P, DRM, fbcon). No networking stack, no filesystem drivers beyond 9P, no USB. Supervisor handles all high-level policy.

**Supervisor-side IPC**: Apps don't directly communicate; supervisor brokers all messages. This centralizes routing logic and enables future debugging/monitoring features.

## Testing Strategy

### Unit tests (supervisor)

```bash
make unit-test            # cargo test inside Docker, RUSTFLAGS=-D warnings
```

Tests live in `supervisor/tests/` and cover manifest parsing/validation, IPC routing, process lifecycle, structured logging, display commands, font rendering, input handling, mouse events, watchdog, and window management. All tests run against the `x86_64-unknown-linux-musl` target (same as production binary).

### Manifest validation

```bash
make check-manifests      # validates all apps/*/vyoma.toml against the schema
```

Prints `OK` or `ERROR <kind>` per manifest; exits non-zero if any error found. Runs automatically before `make apps`.

### Smoke test (headless QEMU boot)

```bash
make smoke                # headless QEMU boot, 30 s timeout
```

Boots `out/bzImage` + `out/initramfs.cpio.gz`, waits for `[lifecycle] all apps spawned` on serial output, then exits. Requires `make build` first. Pass/fail printed as `SMOKE: PASS` or `SMOKE: FAIL: <reason>`.

### Full CI test suite

```bash
make test                 # make build + make unit-test + make smoke
```

### Incremental builds

Per-app stamp files (`out/.apps/<name>.stamp`) ensure only the app whose source changed recompiles:

```bash
touch apps/hello-world/src/main.rs
make apps                 # only hello-world recompiles
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


<!-- SPECKIT START -->
For additional context about technologies to be used, project structure,
shell commands, and other important information, read the current plan at
`specs/046-apple-ui-fidelity/plan.md`.
<!-- SPECKIT END -->
