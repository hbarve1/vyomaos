# Tasks: IoT / Embedded Target (US1 — P1 MVP)

**Parent spec**: `specs/043-universal-modular-os/`
**Branch**: `feat/043-iot-embedded`
**Prerequisite**: `feat/043-core` merged to `develop` first

**Goal**: Deploy the same WASM app binary to multiple architecture targets with capability enforcement and OTA A/B-slot updates.

**Independent Test**: Deploy same `.wasm` to ARM and x86 QEMU instances, verify identical output and capability enforcement.

---

## Phase 3: User Story 1 — IoT Device Deploys Secure Edge Firmware

### Tests

- [ ] T020 [P] Write failing test for cross-arch binary parity in `supervisor/tests/cross_arch_parity.rs` — same WASM binary, same output on different runtime configs
- [ ] T021 [P] Write failing test for A/B slot rollback in `supervisor/tests/ota_rollback.rs` — deploy bad module, verify automatic revert to previous slot
- [ ] T022 [P] Write failing test for capability denial on undeclared peripheral in `supervisor/tests/capability_deny_peripheral.rs` — app without GPIO declaration cannot call GPIO host functions

### Implementation

- [ ] T023 Implement A/B slot manager in `supervisor/src/ota/ab_slot.rs` — manage two module slots, swap active slot, track slot state per data-model.md
- [ ] T024 Implement health check validator in `supervisor/src/ota/health_check.rs` — monitor new module for configurable duration, trigger rollback on failure. Make T021 pass
- [ ] T025 Implement OTA coordinator in `supervisor/src/ota/mod.rs` — orchestrate download → verify → deploy → health-check → commit/rollback flow
- [ ] T026 Integrate OTA system into supervisor lifecycle in `supervisor/src/main.rs` — add `@supervisor: ota-update <name> <path>` IPC command
- [ ] T027 Create IoT platform build config in `platforms/iot-rpi/Makefile` and `platforms/iot-rpi/kernel.config` — ARM64 kernel config with virtio + 9P
- [ ] T028 Create IoT rootfs script in `platforms/iot-rpi/rootfs.sh` — include supervisor + wasm3/WAMR runtime + boot apps from `iot-edge.toml` profile
- [ ] T029 Verify cross-arch parity: build a test WASM app, run on `desktop-x86` and `iot-rpi` platforms, compare output byte-for-byte. Make T020 pass
- [ ] T030 Verify capability denial: deploy app without GPIO, confirm supervisor blocks HAL host function calls. Make T022 pass

**Checkpoint**: Same WASM binary runs on x86 and ARM with identical output. OTA with A/B rollback works. Capability denial enforced on peripherals. `make test` passes.

---

## Phase 7: wasm3 MCU Runtime Integration

**Goal**: Integrate wasm3 interpreter as the MCU/IoT runtime, completing the tiered runtime strategy.

### Tests

- [ ] T052 [P] Write failing test for wasm3 runtime adapter in `supervisor/tests/wasm3_adapter.rs` — test instantiate, execute, terminate, memory_usage on wasm3
- [ ] T053 [P] Write failing test for runtime parity in `supervisor/tests/runtime_parity.rs` — same WASM binary, same output on wasm3 vs Wasmtime

### Implementation

- [ ] T054 Add wasm3 Rust bindings as dependency in `supervisor/Cargo.toml` — evaluate `wasm3` crate compatibility with musl target
- [ ] T055 Implement wasm3 adapter in `supervisor/src/runtime/wasm3.rs` implementing `WasmRuntime` trait. Make T052 pass
- [ ] T056 Wire wasm3 adapter into profile loader — when profile declares `runtime = "wasm3"`, use wasm3 adapter. Make T053 pass
- [ ] T057 Create MCU platform build config in `platforms/mcu-arm-cortex-m/Makefile` — cross-compile supervisor for ARM Cortex-M with wasm3
- [ ] T058 Verify MCU profile boots and runs WASM app with wasm3 interpreter under QEMU ARM emulation

**Checkpoint**: Tiered runtime operational — wasm3 on MCU, Wasmtime on desktop. Same WASM binary produces same output on both.

---

## Acceptance Criteria

1. Same `.wasm` binary deployed to ARM and x86 QEMU → identical output, no recompilation
2. App with `network = false` manifest → supervisor blocks TCP open attempt
3. OTA update → module replaced without rebooting; health check failure → automatic rollback
4. wasm3 and Wasmtime produce equivalent output for same input WASM

## Key files to touch

- `supervisor/src/ota/ab_slot.rs` (new)
- `supervisor/src/ota/health_check.rs` (new)
- `supervisor/src/ota/mod.rs` (new)
- `supervisor/src/runtime/wasm3.rs` (new)
- `supervisor/src/main.rs` (extend OTA IPC command)
- `platforms/iot-rpi/` (new platform directory)
- `platforms/mcu-arm-cortex-m/` (new platform directory)
- `supervisor/tests/ota_rollback.rs` (new)
- `supervisor/tests/cross_arch_parity.rs` (new)
- `supervisor/tests/wasm3_adapter.rs` (new)
