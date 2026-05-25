# Tasks: VyomaOS Universal Modular Operating System

**Input**: Design documents from `specs/043-universal-modular-os/`

**Prerequisites**: plan.md (required), spec.md (required), research.md, data-model.md, contracts/

**Tests**: Included per Constitution Principle II (Test-First Development).

**Organization**: Tasks are grouped by user story to enable independent implementation and testing of each story.

## Format: `[ID] [P?] [Story] Description`

- **[P]**: Can run in parallel (different files, no dependencies)
- **[Story]**: Which user story this task belongs to (e.g., US1, US2, US3)
- Include exact file paths in descriptions

## Phase 1: Setup (Shared Infrastructure)

**Purpose**: Project initialization — new directories, build infrastructure for multi-platform support

- [ ] T001 Create `supervisor/src/runtime/` directory and `mod.rs` with `WasmRuntime` trait definition per contracts/runtime-adapter.md
- [ ] T002 Create `supervisor/src/hal/` directory and `mod.rs` with GPIO/I2C/SPI/UART/ADC trait definitions per contracts/hal-interface.md
- [ ] T003 Create `supervisor/src/profile/` directory and `mod.rs` with `PlatformProfile` struct per contracts/platform-profile-schema.md
- [ ] T004 [P] Create `supervisor/src/ota/` directory and `mod.rs` with `OtaManager` and `AbSlot` structs per data-model.md
- [ ] T005 [P] Create `supervisor/src/observability/` directory and `mod.rs` with `Heartbeat` struct per data-model.md
- [ ] T006 [P] Create `supervisor/src/capability/` directory and `mod.rs` with `PeripheralCapability` struct per data-model.md
- [ ] T007 Create `platforms/` directory with subdirectories: `mcu-arm-cortex-m/`, `iot-rpi/`, `desktop-x86/`, `server-arm64/`
- [ ] T008 [P] Create `supervisor/src/profile/profiles/` directory with 6 platform profile TOML files per contracts/platform-profile-schema.md: `mcu-minimal.toml`, `iot-edge.toml`, `robotics-rt.toml`, `desktop-full.toml`, `server-headless.toml`, `mobile.toml`

**Checkpoint**: Directory structure complete, all trait definitions and structs compiled. `cargo check` passes.

---

## Phase 2: Foundational (Blocking Prerequisites)

**Purpose**: Core abstractions that ALL user stories depend on — runtime adapter, profile loader, capability enforcer

**⚠️ CRITICAL**: No user story work can begin until this phase is complete

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

**Checkpoint**: Foundation ready — supervisor loads platform profiles, enforces peripheral capabilities, emits heartbeats. `cargo check` and `cargo test` pass. `make build PLATFORM=desktop-full` works.

---

## Phase 3: User Story 1 — IoT Device Deploys Secure Edge Firmware (Priority: P1) 🎯 MVP

**Goal**: Deploy the same WASM app binary to multiple architecture targets with capability enforcement and OTA updates

**Independent Test**: Deploy same `.wasm` to ARM and x86 QEMU instances, verify identical output and capability enforcement

### Tests for User Story 1

- [ ] T020 [P] [US1] Write failing test for cross-arch binary parity in `supervisor/tests/cross_arch_parity.rs` — same WASM binary, same output on different runtime configs
- [ ] T021 [P] [US1] Write failing test for A/B slot rollback in `supervisor/tests/ota_rollback.rs` — deploy bad module, verify automatic revert to previous slot
- [ ] T022 [P] [US1] Write failing test for capability denial on undeclared peripheral in `supervisor/tests/capability_deny_peripheral.rs` — app without GPIO declaration cannot call GPIO host functions

### Implementation for User Story 1

- [ ] T023 [US1] Implement A/B slot manager in `supervisor/src/ota/ab_slot.rs` — manage two module slots, swap active slot, track slot state per data-model.md
- [ ] T024 [US1] Implement health check validator in `supervisor/src/ota/health_check.rs` — monitor new module for configurable duration, trigger rollback on failure. Make T021 pass
- [ ] T025 [US1] Implement OTA coordinator in `supervisor/src/ota/mod.rs` — orchestrate download → verify → deploy → health-check → commit/rollback flow
- [ ] T026 [US1] Integrate OTA system into supervisor lifecycle in `supervisor/src/main.rs` — add `@supervisor: ota-update <name> <path>` IPC command
- [ ] T027 [US1] Create IoT platform build config in `platforms/iot-rpi/Makefile` and `platforms/iot-rpi/kernel.config` — ARM64 kernel config with virtio + 9P
- [ ] T028 [US1] Create IoT rootfs script in `platforms/iot-rpi/rootfs.sh` — include supervisor + wasm3/WAMR runtime + boot apps from `iot-edge.toml` profile
- [ ] T029 [US1] Verify cross-arch parity: build a test WASM app, run on `desktop-x86` and `iot-rpi` platforms, compare output byte-for-byte. Make T020 pass
- [ ] T030 [US1] Verify capability denial: deploy app without GPIO, confirm supervisor blocks HAL host function calls. Make T022 pass

**Checkpoint**: Same WASM binary runs on x86 and ARM with identical output. OTA with A/B rollback works. Capability denial enforced on peripherals. `make test` passes.

---

## Phase 4: User Story 2 — Robotics Modular Sensor/Actuator Stack (Priority: P2)

**Goal**: Hot-swap individual WASM modules on a running system without disrupting other modules

**Independent Test**: Run 4 WASM modules, replace one live, verify other 3 continue uninterrupted

### Tests for User Story 2

- [ ] T031 [P] [US2] Write failing test for hot-swap in `supervisor/tests/hot_swap.rs` — replace running module, verify other modules unaffected
- [ ] T032 [P] [US2] Write failing test for exclusive peripheral access in `supervisor/tests/exclusive_peripheral.rs` — two modules claiming same GPIO pin as output, second one rejected at spawn

### Implementation for User Story 2

- [ ] T033 [P] [US2] Implement HAL GPIO driver (mock) in `supervisor/src/hal/gpio.rs` — implement `GpioDriver` trait with mock hardware for testing
- [ ] T034 [P] [US2] Implement HAL I2C driver (mock) in `supervisor/src/hal/i2c.rs` — implement `I2cDriver` trait with mock hardware
- [ ] T035 [P] [US2] Implement HAL UART driver (mock) in `supervisor/src/hal/uart.rs` — implement `UartDriver` trait with mock hardware
- [ ] T036 [US2] Implement hot-swap coordinator in `supervisor/src/main.rs` — extend `restart` command to support live module replacement without stopping other modules. Make T031 pass
- [ ] T037 [US2] Implement peripheral conflict detection in `supervisor/src/capability/peripheral.rs` — reject modules that declare conflicting exclusive access to same peripheral. Make T032 pass
- [ ] T038 [US2] Wire HAL host functions into runtime adapter in `supervisor/src/runtime/wasmtime.rs` — expose GPIO/I2C/UART as WASI host functions gated by capability checks
- [ ] T039 [US2] Create sample robotics WASM apps: `apps/motor-driver/` (GPIO output), `apps/sensor-reader/` (I2C input), `apps/path-planner/` (stdio only), `apps/lidar-sim/` (UART input)
- [ ] T040 [US2] Smoke test: run 4 robotics apps concurrently, hot-swap `path-planner` while others run, verify zero disruption

**Checkpoint**: Modular robotics stack works with hot-swap. Peripheral conflict detection prevents unsafe hardware sharing. HAL host functions operational.

---

## Phase 5: User Story 3 — Enterprise Cross-Platform Desktop + Server (Priority: P3)

**Goal**: Same productivity app runs on desktop (with display) and server (headless) from single binary

**Independent Test**: Build one app, deploy to desktop and server profiles, verify correct behavior on both

### Tests for User Story 3

- [ ] T041 [P] [US3] Write failing test for profile-dependent behavior in `supervisor/tests/profile_behavior.rs` — same binary uses VYOMA_DRAW on desktop profile, HTTP on server profile

### Implementation for User Story 3

- [ ] T042 [US3] Create server platform build config in `platforms/server-arm64/Makefile` and `platforms/server-arm64/kernel.config` — headless, network-enabled
- [ ] T043 [US3] Create server rootfs script in `platforms/server-arm64/rootfs.sh` — supervisor with `server-headless.toml` profile, no display modules
- [ ] T044 [US3] Create sample dual-mode app in `apps/doc-viewer/` — renders via VYOMA_DRAW when display capability is available, serves via HTTP when network capability is available
- [ ] T045 [US3] Verify dual-mode behavior: deploy `doc-viewer.wasm` to `desktop-full` and `server-headless` profiles, confirm correct mode activation. Make T041 pass
- [ ] T046 [US3] Verify capability enforcement: on server profile with `network = false` manifest, confirm app cannot open sockets

**Checkpoint**: Same binary adapts to desktop and server environments. Capability enforcement works across profiles.

---

## Phase 6: User Story 4 — HPC Reproducible Compute Jobs (Priority: P4)

**Goal**: Deterministic WASM compute jobs produce byte-identical output across architectures

**Independent Test**: Run same compute WASM job on x86-64 and ARM64 nodes, compare output byte-for-byte

### Tests for User Story 4

- [ ] T047 [P] [US4] Write failing test for wasm64 module loading in `supervisor/tests/wasm64_loading.rs` — verify runtime accepts wasm64 binaries on supported platforms

### Implementation for User Story 4

- [ ] T048 [US4] Extend runtime adapter trait to support wasm64 in `supervisor/src/runtime/mod.rs` — add `WasmTarget::Wasm64` variant, reject on runtimes that don't support it
- [ ] T049 [US4] Update Wasmtime adapter in `supervisor/src/runtime/wasmtime.rs` — enable 64-bit memory support when wasm64 target detected. Make T047 pass
- [ ] T050 [US4] Create sample compute job in `apps/compute-bench/` — deterministic math workload (SHA-256 hash chain) targeting wasm64-wasip2
- [ ] T051 [US4] Verify byte-identical output: run `compute-bench.wasm` on two architecture configurations, compare stdout byte-for-byte

**Checkpoint**: wasm64 support operational. Deterministic compute verified across architectures.

---

## Phase 7: wasm3 MCU Runtime Integration (Priority: P1 dependency)

**Goal**: Integrate wasm3 interpreter as the MCU/IoT runtime, completing the tiered runtime strategy

**Independent Test**: Run same WASM app on wasm3 and Wasmtime, compare output

### Tests for User Story 1 (MCU path)

- [ ] T052 [P] [US1] Write failing test for wasm3 runtime adapter in `supervisor/tests/wasm3_adapter.rs` — test instantiate, execute, terminate, memory_usage on wasm3
- [ ] T053 [P] [US1] Write failing test for runtime parity in `supervisor/tests/runtime_parity.rs` — same WASM binary, same output on wasm3 vs Wasmtime

### Implementation

- [ ] T054 [US1] Add wasm3 Rust bindings as dependency in `supervisor/Cargo.toml` — evaluate `wasm3` crate compatibility with musl target
- [ ] T055 [US1] Implement wasm3 adapter in `supervisor/src/runtime/wasm3.rs` implementing `WasmRuntime` trait. Make T052 pass
- [ ] T056 [US1] Wire wasm3 adapter into profile loader — when profile declares `runtime = "wasm3"`, use wasm3 adapter. Make T053 pass
- [ ] T057 [US1] Create MCU platform build config in `platforms/mcu-arm-cortex-m/Makefile` — cross-compile supervisor for ARM Cortex-M with wasm3
- [ ] T058 [US1] Verify MCU profile boots and runs WASM app with wasm3 interpreter under QEMU ARM emulation

**Checkpoint**: Tiered runtime operational — wasm3 on MCU, Wasmtime on desktop. Same WASM binary produces same output on both.

---

## Phase 8: Polish & Cross-Cutting Concerns

**Purpose**: Documentation, quality, performance across all platforms

- [ ] T059 [P] Update `CLAUDE.md` with multi-platform build instructions and new module descriptions
- [ ] T060 [P] Update `README.md` with universal OS positioning, platform targets, and platform profile documentation
- [ ] T061 [P] Update `website/` documentation pages with new architecture content (HAL, profiles, OTA, tiered runtime)
- [ ] T062 Add `make check-profiles` target in `Makefile` — validate all platform profile TOML files against schema
- [ ] T063 Add `make test-all-platforms` target in `Makefile` — run unit tests + smoke test for each platform profile
- [ ] T064 [P] Add observability documentation in `docs/observability.md` — heartbeat format, OpenTelemetry integration guide
- [ ] T065 [P] Add OTA documentation in `docs/ota-updates.md` — A/B slot mechanism, health check flow, rollback behavior
- [ ] T066 Run `quickstart.md` validation — follow the quickstart guide end-to-end and verify all steps work
- [ ] T067 Performance profiling: measure supervisor boot time, WASM cold start, and memory usage per platform profile
- [ ] T068 Security audit: verify all peripheral capability paths enforce deny-by-default, no ambient authority leaks

**Checkpoint**: All documentation current. All platforms build and pass tests. Performance and security validated.

---

## Dependencies & Execution Order

### Phase Dependencies

- **Setup (Phase 1)**: No dependencies — can start immediately
- **Foundational (Phase 2)**: Depends on Setup completion — BLOCKS all user stories
- **US1 (Phase 3)**: Depends on Foundational (Phase 2)
- **US2 (Phase 4)**: Depends on Foundational (Phase 2) — can run in parallel with US1
- **US3 (Phase 5)**: Depends on Foundational (Phase 2) — can run in parallel with US1/US2
- **US4 (Phase 6)**: Depends on Foundational (Phase 2) — can run in parallel with others
- **wasm3 (Phase 7)**: Depends on Foundational (Phase 2) — can run in parallel with US2/US3/US4
- **Polish (Phase 8)**: Depends on all desired user stories being complete

### User Story Dependencies

- **US1 (IoT)**: No dependency on other stories. MVP target.
- **US2 (Robotics)**: Independent of US1 but shares HAL infrastructure from Foundational phase
- **US3 (Enterprise)**: Independent. Requires server platform config.
- **US4 (HPC)**: Independent. Requires wasm64 toolchain readiness.
- **wasm3 (MCU runtime)**: Extends US1 with the MCU-specific runtime path

### Within Each User Story

- Tests MUST be written and FAIL before implementation (Constitution Principle II)
- Trait implementations before integrations
- Core logic before IPC/command wiring
- Story complete before moving to next priority

### Parallel Opportunities

- T004, T005, T006 can run in parallel (different directories)
- T013/T014 and T015/T016 can run in parallel (different subsystems)
- T020, T021, T022 can run in parallel (different test files)
- T033, T034, T035 can run in parallel (different HAL drivers)
- US1, US2, US3, US4 can all run in parallel after Foundational phase
- All Phase 8 documentation tasks can run in parallel

---

## Parallel Example: User Story 1

```bash
# Launch all tests for US1 together:
Task: "Write failing test for cross-arch parity in supervisor/tests/cross_arch_parity.rs"
Task: "Write failing test for A/B slot rollback in supervisor/tests/ota_rollback.rs"
Task: "Write failing test for capability denial in supervisor/tests/capability_deny_peripheral.rs"

# Launch all US1 implementation that touches different files:
Task: "Implement A/B slot manager in supervisor/src/ota/ab_slot.rs"
Task: "Create IoT platform build config in platforms/iot-rpi/Makefile"
```

---

## Implementation Strategy

### MVP First (User Story 1 Only)

1. Complete Phase 1: Setup (T001–T008)
2. Complete Phase 2: Foundational (T009–T019)
3. Complete Phase 3: User Story 1 — IoT (T020–T030)
4. **STOP and VALIDATE**: Test cross-arch parity, OTA rollback, capability enforcement
5. Deploy/demo: Same WASM binary on x86 + ARM with OTA updates

### Incremental Delivery

1. Setup + Foundational → Foundation ready
2. US1 (IoT) → Test independently → Demo (MVP!)
3. US2 (Robotics) → Test hot-swap → Demo
4. Phase 7 (wasm3 MCU) → Test runtime parity → Demo MCU target
5. US3 (Enterprise) → Test dual-mode → Demo cross-platform
6. US4 (HPC) → Test wasm64 determinism → Demo
7. Polish → Documentation + validation

### Parallel Team Strategy

With multiple developers:

1. Team completes Setup + Foundational together
2. Once Foundational is done:
   - Developer A: US1 (IoT + OTA)
   - Developer B: US2 (Robotics + HAL)
   - Developer C: Phase 7 (wasm3 runtime)
3. After US1/US2 complete:
   - Developer A: US3 (Enterprise)
   - Developer B: US4 (HPC)
   - Developer C: Polish

---

## Notes

- [P] tasks = different files, no dependencies
- [Story] label maps task to specific user story for traceability
- Each user story should be independently completable and testable
- Tests MUST fail before implementing (Constitution Principle II)
- Commit after each task (Constitution compliance)
- 500-line file limit applies to all new `.rs` files
- `cargo check` MUST pass with zero warnings before each commit
