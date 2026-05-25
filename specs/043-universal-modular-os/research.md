# Research: VyomaOS Universal Modular OS

**Date**: 2026-05-25 | **Spec**: [spec.md](spec.md)

## R1: Lightweight WASM Runtime for MCU Platforms

**Decision**: Use **wasm3** as the primary MCU/IoT runtime, with WAMR as a secondary option for platforms needing AOT compilation.

**Rationale**:
- wasm3 is the fastest WASM interpreter by cold-start benchmarks (Jan 2026 data)
- Runs on as little as 64 KB RAM — fits the MCU target (≤ 256 KB)
- Pure C, no JIT — works on Harvard-architecture MCUs without executable memory
- WAMR offers AOT mode for better steady-state performance on richer IoT devices but requires a compilation step

**Alternatives considered**:
- **Wasmtime**: Requires ~4 MB RAM, JIT/AOT only — too heavy for MCUs
- **Wasmer**: Similar RAM requirements to Wasmtime; WASIX extensions are non-standard
- **wasm3 only**: Acceptable for MCU/IoT; interpreter overhead (~3-5x native) is fine for control logic, not for compute-heavy workloads
- **WAMR only**: Good middle ground but lacks wasm3's minimal footprint for the most constrained devices

## R2: Hardware Abstraction Layer (HAL) Design Pattern

**Decision**: Define HAL as a set of **Rust traits** in the supervisor that are implemented per-platform, exposing hardware interfaces as **typed WASI host functions** consumable by WASM modules.

**Rationale**:
- Rust traits provide compile-time guarantees that every platform implements the required interfaces
- WASI host functions are the standard WASM mechanism for host-provided capabilities
- Per-peripheral capability checks happen in the supervisor before dispatching to the HAL implementation
- This pattern mirrors embedded Rust ecosystem conventions (embedded-hal trait crate)

**Alternatives considered**:
- **Device tree / Kconfig**: Zephyr's approach — powerful but adds configuration complexity unsuitable for the explicit-over-implicit constitution principle
- **Direct memory-mapped I/O**: Unsafe, bypasses WASM sandbox, violates capability model
- **WASI proposals for hardware**: No upstream WASI proposal covers GPIO/I2C/SPI; VyomaOS must define its own host functions

## R3: Platform Profile System

**Decision**: Platform profiles are **TOML files** that declare which supervisor modules, HAL drivers, WASM runtime, and default apps are included in a build. Selected at build time via `PLATFORM=<name>` in the Makefile.

**Rationale**:
- TOML is already used for vyoma.toml manifests — consistent tooling
- Build-time selection keeps the supervisor binary minimal per platform (Principle III)
- Profiles are explicit, auditable configuration files (Principle V)
- Each profile produces a distinct initramfs/firmware image

**Example profile** (`profiles/mcu-minimal.toml`):
```toml
[platform]
name = "mcu-minimal"
arch = "arm-cortex-m4"
runtime = "wasm3"
min_ram_kb = 128

[supervisor]
modules = ["lifecycle", "capability", "ipc", "hal", "observability"]
exclude = ["display", "chrome", "mouse_input", "toast", "packages"]

[hal]
drivers = ["gpio", "i2c", "uart"]

[boot]
apps = ["sensor-reader", "heartbeat"]
```

**Alternatives considered**:
- **Cargo features**: Compile-time feature flags — less readable, harder to audit, mixes configuration with code
- **Runtime configuration**: Loading modules dynamically — increases supervisor size and complexity
- **Separate supervisor binaries**: One per platform — too much code duplication

## R4: A/B Slot OTA Update Mechanism

**Decision**: Implement A/B slot OTA at the **WASM module level** (not full firmware), with supervisor-managed health checks and automatic rollback.

**Rationale**:
- Module-level OTA means only changed apps are transferred — bandwidth efficient for IoT
- A/B slots ensure the device always has a known-good fallback
- Health check: supervisor monitors the new module for 60 seconds after launch; if it crashes, fails watchdog, or reports unhealthy, the supervisor reverts to the previous slot
- On MCU platforms, A/B slots map to two flash regions for the WASM binary

**Update flow**:
1. New `.wasm` binary is downloaded to slot B (while slot A is running)
2. Supervisor verifies hash/signature of new binary
3. Supervisor swaps to slot B, launches new version
4. Health check runs for 60 seconds (configurable via profile)
5. If healthy → slot B becomes primary; slot A is marked for reuse
6. If unhealthy → supervisor reverts to slot A, marks slot B as failed, reports error

**Alternatives considered**:
- **In-place update**: No rollback — unacceptable for unattended field devices
- **Full firmware A/B**: Industry standard (Android, Mender) but requires 2x flash — VyomaOS needs it only per-module
- **Differential updates**: Saves bandwidth but adds patch complexity; can be layered on top of A/B in a future phase

## R5: Observability Architecture

**Decision**: Two-tier observability — **structured log + heartbeat** (baseline, all platforms) and **OpenTelemetry** (optional, resource-rich platforms).

**Rationale**:
- MCU/IoT devices cannot afford a full telemetry SDK; structured log lines + periodic heartbeats are sufficient for field debugging
- Desktop/server platforms have RAM/CPU budget for OpenTelemetry traces, metrics, and log export
- Platform profile declares which observability tier is active
- Heartbeat format: JSON-line emitted every 30s (configurable) with module name, uptime, memory usage, last error

**Baseline heartbeat format**:
```json
{"type":"heartbeat","module":"sensor-reader","uptime_s":3600,"mem_kb":12,"status":"healthy","ts":"2026-05-25T09:00:00Z"}
```

**Alternatives considered**:
- **No observability**: Unacceptable — violates Constitution Principle VI
- **Full OpenTelemetry everywhere**: Too heavy for MCU (otel-rust SDK is ~2 MB)
- **Custom binary protocol**: More efficient but adds parsing burden on the monitoring side; JSON-lines are universally parseable

## R6: Per-Peripheral Capability Enforcement

**Decision**: Extend vyoma.toml with **typed peripheral declarations** that the supervisor validates at module spawn time, granting only the declared hardware interfaces via HAL host functions.

**Rationale**:
- Consistent with existing capability model (deny-by-default)
- Supervisor checks manifest before wiring up HAL host functions — undeclared peripherals have no callable interface
- Pin-level granularity prevents one module from interfering with another's hardware

**Manifest extension**:
```toml
[capabilities]
stdio = true

[capabilities.gpio]
pins = [2, 4, 17]
direction = "output"  # "input", "output", "bidirectional"

[capabilities.i2c]
bus = 1
address = 0x48

[capabilities.uart]
port = 0
baud = 115200
```

**Alternatives considered**:
- **Flat `hardware = true`**: Too coarse — one module gets all peripherals
- **Linux device permissions**: Requires Linux userland; VyomaOS avoids this
- **SELinux/AppArmor policies**: Too complex; VyomaOS capability model is simpler and stronger
