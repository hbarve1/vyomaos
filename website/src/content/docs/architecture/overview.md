---
title: System Overview
description: How VyomaOS is architected from kernel to apps.
---

VyomaOS is a vertically integrated OS where the Linux kernel handles hardware only, and all policy — security, lifecycle, display, IPC — lives in a single Rust supervisor.

## System Stack

```
Linux 5.10 kernel (allnoconfig, 2.3 MB)
  ↓ hardware abstraction only
Rust supervisor (PID 1, 697 KB static musl)
  ├─ Manifest parser (TOML capabilities)
  ├─ Concurrent scheduler (one thread per app)
  ├─ IPC broker (route @<app>: messages)
  ├─ Framebuffer driver (DRM/virtio-gpu + VYOMA_DRAW protocol)
  ├─ TTY input router (raw mode, per-keypress dispatch)
  └─ Process manager (ps, kill, restart, reload, log)
      ↓ one wasmtime process per app
Wasmtime runtime (WASI Preview 2)
  ↓ only declared capabilities wired up
WASM apps (wasm32-wasip2 binaries, 1–10 KB each)
```

## Key Numbers

| Component | Size |
|-----------|------|
| Linux kernel | 2.3 MB (allnoconfig + virtio + DRM) |
| Supervisor binary | 697 KB (static musl, stripped) |
| Full initramfs | 18 MB (Wasmtime dominates) |
| Average app | 1–10 KB |
| Boot time | < 5 seconds in QEMU |
| Wasmtime version | 43.0.0 |

## Linux Kernel

The kernel is compiled with `allnoconfig` — the absolute minimum configuration — plus only the drivers VyomaOS actually uses:

- **virtio-blk**: Block device for data disk
- **virtio-gpu**: Display output
- **virtio-net**: Networking (optional)
- **9P/virtio**: Filesystem sharing (host ↔ guest)
- **DRM**: Direct Rendering Manager for framebuffer
- **fbcon**: Framebuffer console

No networking stack, no USB, no sound, no filesystem drivers beyond 9P. The kernel is a hardware abstraction layer and nothing more.

## Supervisor (PID 1)

The Rust supervisor is the only native binary in userspace. It:

1. **Parses `/etc/vyoma/boot.toml`** at startup to discover which apps to launch
2. **Reads each app's `vyoma.toml`** to determine declared capabilities
3. **Spawns one Wasmtime process per app** with only the declared WASI imports wired up
4. **Routes IPC messages** between apps via the `@<target>:` protocol
5. **Manages the framebuffer** — parses `VYOMA_DRAW:` commands from app stdout and renders to DRM
6. **Routes keyboard/mouse input** to the focused app
7. **Handles process lifecycle** — restart policies, watchdog, kill, log

## App Model

Every app is a standalone Rust crate targeting `wasm32-wasip2`. Apps:

- Are fully sandboxed inside Wasmtime
- Declare capabilities in `vyoma.toml` (stdio, filesystem, network, display, shell, mouse)
- Communicate only through supervisor-mediated IPC
- Have no access to capabilities they don't declare
- Produce byte-identical binaries across builds

## Display System

Apps draw to the screen by writing `VYOMA_DRAW:` protocol commands to stdout. The supervisor intercepts these, renders to a framebuffer, and manages window chrome (title bars, focus borders, status strips).

Font rendering uses a built-in 8x16 bitmap font with three sizes: small (4x8), medium (8x16), and large (16x32).

## Storage

Host `data/` directory is mounted via 9P virtio at `/data` inside the VM. Apps with `filesystem = true` can read/write files that persist across VM reboots.

## Build System

Docker-based hermetic builds ensure reproducibility:

- All compilation happens inside a Docker container
- Wasmtime and BusyBox binaries are SHA-256 verified
- Per-app stamp files enable incremental builds
- `make test` runs build + unit tests + headless QEMU smoke test
