# Tasks: Robotics Target (US2 — P2)

**Parent spec**: `specs/043-universal-modular-os/`
**Branch**: `feat/043-robotics`
**Prerequisite**: `feat/043-core` merged to `develop` first

**Goal**: Hot-swap individual WASM modules on a running robot stack without disrupting other modules. HAL drivers for GPIO, I2C, UART with exclusive-access conflict detection.

**Independent Test**: Run 4 WASM modules (camera, LIDAR, planner, motor), replace planner live, verify other 3 continue uninterrupted.

---

## Phase 4: User Story 2 — Robotics Modular Sensor/Actuator Stack

### Tests

- [ ] T031 [P] Write failing test for hot-swap in `supervisor/tests/hot_swap.rs` — replace running module, verify other modules unaffected
- [ ] T032 [P] Write failing test for exclusive peripheral access in `supervisor/tests/exclusive_peripheral.rs` — two modules claiming same GPIO pin as output, second one rejected at spawn

### Implementation

- [ ] T033 [P] Implement HAL GPIO driver (mock) in `supervisor/src/hal/gpio.rs` — implement `GpioDriver` trait with mock hardware for testing
- [ ] T034 [P] Implement HAL I2C driver (mock) in `supervisor/src/hal/i2c.rs` — implement `I2cDriver` trait with mock hardware
- [ ] T035 [P] Implement HAL UART driver (mock) in `supervisor/src/hal/uart.rs` — implement `UartDriver` trait with mock hardware
- [ ] T036 Implement hot-swap coordinator in `supervisor/src/main.rs` — extend `restart` command to support live module replacement without stopping other modules. Make T031 pass
- [ ] T037 Implement peripheral conflict detection in `supervisor/src/capability/peripheral.rs` — reject modules that declare conflicting exclusive access to same peripheral. Make T032 pass
- [ ] T038 Wire HAL host functions into runtime adapter in `supervisor/src/runtime/wasmtime.rs` — expose GPIO/I2C/UART as WASI host functions gated by capability checks
- [ ] T039 Create sample robotics WASM apps: `apps/motor-driver/` (GPIO output), `apps/sensor-reader/` (I2C input), `apps/path-planner/` (stdio only), `apps/lidar-sim/` (UART input)
- [ ] T040 Smoke test: run 4 robotics apps concurrently, hot-swap `path-planner` while others run, verify zero disruption

**Checkpoint**: Modular robotics stack works with hot-swap. Peripheral conflict detection prevents unsafe hardware sharing. HAL host functions operational.

---

## Acceptance Criteria

1. 4 WASM modules running → replace one → other 3 continue without interruption
2. Two apps declaring the same GPIO pin (exclusive) → second app rejected at spawn with clear error
3. Motor controller (`gpio = true`) can call GPIO host functions; camera (`gpio = false`) gets WASM trap when attempting GPIO call

## Key files to touch

- `supervisor/src/hal/gpio.rs` (new)
- `supervisor/src/hal/i2c.rs` (new)
- `supervisor/src/hal/uart.rs` (new)
- `supervisor/src/capability/peripheral.rs` (extend conflict detection)
- `supervisor/src/runtime/wasmtime.rs` (wire HAL host functions)
- `supervisor/src/main.rs` (extend hot-swap in restart command)
- `apps/motor-driver/` (new app)
- `apps/sensor-reader/` (new app)
- `apps/path-planner/` (new app)
- `apps/lidar-sim/` (new app)
- `supervisor/tests/hot_swap.rs` (new)
- `supervisor/tests/exclusive_peripheral.rs` (new)
