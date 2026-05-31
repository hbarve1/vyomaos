---
title: Quick Start
description: Build and boot VyomaOS in 5 minutes.
order: 2
---

# Quick Start

This guide walks you through building VyomaOS from source and booting it in QEMU.

## Prerequisites

- **Docker**: All builds run inside a hermetic container
- **QEMU**: `qemu-system-x86_64` for desktop profile
- **Git**: To clone the repository

## 1. Clone the repository

```bash
git clone https://github.com/hbarve1/vyomaos.git
cd vyomaos
```

## 2. Build the Docker image

```bash
make image
```

This creates the `vyomaos-builder` Docker image with all build dependencies: kernel build tools, musl-tools, Rust (stable), and the `wasm32-wasip2` target.

## 3. Full build

```bash
make build
```

This runs the full pipeline inside Docker:

1. **Kernel**: Compiles Linux 5.10 with allnoconfig + VyomaOS drivers (2.3 MB)
2. **Supervisor**: Compiles the Rust supervisor for `x86_64-unknown-linux-musl` (~2.9 MB)
3. **Apps**: Compiles all WASM apps for `wasm32-wasip2`
4. **Rootfs**: Packages everything into `initramfs.cpio.gz` (~34 MB)

## 4. Boot it

```bash
# Headless (serial console)
make run

# With display (Linux)
make run-gui DISPLAY_BACKEND=sdl

# With display (macOS)
make run-gui DISPLAY_BACKEND=cocoa

# With networking
make run-net
```

## 5. Interact with VyomaOS

At the supervisor console, try:

```
ps                    # List running apps
log hello-world       # View app logs
@supervisor: list     # List apps via IPC
```

## Creating your first app

```bash
mkdir apps/hello
```

Create `apps/hello/Cargo.toml`:

```toml
[package]
name = "hello"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "hello"
path = "src/main.rs"
```

Create `apps/hello/src/main.rs`:

```rust
fn main() {
    println!("Hello from VyomaOS!");
}
```

Create `apps/hello/vyoma.toml`:

```toml
[app]
name = "hello"
version = "0.1.0"
wasm = "hello.wasm"

[capabilities]
stdio = true
```

Build and run:

```bash
cd apps/hello
cargo build --target wasm32-wasip2 --release
cd ../..
make rootfs && make run
```

## Multi-platform builds

```bash
make build PLATFORM=iot-edge         # ARM64 IoT
make build PLATFORM=mobile           # ARM64 mobile
make build PLATFORM=server-headless  # Headless server
make build PLATFORM=mcu-minimal      # ARM Cortex-M MCU
```
