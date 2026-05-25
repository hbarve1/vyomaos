# Quickstart: VyomaOS Universal Modular OS

## Prerequisites

- Docker (for hermetic builds)
- QEMU (for desktop/server testing)
- Rust toolchain with `wasm32-wasip2` target
- ARM/RISC-V toolchain (for MCU builds — installed via Docker)

## Build for a Platform

```bash
# MCU (ARM Cortex-M4)
make build PLATFORM=mcu-minimal

# IoT (Raspberry Pi / ARM64 SBC)
make build PLATFORM=iot-edge

# Desktop (current default)
make build PLATFORM=desktop-full

# Server (headless)
make build PLATFORM=server-headless
```

## Create a Cross-Platform WASM App

```bash
mkdir apps/my-sensor
cd apps/my-sensor
```

**Cargo.toml**:
```toml
[package]
name = "my-sensor"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "my-sensor"
path = "src/main.rs"
```

**vyoma.toml** (with GPIO capability):
```toml
[app]
name = "my-sensor"
version = "0.1.0"
wasm = "my-sensor.wasm"

[capabilities]
stdio = true

[capabilities.gpio]
pins = [4]
direction = "input"

[capabilities.i2c]
bus = 1
address = 0x48
```

**src/main.rs**:
```rust
fn main() {
    loop {
        // Read temperature from I2C sensor
        // (HAL host functions are auto-wired by supervisor based on capabilities)
        let temp = vyoma_hal::i2c_read(1, 0x48, 2);
        eprintln!("[my-sensor] temperature: {}°C", temp);

        // Emit heartbeat
        std::thread::sleep(std::time::Duration::from_secs(5));
    }
}
```

Build:
```bash
cargo build --target wasm32-wasip2 --release
```

## Run on Different Platforms

The **same** `my-sensor.wasm` binary runs on:

```bash
# MCU (ARM Cortex-M4, wasm3 interpreter)
make run PLATFORM=mcu-minimal

# IoT gateway (ARM64, WAMR AOT)
make run PLATFORM=iot-edge

# Desktop (x86-64, Wasmtime JIT) — for development/testing
make run PLATFORM=desktop-full
```

## OTA Update a Running Module

```bash
# Build new version
cd apps/my-sensor
# ... make changes ...
cargo build --target wasm32-wasip2 --release

# Push to device (A/B slot update)
vyoma-ota push my-sensor target-device-ip
# → Downloads to slot B
# → Supervisor swaps, runs health check for 60s
# → If healthy: slot B becomes primary
# → If unhealthy: automatic rollback to slot A
```

## Monitor Fleet

```bash
# View heartbeats from all modules on a device
vyoma-monitor --target device-ip

# Output:
# {"type":"heartbeat","module":"my-sensor","uptime_s":3600,"mem_kb":12,"status":"healthy","ts":"..."}
# {"type":"heartbeat","module":"heartbeat","uptime_s":3600,"mem_kb":4,"status":"healthy","ts":"..."}
```

## Test Across Runtimes

```bash
# Verify same binary produces same output on wasm3 and Wasmtime
make test-runtime-parity APP=my-sensor
# → Runs on wasm3 interpreter, captures output
# → Runs on Wasmtime JIT, captures output
# → Compares outputs byte-for-byte
```
