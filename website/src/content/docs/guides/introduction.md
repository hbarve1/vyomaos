---
title: Introduction
description: Core concepts and architecture of VyomaOS.
order: 1
---

# Introduction to VyomaOS

VyomaOS is built on a simple principle: **every application is a WebAssembly binary, and security comes from capability declaration, not filtering**.

## Core architecture

The system has four layers:

1. **Linux Kernel** (2.3 MB) -- Handles hardware only. Compiled with `allnoconfig` plus virtio, 9P, DRM, and fbcon drivers.
2. **Rust Supervisor** (PID 1, ~2.9 MB) -- Static musl-linked binary that manages everything above the kernel.
3. **Wasmtime Runtime** -- Executes `wasm32-wasip2` binaries with WASI Preview 2.
4. **WASM Apps** -- User applications, typically 1-10 KB each.

## The capability model

Every app has a `vyoma.toml` manifest that declares its capabilities:

```toml
[app]
name = "my-app"
version = "0.1.0"
wasm = "my-app.wasm"

[capabilities]
stdio = true
filesystem = true
network = false
display = false
```

The supervisor only wires up the WASI imports that are declared. If an app does not declare `network = true`, there is no network interface available -- no amount of syscalls can reach the network.

## IPC system

Apps communicate through the supervisor's IPC broker:

```rust
// Send a message to another app
println!("@pong: hello from ping");

// The supervisor routes it and strips the prefix
// before delivering to the target app's stdin
```

## Display protocol

Apps with `display = true` can draw to the framebuffer using the VYOMA_DRAW protocol:

```rust
println!("VYOMA_DRAW:fill_rect:0,0,960,700,{BG}");
println!("VYOMA_DRAW:draw_text:8,24,{WHITE},m,Hello");
println!("VYOMA_DRAW:flush");
```

## What is in the supervisor?

The supervisor contains 62+ subsystem modules:

- Manifest parser and capability enforcer
- Concurrent scheduler (one thread per app)
- IPC broker with message routing
- Framebuffer driver and window compositor
- TTY input router (raw mode, per-keypress dispatch)
- Process manager (ps, kill, restart, reload)
- HAL for GPIO, I2C, SPI, UART, ADC
- OTA update manager with A/B slots
- Package manager and app store client
