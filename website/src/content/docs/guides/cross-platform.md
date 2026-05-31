---
title: "One App, Six Platforms"
description: How a single VyomaOS WASM binary runs on desktop, mobile, IoT, robotics, server, and MCU platforms without recompilation.
order: 4
---

# One App, Six Platforms

A core design goal of VyomaOS is **write once, run anywhere** -- not through emulation layers or compatibility shims, but because WebAssembly is a genuinely portable compilation target. The same `wasm32-wasip2` binary that runs on your x86-64 development workstation runs identically on an ARM64 Raspberry Pi, an ARM Cortex-M4 microcontroller, and everything in between.

This guide explains how VyomaOS achieves this through platform profiles, runtime adapters, and capability-aware manifests.

## The six platforms

| Platform | Target hardware | WASM runtime | RAM floor | Use case |
|----------|----------------|--------------|-----------|----------|
| `desktop-full` | x86-64 workstation | Wasmtime JIT | 512 MB | Development, desktop OS |
| `mobile` | ARM64 tablet/phone | Wasmtime JIT | 256 MB | Touch-driven mobile UI |
| `iot-edge` | ARM64 SBC (Raspberry Pi) | WAMR AOT | 4 MB | Sensor hubs, edge gateways |
| `robotics-rt` | ARM64 robot controller | WAMR AOT | 8 MB | Motor control, sensor fusion |
| `server-headless` | ARM64 / x86-64 server | Wasmtime JIT | 1 GB | Headless services, APIs |
| `mcu-minimal` | ARM Cortex-M4 MCU | wasm3 interpreter | 128 KB | Bare-metal embedded |

Each platform uses a different WASM runtime optimized for its constraints, but all execute the same `wasm32-wasip2` bytecode.

## How it works

### 1. You build once

```bash
cd apps/my-app
cargo build --target wasm32-wasip2 --release
```

This produces a single `.wasm` file. It contains no platform-specific code, no architecture-dependent instructions, and no linked system libraries. The binary is byte-identical regardless of which host machine compiles it.

### 2. The platform profile selects the runtime

When you build a VyomaOS image for a specific platform, the build system selects the appropriate WASM runtime:

```bash
make build PLATFORM=desktop-full     # Bundles Wasmtime JIT
make build PLATFORM=iot-edge         # Bundles WAMR AOT compiler
make build PLATFORM=mcu-minimal      # Bundles wasm3 interpreter
```

### 3. The supervisor adapts at startup

The supervisor reads the platform profile TOML at boot and configures itself accordingly. It uses the `WasmRuntime` trait to abstract over Wasmtime, WAMR, and wasm3:

```
Supervisor starts
  |-- Reads platform profile (e.g., iot-edge.toml)
  |-- Selects runtime adapter (WAMR for this profile)
  |-- Loads app manifests (vyoma.toml per app)
  |-- Wires up capabilities based on manifest + profile
  |-- Spawns apps through the selected runtime
```

## Platform profiles

Each platform is defined by a TOML file in `supervisor/src/profile/profiles/`. Here is what differentiates them:

### desktop-full.toml

```toml
[profile]
name = "desktop-full"
arch = "x86_64"
runtime = "wasmtime"

[resources]
ram_mb = 512
max_apps = 100
display = true
network = true
filesystem = true

[runtime]
jit = true
optimization_level = "speed"
cache_compiled = true
```

The desktop profile enables all capabilities, uses JIT compilation for maximum performance, and supports up to 100 concurrent apps.

### mobile.toml

```toml
[profile]
name = "mobile"
arch = "aarch64"
runtime = "wasmtime"

[resources]
ram_mb = 256
max_apps = 50
display = true
network = true
filesystem = true
touch = true

[runtime]
jit = true
optimization_level = "balanced"
cache_compiled = true
```

The mobile profile adds touch input support and reduces the app limit. The runtime optimization level is `balanced` to save battery.

### iot-edge.toml

```toml
[profile]
name = "iot-edge"
arch = "aarch64"
runtime = "wamr"

[resources]
ram_mb = 4
max_apps = 10
display = false
network = true
filesystem = true

[peripherals]
gpio = true
i2c = true
spi = true

[runtime]
aot = true
optimization_level = "size"
```

The IoT profile disables the display subsystem entirely, enables peripheral hardware access (GPIO, I2C, SPI), and uses ahead-of-time compilation through WAMR for lower memory overhead.

### robotics-rt.toml

```toml
[profile]
name = "robotics-rt"
arch = "aarch64"
runtime = "wamr"

[resources]
ram_mb = 8
max_apps = 15
display = false
network = true
filesystem = true

[peripherals]
gpio = true
i2c = true
spi = true
uart = true
adc = true

[runtime]
aot = true
optimization_level = "speed"
watchdog_default_secs = 5
```

The robotics profile enables all peripheral types and sets a default watchdog timeout. If any app goes silent for 5 seconds, the supervisor kills and restarts it -- critical for real-time control systems.

### server-headless.toml

```toml
[profile]
name = "server-headless"
arch = "aarch64"
runtime = "wasmtime"

[resources]
ram_mb = 1024
max_apps = 200
display = false
network = true
filesystem = true

[runtime]
jit = true
optimization_level = "speed"
cache_compiled = true
```

The server profile maximizes throughput: 200 concurrent apps, JIT at full speed, no display overhead.

### mcu-minimal.toml

```toml
[profile]
name = "mcu-minimal"
arch = "arm-cortex-m4"
runtime = "wasm3"

[resources]
ram_kb = 128
max_apps = 3
display = false
network = false
filesystem = false

[peripherals]
gpio = true
uart = true
adc = true

[runtime]
interpreter = true
stack_size_kb = 4
```

The MCU profile is radically constrained: 128 KB of RAM, 3 apps maximum, no display, no network, no filesystem. It uses the wasm3 interpreter because JIT compilation requires more memory than the entire system has. GPIO, UART, and ADC provide the hardware interface.

## Writing platform-aware apps

Your app code does not need to know which platform it runs on. The manifest declares capabilities, and the supervisor provides them (or does not) based on the platform profile.

However, you can write apps that adapt to available capabilities:

```rust
fn main() {
    // Try to draw -- this works on desktop and mobile
    println!("VYOMA_DRAW:fill_rect:0,0,400,300,{BG}");
    println!("VYOMA_DRAW:draw_text:10,10,{WHITE},m,Hello");
    println!("VYOMA_DRAW:flush");

    // On IoT/MCU, the display commands are silently ignored
    // because display = false in the profile.
    // The app still runs -- it just has no visible output.

    // stdio always works (if declared), so log to stdout:
    eprintln!("[my-app] started successfully");
}
```

For apps that need to behave differently on different platforms, use environment variables or the IPC system to query the supervisor:

```rust
println!("@supervisor: platform");
// Supervisor responds with the platform name on stdin
```

## Building for a specific platform

```bash
# Build the full OS image for each platform
make build PLATFORM=desktop-full
make build PLATFORM=mobile
make build PLATFORM=iot-edge
make build PLATFORM=robotics-rt
make build PLATFORM=server-headless
make build PLATFORM=mcu-minimal
```

### Running in QEMU

Each platform uses a different QEMU configuration:

```bash
# Desktop: x86-64 with virtio-gpu display
make run-gui PLATFORM=desktop-full

# Mobile: ARM64 with touch input emulation
make run PLATFORM=mobile

# IoT: ARM64 headless (serial console)
make run PLATFORM=iot-edge

# Robotics: ARM64 headless
make run PLATFORM=robotics-rt

# Server: ARM64 or x86-64 headless
make run PLATFORM=server-headless

# MCU: ARM Cortex-M3 emulation (qemu-system-arm)
make run PLATFORM=mcu-minimal
```

The QEMU targets per platform:

| Platform | QEMU command | Machine |
|----------|-------------|---------|
| `desktop-full` | `qemu-system-x86_64` | Default with `-kernel` |
| `mobile` | `qemu-system-aarch64` | `virt` with `-cpu cortex-a72` |
| `iot-edge` | `qemu-system-aarch64` | `virt` with `-cpu cortex-a72` |
| `robotics-rt` | `qemu-system-aarch64` | `virt` with `-cpu cortex-a72` |
| `server-headless` | `qemu-system-aarch64` | `virt` with `-cpu cortex-a72` |
| `mcu-minimal` | `qemu-system-arm` | `mps2-an385` with `-cpu cortex-m3` |

## Peripheral capabilities by platform

Not all capabilities are available on all platforms. The supervisor enforces this at the profile level:

| Capability | Desktop | Mobile | IoT | Robotics | Server | MCU |
|-----------|---------|--------|-----|----------|--------|-----|
| `stdio` | yes | yes | yes | yes | yes | yes |
| `display` | yes | yes | no | no | no | no |
| `network` | yes | yes | yes | yes | yes | no |
| `filesystem` | yes | yes | yes | yes | yes | no |
| `mouse` | yes | no | no | no | no | no |
| `touch` | no | yes | no | no | no | no |
| `gpio` | no | no | yes | yes | no | yes |
| `i2c` | no | no | yes | yes | no | no |
| `spi` | no | no | yes | yes | no | no |
| `uart` | no | no | no | yes | no | yes |
| `adc` | no | no | no | yes | no | yes |

If an app declares a capability that the platform does not support, the supervisor logs a warning and the capability is silently unavailable. The app still starts -- it just cannot use that feature.

## Why this matters

Traditional operating systems require recompilation for each target architecture. A Linux binary compiled for x86-64 cannot run on ARM64 without an emulation layer. Docker images are architecture-specific. Even Java's "write once, run anywhere" requires a JVM per platform and does not extend to bare-metal microcontrollers.

VyomaOS apps are different:

- **No recompilation**: The `.wasm` binary is the deployable artifact for all platforms
- **No emulation**: Each platform runs a native WASM runtime optimized for its hardware
- **No compatibility layers**: WASI Preview 2 provides a standardized system interface
- **Byte-identical binaries**: SHA-256 verification ensures the same binary runs everywhere
- **128 KB to 1 GB**: The same app model scales from microcontrollers to servers

This is not a theoretical capability. VyomaOS ships 207+ apps, and every one of them runs on all six platforms (subject to capability availability).
