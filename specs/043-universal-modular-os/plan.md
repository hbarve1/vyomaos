# Implementation Plan: VyomaOS Universal Modular Operating System

**Branch**: `043-universal-modular-os` | **Date**: 2026-05-25 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/043-universal-modular-os/spec.md`

## Summary

Transform VyomaOS from a single-platform desktop OS into a universal, modular operating system that runs on hardware from microcontrollers to supercomputers. The core approach: a tiered WASM runtime (wasm3/WAMR for MCU/IoT, Wasmtime for desktop/server), plug-and-play module architecture with platform profiles, per-peripheral capability declarations, A/B slot OTA updates, and structured observability. Rollout follows a bottom-up strategy: MCU → IoT → Robotics → Mobile → Desktop → Server → HPC.

## Technical Context

**Language/Version**: Rust (stable, 2021 edition) for supervisor and HAL; any language targeting wasm32-wasip2 / wasm64-wasip2 for apps

**Primary Dependencies**:
- Wasmtime 43.0.0 (desktop/server runtime)
- wasm3 or WAMR (MCU/IoT runtime — requires evaluation in Phase 0)
- Linux kernel 5.10 (allnoconfig per platform profile)
- BusyBox 1.35.0 (minimal userland utilities)

**Storage**: 9P virtio persistent storage (`/data`), ext4 data disk; flash storage on MCU platforms

**Testing**: `cargo test` (unit), `make smoke` (integration boot test), `make check-manifests` (manifest validation)

**Target Platform**: MCU (ARM Cortex-M, RISC-V), IoT (ARM/x86 SBCs), Robotics (ARM64), Mobile (ARM64), Desktop (x86-64, ARM64), Server (x86-64, ARM64), HPC (x86-64, ARM64)

**Project Type**: Operating system (kernel + supervisor + runtime + apps)

**Performance Goals**: < 1s boot on embedded with preloaded image, < 5s on desktop/server; < 5ms WASM app cold start; < 10% runtime overhead on MCU interpreter

**Constraints**: Supervisor ≤ 1 MB stripped (Constitution Principle III); MCU target ≤ 256 KB RAM for supervisor + runtime; deterministic byte-identical WASM binaries; no ambient authority (Constitution Principle I)

**Scale/Scope**: 200+ WASM apps currently; target 4+ CPU architectures; 7 platform profiles

## Constitution Check

*GATE: Must pass before Phase 0 research. Re-check after Phase 1 design.*

| Principle | Status | Notes |
|-----------|--------|-------|
| I. Capability-Secure by Default | ✅ PASS | Per-peripheral capabilities in vyoma.toml; deny-by-default enforced across all platforms; ReBAC planned extension |
| II. Test-First Development | ✅ PASS | Unit tests for HAL abstraction, runtime adapter, capability enforcement; smoke tests per platform profile |
| III. Minimal Surface Area | ✅ PASS | Platform profiles select only needed modules; supervisor ≤ 1 MB; tiered runtime avoids bloating MCU targets |
| IV. Hermetic, Reproducible Builds | ✅ PASS | Docker-based builds; WASM bytecode determinism; per-platform build targets in Makefile |
| V. Explicit Over Implicit | ✅ PASS | Platform profiles are explicit TOML configs; no runtime feature detection; capabilities declared not inferred |
| VI. Observability First | ✅ PASS | Structured logs + health heartbeats baseline; OpenTelemetry optional on rich platforms |

All gates pass. Proceeding to Phase 0.

## Project Structure

### Documentation (this feature)

```text
specs/043-universal-modular-os/
├── spec.md              # Feature specification (complete)
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/           # Phase 1 output
│   ├── platform-profile-schema.md
│   ├── hal-interface.md
│   └── runtime-adapter.md
└── tasks.md             # Phase 2 output (/speckit-tasks)
```

### Source Code (repository root)

```text
supervisor/
├── src/
│   ├── main.rs              # Entry point, boot sequence
│   ├── runtime/             # NEW: Runtime adapter layer
│   │   ├── mod.rs           # Runtime trait definition
│   │   ├── wasmtime.rs      # Wasmtime adapter (desktop/server)
│   │   └── wasm3.rs         # wasm3/WAMR adapter (MCU/IoT)
│   ├── hal/                 # NEW: Hardware Abstraction Layer
│   │   ├── mod.rs           # HAL trait definitions
│   │   ├── gpio.rs          # GPIO interface
│   │   ├── i2c.rs           # I2C bus interface
│   │   ├── spi.rs           # SPI interface
│   │   ├── uart.rs          # UART interface
│   │   └── adc.rs           # ADC interface
│   ├── profile/             # NEW: Platform profile system
│   │   ├── mod.rs           # Profile loader
│   │   └── profiles/        # Profile definitions
│   │       ├── mcu-minimal.toml
│   │       ├── iot-edge.toml
│   │       ├── robotics-rt.toml
│   │       ├── desktop-full.toml
│   │       ├── server-headless.toml
│   │       └── mobile.toml
│   ├── ota/                 # NEW: OTA update system
│   │   ├── mod.rs           # OTA coordinator
│   │   ├── ab_slot.rs       # A/B slot manager
│   │   └── health_check.rs  # Post-update health validation
│   ├── observability/       # NEW: Structured logging + telemetry
│   │   ├── mod.rs           # Log aggregator
│   │   ├── heartbeat.rs     # Health heartbeat emitter
│   │   └── otel.rs          # Optional OpenTelemetry adapter
│   ├── capability/          # NEW: Extended capability system
│   │   ├── mod.rs           # Capability enforcer
│   │   └── peripheral.rs    # Per-peripheral capability checks
│   ├── chrome.rs            # (existing) Window chrome
│   ├── display/             # (existing) Display system
│   ├── ipc_handlers.rs      # (existing) IPC routing
│   └── ...                  # (existing modules)
└── tests/
    ├── runtime_adapter.rs   # Runtime abstraction tests
    ├── hal_mock.rs          # HAL mock tests
    ├── platform_profile.rs  # Profile loading tests
    ├── ota_rollback.rs      # A/B slot rollback tests
    └── ...                  # (existing tests)

platforms/                   # NEW: Platform-specific build configs
├── mcu-arm-cortex-m/
│   ├── Makefile
│   ├── kernel.config
│   └── rootfs.sh
├── iot-rpi/
│   ├── Makefile
│   ├── kernel.config
│   └── rootfs.sh
├── desktop-x86/             # (current, refactored from base/)
│   ├── Makefile
│   ├── kernel.config
│   └── rootfs.sh
└── server-arm64/
    ├── Makefile
    ├── kernel.config
    └── rootfs.sh
```

**Structure Decision**: The existing single-platform structure under `base/` and `supervisor/` is extended with new subdirectories for runtime abstraction (`runtime/`), HAL (`hal/`), platform profiles (`profile/`), OTA (`ota/`), and observability (`observability/`). Platform-specific build configurations move to a top-level `platforms/` directory. The 500-line file limit rule continues to apply.

## Complexity Tracking

No constitution violations. All new modules fit within existing patterns (separate files per subsystem, capability-driven, explicit configuration).
