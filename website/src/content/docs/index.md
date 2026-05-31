---
title: Getting Started
description: Welcome to VyomaOS -- the WASM-first operating system.
order: 0
---

# Getting Started with VyomaOS

VyomaOS is a **WASM-first operating system** where every application is a `wasm32-wasip2` binary executed by Wasmtime under a Rust PID 1 supervisor. The Linux kernel handles hardware only. Capabilities (filesystem, network, display, stdio) are declared per-app in a manifest and enforced at runtime.

## What makes VyomaOS different?

- **Capability-secure by default**: Apps only get the WASI imports they declare. No syscall filtering needed.
- **Deterministic binaries**: WASM bytecode is byte-identical across builds and hosts.
- **Minimal kernel**: Linux compiled with allnoconfig + only the drivers VyomaOS uses.
- **Multi-platform**: One codebase targets desktop, mobile, IoT, MCU, robotics, and server.

## Quick links

- [Introduction](/docs/guides/introduction) -- Learn the core concepts
- [Quick Start](/docs/guides/quick-start) -- Build and boot VyomaOS
- [Architecture Overview](/docs/architecture/overview) -- Understand the system stack
- [Manifest Reference](/docs/reference/manifest) -- App manifest format
- [Display Protocol](/docs/reference/display-protocol) -- VYOMA_DRAW protocol spec

## System requirements

- Docker (for hermetic builds)
- QEMU (for running the OS)
- ~2 GB disk space for build artifacts

## Building VyomaOS

```bash
git clone https://github.com/hbarve1/vyomaos.git
cd vyomaos
make build    # Full build: kernel + supervisor + apps + rootfs
make run      # Boot in QEMU (headless)
make run-gui  # Boot with display
```
