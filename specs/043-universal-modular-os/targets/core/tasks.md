# Tasks: Core Foundation (Phase 1 + 2)

**Parent spec**: `specs/043-universal-modular-os/`
**Branch**: `feat/043-core`
**Blocks**: ALL target market branches — complete this first

These tasks create the shared infrastructure that every market segment depends on.
No user-story-specific code lives here.

---

## Phase 1: Setup

- [ ] T001 Create `supervisor/src/runtime/` directory and `mod.rs` with `WasmRuntime` trait definition per contracts/runtime-adapter.md
- [ ] T002 Create `supervisor/src/hal/` directory and `mod.rs` with GPIO/I2C/SPI/UART/ADC trait definitions per contracts/hal-interface.md
- [ ] T003 Create `supervisor/src/profile/` directory and `mod.rs` with `PlatformProfile` struct per contracts/platform-profile-schema.md
- [ ] T004 [P] Create `supervisor/src/ota/` directory and `mod.rs` with `OtaManager` and `AbSlot` structs per data-model.md
- [ ] T005 [P] Create `supervisor/src/observability/` directory and `mod.rs` with `Heartbeat` struct per data-model.md
- [ ] T006 [P] Create `supervisor/src/capability/` directory and `mod.rs` with `PeripheralCapability` struct per data-model.md
- [ ] T007 Create `platforms/` directory with subdirectories: `mcu-arm-cortex-m/`, `iot-rpi/`, `desktop-x86/`, `server-arm64/`
- [ ] T008 [P] Create `supervisor/src/profile/profiles/` directory with 6 platform profile TOML files: `mcu-minimal.toml`, `iot-edge.toml`, `robotics-rt.toml`, `desktop-full.toml`, `server-headless.toml`, `mobile.toml`

**Checkpoint**: Directory structure complete, all trait definitions and structs compiled. `cargo check` passes.

---

## Phase 2: Foundational (Blocking Prerequisites)

- [ ] T009 Write failing unit tests for `WasmRuntime` trait in `supervisor/tests/runtime_adapter.rs` — test instantiate, execute, terminate, memory_usage
- [ ] T010 Implement Wasmtime adapter in `supervisor/src/runtime/wasmtime.rs` implementing `WasmRuntime` trait — make T009 tests pass
- [ ] T011 Write failing unit tests for platform profile loading in `supervisor/tests/platform_profile.rs` — test TOML parsing, validation rules, module selection
- [ ] T012 Implement profile loader in `supervisor/src/profile/mod.rs` — parse TOML profiles, validate against schema, select modules. Make T011 tests pass
- [ ] T013 [P] Write failing unit tests for peripheral capability enforcement in `supervisor/tests/peripheral_capability.rs` — test per-pin GPIO, per-bus I2C, deny undeclared
- [ ] T014 [P] Implement peripheral capability enforcer in `supervisor/src/capability/peripheral.rs` — parse `[capabilities.gpio]`, `[capabilities.i2c]` etc. from vyoma.toml, enforce at spawn time. Make T013 tests pass
- [ ] T015 [P] Write failing unit tests for heartbeat emission in `supervisor/tests/heartbeat.rs` — test JSON-line format, interval, module metadata
- [ ] T016 [P] Implement heartbeat emitter in `supervisor/src/observability/heartbeat.rs` — emit structured JSON-line heartbeats per data-model.md format. Make T015 tests pass
- [ ] T017 Integrate profile loader into supervisor boot sequence in `supervisor/src/main.rs` — read `PLATFORM` env var, load profile, conditionally include/exclude modules
- [ ] T018 Integrate peripheral capability enforcer into manifest parser in `supervisor/src/main.rs` — extend existing `parse_manifest()` to handle peripheral capability sections
- [ ] T019 Update `Makefile` to accept `PLATFORM=<name>` argument, select kernel config and rootfs script from `platforms/<name>/`

**Checkpoint**: Supervisor loads platform profiles, enforces peripheral capabilities, emits heartbeats. `cargo check` and `cargo test` pass. `make build PLATFORM=desktop-full` works.

---

## After merge → unblock all target branches

Once this branch is merged to `develop`, the following branches can begin work:
- `feat/043-iot-embedded` (Phase 3 + Phase 7)
- `feat/043-robotics` (Phase 4)
- `feat/043-desktop-server` (Phase 5)
- `feat/043-hpc` (Phase 6)
