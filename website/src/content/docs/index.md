---
title: VyomaOS Documentation
description: Documentation for VyomaOS -- the WASM-first operating system built on capability-secure WebAssembly.
order: 0
---

# VyomaOS Documentation

VyomaOS is a **WASM-first operating system** built from the ground up on a capability-secure WebAssembly foundation. Every application is a `wasm32-wasip2` binary executed by Wasmtime under a Rust PID 1 supervisor. The Linux kernel handles hardware only -- everything above the kernel is managed by a single Rust binary that enforces security through capability declaration rather than syscall filtering.

Unlike traditional operating systems that bolt on sandboxing after the fact (containers, AppArmor, SELinux), VyomaOS makes isolation the default. Apps declare what they need in a TOML manifest. If an app does not declare `network = true`, no network interface exists for that process. No filtering layer required. This architecture produces deterministic, byte-identical WASM binaries that run on six hardware platforms -- from ARM Cortex-M4 microcontrollers to x86-64 workstations -- without recompilation.

## At a glance

| Metric | Value |
|--------|-------|
| Supervisor modules | 62+ subsystems |
| WASM applications | 207+ apps |
| Test coverage | 425+ tests |
| Cold boot time | < 2 seconds |
| Platform targets | 6 (desktop, mobile, IoT, robotics, server, MCU) |
| Kernel size | 2.3 MB (allnoconfig Linux 5.10) |
| Supervisor binary | ~2.9 MB (static musl) |
| App binary size | 1--10 KB typical |

## Architecture

```
Linux 5.10 kernel (allnoconfig, 2.3 MB)
  |
Rust supervisor (PID 1, ~2.9 MB static musl)
  |-- Manifest parser         -- TOML capability declarations
  |-- Concurrent scheduler    -- one thread per app
  |-- IPC broker              -- route @<app>: messages
  |-- Framebuffer driver      -- DRM/virtio-gpu + VYOMA_DRAW
  |-- Window compositor       -- z-order, decorations, resize
  |-- TTY input router        -- raw mode, per-keypress dispatch
  |-- Process manager         -- ps, kill, restart, reload
  |-- HAL layer               -- GPIO, I2C, SPI, UART, ADC
  |-- OTA update manager      -- A/B slots, rollback
  |-- Security enforcer       -- seccomp, namespaces, audit
  |
Wasmtime / WAMR / wasm3 runtime (per platform)
  |
WASM apps (wasm32-wasip2 binaries)
```

## Documentation sections

### Getting Started

- [Introduction](/docs/guides/introduction) -- Core concepts: capability model, IPC, display protocol
- [Quick Start](/docs/guides/quick-start) -- Build and boot VyomaOS in 5 minutes
- [Building Your First App](/docs/guides/building-first-app) -- Step-by-step tutorial: hello-world with screen drawing
- [Cross-Platform Guide](/docs/guides/cross-platform) -- One app, six platforms: desktop to microcontroller

### Architecture

- [Architecture Overview](/docs/architecture/overview) -- System stack, design decisions, subsystems
- [Security Architecture](/docs/architecture/security) -- Eight security layers: WASM sandbox through audit logging

### Reference

- [Manifest Reference](/docs/reference/manifest) -- Complete `vyoma.toml` format and capability fields
- [Display Protocol](/docs/reference/display-protocol) -- VYOMA_DRAW v2: fill_rect, draw_text, draw_glyph, draw_image

### Comparisons

- How VyomaOS compares to Docker, traditional VMs, and other lightweight OSes

### Deep Dives

- Detailed explorations of specific subsystems and implementation decisions

## Quick start

```bash
git clone https://github.com/hbarve1/vyomaos.git
cd vyomaos
make build    # Full build: kernel + supervisor + apps + rootfs
make run      # Boot in QEMU (headless, serial console)
make run-gui  # Boot with virtio-gpu display
```

### System requirements

- Docker (hermetic builds run inside a container)
- QEMU (boots the OS image)
- ~2 GB disk space for build artifacts

### Multi-platform builds

```bash
make build PLATFORM=desktop-full     # x86-64 workstation (default)
make build PLATFORM=mobile           # ARM64 tablet/phone
make build PLATFORM=iot-edge         # ARM64 SBC (Raspberry Pi)
make build PLATFORM=robotics-rt      # ARM64 robot controller
make build PLATFORM=server-headless  # ARM64/x86-64 server
make build PLATFORM=mcu-minimal      # ARM Cortex-M4 MCU
```

## Key design principles

1. **Capability-secure by default** -- No filtering layers. Undeclared capabilities do not exist.
2. **Deterministic binaries** -- WASM bytecode is byte-identical across builds and hosts.
3. **Minimal kernel** -- Only the drivers VyomaOS uses. No networking stack, no USB, no excess.
4. **Supervisor-side IPC** -- All inter-app communication is brokered and auditable.
5. **Multi-platform from one codebase** -- Same WASM binary runs on six hardware targets.
6. **Small by default** -- Apps are 1--10 KB. The entire OS boots in under 2 seconds.

## Community and source

- **Repository**: [github.com/hbarve1/vyomaos](https://github.com/hbarve1/vyomaos)
- **License**: Open source
- **Issues**: [github.com/hbarve1/vyomaos/issues](https://github.com/hbarve1/vyomaos/issues)
