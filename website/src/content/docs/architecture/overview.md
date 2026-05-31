---
title: Architecture Overview
description: Understanding the VyomaOS system stack and design decisions.
order: 1
---

# Architecture Overview

VyomaOS is a four-layer operating system where the kernel handles hardware, and everything else runs through a Rust supervisor that manages WebAssembly applications.

## System stack

```
Linux 5.10 kernel (allnoconfig, 2.3 MB)
  |
Rust supervisor (PID 1, ~2.9 MB static musl)
  |-- Manifest parser (TOML capabilities)
  |-- Concurrent scheduler (one thread per app)
  |-- IPC broker (route @<app>: messages)
  |-- Framebuffer driver (DRM/virtio-gpu)
  |-- TTY input router (raw mode)
  |-- Process manager (ps, kill, restart)
  |
Wasmtime runtime (WASI Preview 2)
  |
WASM apps (wasm32-wasip2 binaries, 1-10 KB each)
```

## Design decisions

### Capability-secure by default

The supervisor does not filter app syscalls. It only wires up the WASI imports declared in each app's `vyoma.toml`. This means:

- No `network = true` declaration = no network interface exists for the app
- No `filesystem = true` declaration = no `/data` mount exists
- No seccomp, AppArmor, or SELinux layer needed

### Deterministic binaries

WASM bytecode is byte-identical across builds and hosts. Unlike ELF binaries that vary by libc and architecture, WASM apps produce the same output everywhere. This enables reproducible deployments.

### Minimal kernel

The Linux kernel is compiled with `allnoconfig` plus only the drivers VyomaOS needs:

- virtio (block, network, GPU, console)
- 9P filesystem (host-VM file sharing)
- DRM (framebuffer for display)
- fbcon (early console output)

No networking stack, no USB drivers, no excess filesystem drivers.

### Supervisor-side IPC

Apps never communicate directly. The supervisor brokers all messages, which:

- Centralizes routing logic
- Enables monitoring and debugging
- Allows message filtering and rate limiting
- Supports future features like message logging

## Supervisor subsystems

The supervisor is organized into focused modules:

| Subsystem | Purpose |
|-----------|---------|
| `runtime/` | WasmRuntime trait + Wasmtime/wasm3 adapters |
| `hal/` | Hardware Abstraction Layer (GPIO, I2C, SPI, UART, ADC) |
| `profile/` | Platform profile loader (desktop, mobile, IoT, MCU) |
| `ota/` | A/B slot OTA update manager |
| `observability/` | Structured heartbeat and metrics emitter |
| `capability/` | Peripheral capability enforcer |
| `display/` | Framebuffer driver and VYOMA_DRAW parser |
| `font/` | Scalable font rendering via fontdue |
| `image/` | PNG image loading via lodepng |
| `chrome/` | Window decorations and compositor |

## Build system

Docker-based hermetic builds ensure reproducibility:

```bash
make build    # kernel + supervisor + apps + rootfs
```

The Makefile orchestrates:
1. Linux kernel compilation
2. Supervisor compilation (`x86_64-unknown-linux-musl`)
3. WASM app compilation (`wasm32-wasip2`)
4. Rootfs packaging (initramfs.cpio.gz)
5. Data disk creation (ext4, 64 MB)

## Multi-platform support

Six platform profiles target different hardware:

| Platform | Target | Runtime | RAM |
|----------|--------|---------|-----|
| `desktop-full` | x86-64 | Wasmtime JIT | 512 MB |
| `mobile` | ARM64 | Wasmtime JIT | 256 MB |
| `server-headless` | ARM64/x86-64 | Wasmtime JIT | 1 GB |
| `iot-edge` | ARM64 SBC | WAMR AOT | 4 MB |
| `robotics-rt` | ARM64 | WAMR AOT | 8 MB |
| `mcu-minimal` | ARM Cortex-M4 | wasm3 | 128 KB |
