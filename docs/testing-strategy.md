# VyomaOS Comprehensive Multi-Platform Testing Strategy

**Spec**: `specs/043-universal-modular-os/` | **Date**: 2026-05-25 | **Status**: Living document

---

## Table of Contents

1. [Testing Philosophy](#1-testing-philosophy)
2. [Testing Layers](#2-testing-layers)
   - [Layer 1: Unit Tests](#layer-1-unit-tests)
   - [Layer 2: Cargo Check / Clippy](#layer-2-cargo-check--clippy)
   - [Layer 3: Manifest Validation](#layer-3-manifest-validation)
   - [Layer 4: Platform Smoke Test](#layer-4-platform-smoke-test)
   - [Layer 5: Integration Tests](#layer-5-integration-tests)
   - [Layer 6: Cross-Arch Parity Test](#layer-6-cross-arch-parity-test)
   - [Layer 7: Security / Capability Denial Tests](#layer-7-security--capability-denial-tests)
   - [Layer 8: Performance Baseline](#layer-8-performance-baseline)
3. [Per-Platform Testing Guide](#3-per-platform-testing-guide)
   - [Platform 1: MCU (ARM Cortex-M)](#platform-1-mcu-arm-cortex-m-mcu-minimaltoml)
   - [Platform 2: IoT/Embedded (ARM64 SBC)](#platform-2-iotembedded-arm64-sbc-iot-edgetoml)
   - [Platform 3: Robotics (ARM64)](#platform-3-robotics-arm64-robotics-rttoml)
   - [Platform 4: Mobile/Tablet (ARM64)](#platform-4-mobiletablet-arm64-mobiletoml)
   - [Platform 5: Desktop (x86-64)](#platform-5-desktop-x86-64-desktop-fulltoml)
   - [Platform 6: Server (ARM64)](#platform-6-server-arm64-server-headlesstoml)
   - [Platform 7: HPC (x86-64 + ARM64)](#platform-7-hpc-x86-64--arm64)
4. [CI/CD Pipeline Design](#4-cicd-pipeline-design)
5. [Test Data and Fixtures](#5-test-data-and-fixtures)
6. [Acceptance Test Checklists](#6-acceptance-test-checklists)
7. [Regression Test Policy](#7-regression-test-policy)

---

## 1. Testing Philosophy

### Why Multi-Layer Testing Matters for a Capability-Secure WASM OS

VyomaOS occupies an unusual position in the OS landscape: it is simultaneously a kernel runtime, a security enforcement boundary, and an application platform. Its central value proposition — that any `.wasm` binary runs identically and securely on hardware from 128 KB MCUs to multi-terabyte HPC nodes — only holds if the testing strategy exercises every link in that chain.

Traditional OS testing focuses on a single hardware target and trusts the operating environment. VyomaOS testing must prove three cross-cutting invariants that no single test layer can address alone. This is why a multi-layer approach is mandatory: each layer catches a category of failure that the others miss.

### The Three Invariants That Must Hold on Every Platform

**Invariant A — Capability Isolation**: An application that does not declare a capability in its `vyoma.toml` manifest MUST have zero access to the corresponding resource interface. This is not a filtering policy on top of a permissive baseline; it is structural. The supervisor wires up only declared WASI imports at spawn time. Testing must confirm that undeclared capabilities produce a denied call (or, for HAL peripherals, a WASM trap) rather than silent access or a soft warning. This invariant is tested by Layer 7 and enforced structurally by Layers 1 and 3.

**Invariant B — Binary Portability**: The same `.wasm` binary, compiled once, must produce identical observable output when run on any supported architecture (x86-64, ARM64, ARM32, RISC-V). "Identical" means byte-for-byte stdout agreement for deterministic workloads and behavioral equivalence for I/O-bound apps. This invariant underpins the entire "universal deployment" claim. It is tested by Layer 6 and by the per-platform smoke tests of Layer 4.

**Invariant C — Supervisor Stability**: The Rust supervisor is PID 1. If it panics or exits, the system halts. No amount of WASM-level sandboxing helps if the supervisor itself crashes under load, bad input, or unexpected platform conditions. Testing must cover the supervisor's state machine (manifest parsing, IPC routing, lifecycle management, display protocol parsing) exhaustively at unit level (Layer 1), and must confirm end-to-end stability through boot and sustained operation (Layer 4).

### Test-First Development Connection

All new supervisor modules follow the TDD discipline established in `CLAUDE.md` and the Constitution. Tests are committed in the same PR as, or before, the implementation they validate. Every regression fix must add a test that would have caught the bug. The testing layers described below operationalize this principle at scale across seven platform targets.

---

## 2. Testing Layers

### Layer 1: Unit Tests

**What it tests**: Per-module logic in the supervisor, with no QEMU, no Docker, and no external processes. Functions are tested against their contracts in isolation using Rust's built-in test framework.

**How to run**:
```bash
make unit-test
# Equivalent to: docker run ... cargo test --manifest-path supervisor/Cargo.toml \
#                 --target x86_64-unknown-linux-musl
```

**What it catches**:
- Logic errors in parsing (manifests, TOML profiles, IPC messages, ESC sequences)
- State machine bugs in lifecycle management (should_restart, focus transfer, watchdog backoff)
- Display primitive math (word wrap, clipping, blending, font dimensions)
- IPC routing invariants (parse_ipc_target, broadcast, reply routing)
- Window management geometry (tiling layout, snap layout, drag clamping, hit testing)
- Arithmetic correctness (CPU usage formatting, uptime formatting, FPS calculation)

**Existing test files** (all in `supervisor/tests/`):

| File | What it covers |
|------|---------------|
| `manifest_tests.rs` | TOML parsing, unknown fields, missing fields, type errors, capability defaults |
| `ipc_tests.rs` | `parse_ipc_target`, broadcast, reply routing, `format_pong_reply`, `format_app_list`, `validate_kill_target` |
| `ipc_test.rs` | `format_version`, `parse_log_level`, `LogLevel` ordering |
| `lifecycle_tests.rs` | `should_restart`, `is_watchdog_kill`, `format_ps_line`, focus transfer, crash notifications, `format_cpu`, `format_uptime` |
| `logging_tests.rs` | `format_log`, ISO-8601 timestamps, log level formatting, app field inclusion |
| `loglevel_test.rs` | `parse_log_level` case-insensitivity, invalid values |
| `display_test.rs` | `titlebar_color`, `border_color`, word wrap, cursor draw/restore, `app_accent_color`, `blend_alpha`, `format_fps` |
| `font_test.rs` | Glyph dimensions, font size parsing, pixel-doubling for Large scale |
| `input_test.rs` | ESC sequence classification, Alt+Tab/W/F/Shift+Tab, focus cycle forward/backward |
| `mouse_test.rs` | Traffic-light hit detection, window hit test, local coord conversion, mouse event formatting, titlebar state colors, drag delta |
| `watchdog_test.rs` | Exponential backoff capping at 300 s, disabled watchdog |
| `window_test.rs` | `clip_fill`, `offset_text` clipping at window bounds |
| `window_layout_test.rs` | `compute_tiling`, `compute_tiling_with_hints`, `compute_snap_layout`, `clamp_tile_size`, MIN_WIN_W/H enforcement |
| `drag_test.rs` | Drag clamping (x/y min), `nearest_tiled_slot` snap, minimize strip layout |
| `menubar_test.rs` | `menubar_hit_app`, `menubar_label_width`, boundary pixels, multiple apps |
| `statusbar_test.rs` | `format_status_text` |

**Future test files** (to be added per spec-043 task plan):
- `supervisor/tests/runtime_adapter.rs` — `WasmRuntime` trait: instantiate, execute, terminate, memory_usage
- `supervisor/tests/platform_profile.rs` — TOML profile loading, validation rules, module selection
- `supervisor/tests/peripheral_capability.rs` — per-pin GPIO, per-bus I2C, deny undeclared
- `supervisor/tests/heartbeat.rs` — JSON-line heartbeat format, interval, module metadata
- `supervisor/tests/ota_rollback.rs` — A/B slot deploy, bad-module rollback
- `supervisor/tests/cross_arch_parity.rs` — same WASM binary, same output on different runtime configs
- `supervisor/tests/hot_swap.rs` — replace running module, verify other modules unaffected
- `supervisor/tests/exclusive_peripheral.rs` — two modules claiming same GPIO pin rejected
- `supervisor/tests/profile_behavior.rs` — same binary activates VYOMA_DRAW on desktop profile, HTTP on server profile
- `supervisor/tests/wasm64_loading.rs` — runtime accepts wasm64 binaries on supported platforms
- `supervisor/tests/touch_input.rs` — parse `VYOMA_INPUT:touch:tap:` and `swipe:` lines
- `supervisor/tests/mobile_profile.rs` — `mobile.toml` profile loads with correct capability set
- `supervisor/tests/wasm3_adapter.rs` — wasm3 runtime adapter: instantiate, execute, terminate
- `supervisor/tests/runtime_parity.rs` — same WASM binary, same output on wasm3 vs Wasmtime

**RUSTFLAGS enforcement**: All unit tests run with `RUSTFLAGS=-D warnings`. A single compiler warning is a test failure. This is enforced by the `make unit-test` target.

---

### Layer 2: Cargo Check / Clippy

**What it tests**: Type correctness and idiomatic Rust across the entire supervisor codebase, including all feature flag combinations. The 500-line file limit is enforced by a separate CI step.

**How to run**:
```bash
# Inside the builder container (make shell) or in CI:
RUSTFLAGS="-D warnings" cargo check \
  --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl

RUSTFLAGS="-D warnings" cargo clippy \
  --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl \
  -- -D clippy::all

# 500-line file limit check (runs in CI):
find supervisor/src apps -name '*.rs' | while read f; do
  lines=$(wc -l < "$f")
  if [ "$lines" -gt 500 ]; then
    echo "FAIL: $f has $lines lines (limit: 500)"
    exit 1
  fi
done
```

**What it catches**:
- Type mismatches, unused imports, dead code that would become warnings
- Common logic bugs flagged by Clippy (redundant clones, needless borrows, integer overflow patterns)
- Feature flag interactions that break compilation for optional components
- 500-line file size violations before they accumulate (Constitution rule)

**Notes**: `cargo check` runs in under 30 seconds in CI because it skips code generation. It should run on every PR commit, not just at merge time.

---

### Layer 3: Manifest Validation

**What it tests**: All `apps/*/vyoma.toml` files in the repository, parsed against the manifest schema defined in `docs/vyoma-manifest-schema.md`. Catches field-name typos, missing required fields, wrong value types, and unknown capability declarations.

**How to run**:
```bash
make check-manifests
# Equivalent to: docker run ... cargo run --manifest-path tools/check-manifests/Cargo.toml
```

**Expected output** (one line per app):
```
hello-world ... OK
shell       ... OK
gui-demo    ... OK
```
Exit non-zero if any manifest has an error.

**What it catches**:
- Typos in capability field names (e.g., `filesytem` instead of `filesystem`)
- Unknown capability fields that would silently be ignored without this gate
- Missing `[app].name`, `[app].wasm`, or `[app].version` fields
- Type errors (e.g., `watchdog_secs = "yes"`)
- New apps added without a valid manifest

**Integration with build**: `check-manifests` runs automatically before `make apps`. A broken manifest blocks the entire app compilation step.

---

### Layer 4: Platform Smoke Test

**What it tests**: Full end-to-end boot of VyomaOS from `bzImage + initramfs` inside QEMU, verifying that the supervisor starts, parses all manifests, spawns all declared apps, and emits the lifecycle ready signal within a timeout.

**How to run** (current desktop/x86-64 platform):
```bash
make smoke
# Waits up to 30 s for: [lifecycle] all apps spawned
# Prints: SMOKE: PASS or SMOKE: FAIL: <reason>
```

The smoke test script is at `base/scripts/smoke-test.sh`. It runs inside the Docker builder container so QEMU is guaranteed available.

**What it catches**:
- Manifest parser regressions that prevent app spawning
- Supervisor binary link errors or missing dynamic libraries
- Wasmtime binary missing from initramfs
- Boot configuration (boot.toml) syntax errors
- Kernel configuration regressions that prevent the VM from reaching userspace
- Crash bugs in the supervisor's startup sequence that unit tests cannot reach

**Per-platform extension**: Once per-platform build configs exist under `platforms/`, each platform needs its own smoke invocation. The QEMU command lines differ by machine type and architecture; see [Section 3](#3-per-platform-testing-guide) for exact invocations per platform.

**Signal observed**: `[lifecycle] all apps spawned` on serial console (stdout of QEMU). The supervisor's `lifecycle` subsystem emits this line when every app declared in `boot.toml` has been spawned at least once. This is tested structurally by `lifecycle_tests.rs`.

---

### Layer 5: Integration Tests

**What it tests**: Multi-component interactions that require two or more supervisor subsystems to be running simultaneously: IPC round-trips, OTA update flows, hot-swap with peer-module continuity, touch event dispatch, and cross-profile capability routing. These tests cannot be expressed as unit tests because they depend on the supervisor's event loop, IPC broker, and process manager operating in concert.

**How to run** (once implemented):
```bash
make integration-test
# Planned: docker run ... cargo test --features integration \
#           --manifest-path supervisor/Cargo.toml
```

**Individual integration scenarios** (keyed to spec-043 task IDs):

| Test | Task ID | Scenario |
|------|---------|----------|
| IPC round-trip | existing | ping app sends `@pong: hello`, pong app responds `@ping: pong N` |
| OTA A/B rollback | T021 | deploy `bad.wasm` to slot B, health check fails within 60 s, supervisor reverts to slot A |
| Hot-swap continuity | T031 | 4 WASM modules running; replace module 3 in-place; modules 1, 2, 4 continue without IPC disruption |
| Peripheral conflict rejection | T032 | two modules both declare `gpio_pins = [5]` as output; second spawn returns an error |
| Touch dispatch | TM02 | inject `VYOMA_INPUT:touch:tap:540,1000` into supervisor stdin; verify routed to focused app with `touch = true` |
| Profile-gated display | T041 | `doc-viewer.wasm` on `desktop-full` profile emits `VYOMA_DRAW:`; same binary on `server-headless` serves HTTP |

**What it catches**:
- Race conditions in the IPC broker under concurrent load
- State corruption when OTA updates arrive while apps are actively communicating
- Peripheral registry invariant violations that unit tests mock away
- Platform profile capability routing that requires the supervisor event loop to be live

---

### Layer 6: Cross-Arch Parity Test

**What it tests**: The same `.wasm` binary, run on two different architecture QEMU instances, produces byte-for-byte identical stdout for deterministic workloads.

**How to run** (once HPC platform target is complete, T051):
```bash
# Step 1: Build compute benchmark for wasm32 (or wasm64 when toolchain available)
cargo build --manifest-path apps/compute-bench/Cargo.toml \
  --target wasm32-wasip2 --release

# Step 2: Run on x86-64 instance (headless)
qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -nographic \
  -append "console=ttyS0 quiet" \
  -m 512M -no-reboot \
  2>/dev/null | grep "^RESULT:" > /tmp/x86_output.txt

# Step 3: Run on ARM64 instance (headless, using platforms/server-arm64/)
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a72 \
  -kernel out/arm64/Image \
  -initrd out/arm64/initramfs.cpio.gz \
  -nographic \
  -append "console=ttyAMA0 quiet" \
  -m 512M -no-reboot \
  2>/dev/null | grep "^RESULT:" > /tmp/arm64_output.txt

# Step 4: Compare
diff /tmp/x86_output.txt /tmp/arm64_output.txt && echo "PARITY: PASS" || echo "PARITY: FAIL"
```

**What it catches**:
- Endianness bugs in WASM apps that make assumptions about byte order
- Floating-point determinism regressions (WASM specifies IEEE 754 deterministic semantics)
- Host-architecture leakage through WASI host functions that expose non-deterministic values
- Wasmtime JIT compilation differences that produce different observable behavior on different hosts

**Limitation**: WASM floating-point is deterministic per spec, but host WASI calls (e.g., `clock_time_get`) are non-deterministic. Test apps used for parity testing must not call non-deterministic WASI functions. The `compute-bench` app (SHA-256 hash chain, T050) is designed to be fully deterministic.

---

### Layer 7: Security / Capability Denial Tests

**What it tests**: Every capability type has at least one test that confirms an app without that capability declaration cannot access the resource. This layer is the mechanical enforcement of Invariant A.

**How to run**:
```bash
# Unit-level capability denial tests run as part of make unit-test:
cargo test --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl \
  capability_deny
```

**Capability denial test matrix**:

| Capability | Test file | What is verified |
|------------|-----------|-----------------|
| `network` | existing (manifest_tests.rs policy layer) + T022 | app with `network = false` cannot open TCP socket; WASI socket import not wired |
| `filesystem` | manifest_tests.rs + integration | app without `filesystem` has no `/data` mount; `open()` on `/data/file` returns ENOENT or access error |
| `gpio` | T022, T030 | app without `gpio_pins` declaration receives WASM trap when calling GPIO host function |
| `i2c` | T013 | app without `i2c_bus` declaration is rejected before spawn |
| `touch` | TM01 | app with `touch = false` does not receive `VYOMA_INPUT:touch:` events |
| `display` | N/A (structural) | app without `display = true` does not receive `VYOMA_DRAW:` parse callbacks from supervisor |
| `shell` | N/A (structural) | app without `shell = true` cannot send `@supervisor:` IPC commands |
| `mouse` | existing (mouse routing in supervisor) | app without `mouse = true` does not receive `VYOMA_INPUT:mouse:` events |

**What it catches**:
- Capability enforcement regressions introduced when adding new capability types
- HAL host function registration that bypasses the capability check path
- WASI import wiring that exposes interfaces not declared in the manifest

---

### Layer 8: Performance Baseline

**What it tests**: Quantitative performance metrics against the acceptance criteria from `spec.md` (SC-001 through SC-013). These tests are not pass/fail gates in the CI per-PR pipeline; they are nightly regression benchmarks that alert when a metric crosses a threshold.

**How to run** (nightly CI job):
```bash
# Boot time measurement (smoke test variant with timing):
make smoke SMOKE_TIMING=1
# Expected: boot-to-ready < 5 s on x86-64 desktop profile

# WASM cold start latency:
# Measure time from supervisor spawn command to first app stdout line
# Expected: < 5 ms per app (Wasmtime JIT)

# Memory footprint:
# After boot, read /proc/meminfo inside VM and subtract free memory
# Expected: supervisor + runtime + 10 apps < 8 MB on embedded profiles

# FPS measurement (display profile only):
# Count VYOMA_DRAW:flush calls per second under GUI workload
# Expected: >= 30 fps on desktop profile

# Throughput (server profile):
# HTTP server app: measure requests/second at localhost:8080
# Baseline: >= 1000 req/s on server-headless profile
```

**What it catches**:
- Supervisor code changes that add unexpected allocations or blocking operations
- WASM runtime version upgrades that change JIT compilation behavior
- Kernel configuration changes that affect scheduler latency
- Platform profile changes that inadvertently include heavy subsystems on constrained targets

**Thresholds** (from spec.md success criteria):
- SC-004: Boot-to-functional < 5 s on desktop/server, < 1 s on embedded with preloaded image
- SC-006: Supervisor + minimal services < 8 MB RAM on embedded platforms
- SC-009: 200 concurrent WASM apps on desktop/server without resource exhaustion

---

## 3. Per-Platform Testing Guide

### Platform 1: MCU (ARM Cortex-M, `mcu-minimal.toml`)

**Platform profile**: `supervisor/src/profile/profiles/mcu-minimal.toml` (created by T008)
Runtime: wasm3 | RAM: 128 KB | Supervisor modules: lifecycle, capability, ipc, hal, observability | HAL drivers: gpio, i2c, uart

**Build command** (once T057 is complete):
```bash
make build PLATFORM=mcu-arm-cortex-m
# Selects: platforms/mcu-arm-cortex-m/Makefile and kernel.config
# Compiles supervisor for arm-unknown-linux-musleabihf with wasm3 runtime
# Produces: out/mcu-arm-cortex-m/bzImage + initramfs.cpio.gz
```

**QEMU invocation** (MPS2-AN385 board — ARM Cortex-M3, closest to M4 in upstream QEMU):
```bash
qemu-system-arm \
  -machine mps2-an385 \
  -cpu cortex-m3 \
  -kernel out/mcu-arm-cortex-m/bzImage \
  -nographic \
  -append "console=ttyAMA0 quiet" \
  -m 128K \
  -no-reboot
```

**Note**: The MPS2-AN385 board in QEMU has only 4 MB of flash and no network. For testing purposes, the supervisor is loaded as a bare binary directly into RAM using QEMU's `-kernel` argument, not as a full Linux boot. A minimal supervisor stub (without Linux kernel) is the intended form for MCU deployment. The QEMU `mps2-an385` machine is the best available emulation for ARM Cortex-M in CI without physical hardware.

**Run command**:
```bash
make run PLATFORM=mcu-arm-cortex-m
# QEMU command as above, with -nographic
```

**Test command**:
```bash
make smoke PLATFORM=mcu-arm-cortex-m
# Looks for: [lifecycle] all apps spawned on ttyAMA0
# Timeout: 10 s (MCU boots faster than desktop)

make unit-test
# Includes wasm3_adapter.rs and runtime_parity.rs tests (T052, T053)
```

**Layers applicable**: 1 (unit), 2 (check/clippy), 3 (manifests), 4 (smoke), 7 (capability denial)
Layers 5, 6, and 8 are partially applicable: no OTA in Phase 7 scope; parity test is internal (wasm3 vs Wasmtime); no FPS metric.

**Known limitations / caveats**:
- QEMU `mps2-an385` does not emulate virtio, 9P, or DRM. Filesystem and display capabilities are not testable in this environment; they must be skipped in the MCU profile smoke test.
- The wasm3 Rust bindings (`wasm3` crate) must be evaluated for compatibility with `arm-unknown-linux-musleabihf` and musl libc before T054. If incompatible, WAMR may be substituted.
- Memory constraint of 128 KB means the supervisor + wasm3 + one app must fit in that envelope. This is tight; T057 may require stripping unused supervisor modules.
- Linux kernel support for ARM Cortex-M is experimental. The initial MCU target may use a supervisor stub without a Linux kernel base, which is a significant architectural departure from the current design.

**Real hardware checklist** (STM32F4-Discovery or similar Cortex-M4 board):
- [ ] Cross-compile supervisor with `--target thumbv7em-none-eabihf`
- [ ] Flash image via ST-LINK using `openocd` or `probe-rs`
- [ ] Connect UART1 to USB-serial adapter, open at 115200 baud
- [ ] Verify `[lifecycle] all apps spawned` appears on serial within 1 s
- [ ] Run a GPIO blink app: confirm LED toggles at declared pin only
- [ ] Attempt GPIO access from an app without `gpio_pins` declaration; confirm no blink

---

### Platform 2: IoT/Embedded (ARM64 SBC, `iot-edge.toml`)

**Platform profile**: `supervisor/src/profile/profiles/iot-edge.toml` (created by T008)
Runtime: wamr | RAM: 4 MB | Supervisor modules: lifecycle, capability, ipc, hal, observability, net, packages | HAL drivers: gpio, i2c, spi, uart, adc

**Build command** (once T027, T028 are complete):
```bash
make build PLATFORM=iot-rpi
# Selects: platforms/iot-rpi/Makefile and kernel.config
# ARM64 kernel with virtio + 9P support
# Produces: out/iot-rpi/Image + initramfs.cpio.gz
```

**QEMU invocation**:
```bash
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a72 \
  -kernel out/iot-rpi/Image \
  -initrd out/iot-rpi/initramfs.cpio.gz \
  -nographic \
  -append "console=ttyAMA0 quiet" \
  -m 256M \
  -no-reboot \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr
```

**Run command**:
```bash
make run PLATFORM=iot-rpi
```

**Test command**:
```bash
make smoke PLATFORM=iot-rpi
# Looks for: [lifecycle] all apps spawned on ttyAMA0
# Timeout: 30 s

# Cross-arch parity with desktop:
make cross-arch-test PLATFORM_A=desktop-x86 PLATFORM_B=iot-rpi
# Runs compute-bench.wasm on both, diffs stdout

# OTA rollback test:
make integration-test TEST=ota_rollback PLATFORM=iot-rpi
```

**Layers applicable**: 1, 2, 3, 4, 5 (OTA, IPC round-trip), 6 (cross-arch parity vs x86-64), 7, 8 (boot time, memory)

**Known limitations / caveats**:
- QEMU `virt` machine does not emulate real GPIO hardware. The GPIO HAL mock (T033) is used for capability isolation tests; real peripheral behavior requires a Raspberry Pi 4 or similar ARM64 SBC.
- The WAMR runtime for ARM64 Linux requires separate cross-compilation. If the `wamr` Rust bindings are not available at the time of implementation, Wasmtime may be used as a temporary substitute (at the cost of exceeding the 4 MB RAM budget).
- OTA update connectivity testing requires network access from within QEMU. Use `-netdev user,id=net0` + `-device virtio-net-pci,netdev=net0` for connectivity.

**Real hardware checklist** (Raspberry Pi 4 or Orange Pi 5):
- [ ] Build ARM64 image: `make build PLATFORM=iot-rpi`
- [ ] Flash to SD card: `dd if=out/iot-rpi/disk.img of=/dev/sdX bs=4M`
- [ ] Boot and confirm serial console shows `[lifecycle] all apps spawned` within 5 s
- [ ] Connect a GPIO LED to pin 17; run motor-driver app; confirm LED responds
- [ ] Trigger OTA update: `echo "@supervisor: ota-update sensor-reader /data/sensor-reader-v2.wasm" | tee /dev/stdin`
- [ ] Confirm health check runs for 60 s and new version is committed if healthy
- [ ] Push a broken app binary; confirm rollback to previous slot

---

### Platform 3: Robotics (ARM64, `robotics-rt.toml`)

**Platform profile**: `supervisor/src/profile/profiles/robotics-rt.toml` (created by T008)
Runtime: wamr | RAM: 8 MB | Supervisor modules: lifecycle, capability, ipc, hal, observability, net | HAL drivers: gpio, i2c, spi, uart, adc

**Note**: The robotics profile uses the same ARM64 `virt` QEMU machine as IoT but with a larger RAM allocation and all HAL drivers enabled. The key differentiator is the hot-swap and peripheral conflict detection features tested here.

**Build command**:
```bash
make build PLATFORM=robotics-arm64
# Selects: platforms/robotics-arm64/ (created alongside iot-rpi in Phase 4)
# Produces: out/robotics-arm64/Image + initramfs.cpio.gz
```

**QEMU invocation**:
```bash
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a72 \
  -kernel out/robotics-arm64/Image \
  -initrd out/robotics-arm64/initramfs.cpio.gz \
  -nographic \
  -append "console=ttyAMA0 quiet" \
  -m 512M \
  -no-reboot \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr
```

**Run command**:
```bash
make run PLATFORM=robotics-arm64
```

**Test command**:
```bash
make smoke PLATFORM=robotics-arm64
# Looks for: [lifecycle] all apps spawned

# Hot-swap integration test (T031, T040):
make integration-test TEST=hot_swap PLATFORM=robotics-arm64
# Launches motor-driver, sensor-reader, path-planner, lidar-sim concurrently.
# Replaces path-planner while others run.
# Verifies IPC messages still flow between remaining apps.

# Peripheral conflict test (T032):
make unit-test
# supervisor/tests/exclusive_peripheral.rs verifies at unit level
# Integration confirmation:
make integration-test TEST=exclusive_peripheral PLATFORM=robotics-arm64
```

**Layers applicable**: 1, 2, 3, 4, 5 (hot-swap, peripheral conflict), 6 (parity vs IoT ARM64 — same arch, same binary), 7, 8 (hot-swap < 2 s per SC-003)

**Known limitations / caveats**:
- Real-time guarantees (sub-millisecond response to hardware interrupts) are out of scope for this testing phase. The `PREEMPT_RT` kernel patch is not applied to VyomaOS kernel builds. Latency testing against real robotics hardware requires a `PREEMPT_RT` kernel and physical I/O.
- QEMU `virt` machine does not support native DMA for I2C/SPI. The mock HAL (T033–T035) is used exclusively in CI. All HAL driver tests at this stage exercise the supervisor-side capability enforcement and host function dispatch, not actual peripheral hardware transactions.
- The 4-module hot-swap test (T040) is a behavioral regression test, not a timing test. Verifying "zero disruption" means verifying that IPC messages continue to be delivered and no app crashes. Sub-second swap timing measurement is a Layer 8 concern.

**Real hardware checklist** (NVIDIA Jetson Nano or Raspberry Pi 4 with motor/sensor peripherals):
- [ ] Flash robotics-arm64 image to SD card
- [ ] Connect motor driver (PWM GPIO), I2C distance sensor, and UART GPS module
- [ ] Boot and confirm all 4 robotics apps spawn
- [ ] While apps are running, execute: `echo "@supervisor: ota-update path-planner /data/planner-v2.wasm"`
- [ ] Confirm motor continues to receive PWM commands during the 60 s health check window
- [ ] Attempt to spawn a second app declaring the same GPIO pin; confirm rejection with log message

---

### Platform 4: Mobile/Tablet (ARM64, `mobile.toml`)

**Platform profile**: `supervisor/src/profile/profiles/mobile.toml` (created by T008, extended by TM05)
Runtime: wasmtime | RAM: 256 MB | Display: 1080×2340 portrait | Capabilities: display, touch, network, stdio | No mouse

**Build command** (once TM06, TM07 are complete):
```bash
make build PLATFORM=mobile-arm64
# Selects: platforms/mobile-arm64/Makefile and kernel.config
# ARM64 kernel with DRM + virtio-gpu + virtio-touchscreen
# Produces: out/mobile-arm64/Image + initramfs.cpio.gz
```

**QEMU invocation**:
```bash
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a72 \
  -kernel out/mobile-arm64/Image \
  -initrd out/mobile-arm64/initramfs.cpio.gz \
  -append "console=ttyAMA0 console=tty0 panic=1" \
  -device virtio-gpu-pci,xres=1080,yres=2340 \
  -device virtio-keyboard-pci \
  -device virtio-tablet-pci \
  -display sdl,zoom-to-fit=on \
  -serial stdio \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr \
  -m 512M \
  -no-reboot
```

**Note**: `virtio-tablet-pci` provides absolute pointing device input compatible with touchscreen event injection via QEMU monitor. This is the mechanism used to simulate touch events in CI.

**Run command**:
```bash
make run PLATFORM=mobile-arm64
# On macOS: make run-gui PLATFORM=mobile-arm64 DISPLAY_BACKEND=cocoa
```

**Test command**:
```bash
make smoke PLATFORM=mobile-arm64
# Looks for: [lifecycle] all apps spawned on ttyAMA0

# Touch event injection (TM10):
# In a second terminal, connect to QEMU monitor:
echo "mouse_move 540 1000" | nc -U /tmp/qemu-monitor.sock
echo "mouse_button 1"      | nc -U /tmp/qemu-monitor.sock
echo "mouse_button 0"      | nc -U /tmp/qemu-monitor.sock
# Verify in VM serial output: VYOMA_INPUT:touch:tap:540,1000 appears in touch-demo app stdout

# Mobile profile capability test (TM02):
make unit-test
# mobile_profile.rs verifies profile loads with touch=true, mouse=false, 1080x2340 resolution
```

**To enable QEMU monitor socket**, add to the QEMU command:
```bash
-monitor unix:/tmp/qemu-monitor.sock,server,nowait
```

**Layers applicable**: 1, 2, 3, 4, 5 (touch dispatch, profile routing), 7 (touch-without-capability denied, mouse events absent on mobile profile), 8 (boot time)

**Known limitations / caveats**:
- QEMU `virtio-tablet-pci` generates absolute pointer events in QEMU's internal coordinate system (0–32767 range), not pixel coordinates. The supervisor must translate these to screen pixel coordinates before emitting `VYOMA_INPUT:touch:` events. This translation is platform-profile-specific (screen_w, screen_h in `mobile.toml`).
- Multi-touch (pinch-to-zoom) requires `virtio-input` with a multi-touch HID descriptor. This is out of scope for the initial mobile phase. Only tap and swipe are tested.
- Portrait orientation (1080×2340) may not render correctly in QEMU SDL display at full resolution. The `zoom-to-fit=on` flag compensates, but visual regression testing requires checking framebuffer pixel data directly, not just the QEMU window.
- The same `.wasm` binary running on desktop (1440×900, landscape) and mobile (1080×2340, portrait) may produce different VYOMA_DRAW layouts if apps query screen dimensions. The binary portability invariant applies to capability behavior and compute output, not pixel-for-pixel layout identity across screen shapes.

**Real hardware checklist** (Raspberry Pi 4 with 7" DSI touchscreen, or PinePhone Pro):
- [ ] Flash mobile-arm64 image to SD card / eMMC
- [ ] Boot and confirm display initializes at 1080×2340 (or closest supported resolution)
- [ ] Run `touch-demo.wasm`; tap the screen; verify colored circles appear at tap coordinates
- [ ] Swipe vertically; verify canvas scrolls
- [ ] Confirm mouse move events are NOT delivered to `touch-demo` app (mouse=false in mobile profile)
- [ ] Confirm `touch-demo.wasm` binary is byte-identical to the copy that runs on desktop

---

### Platform 5: Desktop (x86-64, `desktop-full.toml`)

**Platform profile**: `supervisor/src/profile/profiles/desktop-full.toml` (created by T008)
Runtime: wasmtime | RAM: 512 MB | All supervisor modules | Display: 1440×900 | Capabilities: all

This is the current primary development platform. The existing build and run infrastructure (`make build`, `make run`, `make run-gui`) targets this profile.

**Build command**:
```bash
make build
# Equivalent to: make build PLATFORM=desktop-x86 (default)
```

**QEMU invocation** (headless):
```bash
qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -append "console=ttyS0 panic=1" \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr \
  -nographic \
  -m 512M \
  -no-reboot
```

**QEMU invocation** (GUI — virtio-gpu):
```bash
qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -append "console=tty0 console=ttyS0 panic=1" \
  -device virtio-vga,xres=1440,yres=900 \
  -device virtio-mouse-pci \
  -display sdl,zoom-to-fit=on,full-screen=on \
  -serial stdio \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr \
  -m 512M \
  -no-reboot
```

**QEMU invocation** (with networking):
```bash
qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -append "console=ttyS0 panic=1" \
  -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
  -device virtio-net-pci,netdev=net0 \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr \
  -nographic \
  -m 512M \
  -no-reboot
```

**Run commands**:
```bash
make run             # headless
make run-gui         # SDL display (Linux)
make run-gui DISPLAY_BACKEND=cocoa  # macOS display
make run-net         # headless + networking
make run-gui-net     # GUI + networking
```

**Test command**:
```bash
make test            # Full suite: make build + make unit-test + make smoke
make unit-test       # Supervisor cargo tests only
make smoke           # Headless boot, 30 s timeout
make check-manifests # Validate all app manifests
```

**Layers applicable**: All 8 layers. This is the reference platform for all testing layers.

**Known limitations / caveats**:
- `make smoke` runs inside Docker, which means QEMU inside Docker. On macOS, this requires Linux Docker (via Docker Desktop). KVM acceleration is not available inside Docker on macOS; the emulated VM is slower.
- GUI testing (`make run-gui`) requires a display server and is not available in headless CI. Visual regression testing must rely on framebuffer pixel comparisons via QEMU snapshot commands rather than visual inspection.
- The `DISPLAY_BACKEND=cocoa` option requires QEMU built with Cocoa support (standard in Homebrew QEMU on macOS). The Docker-based smoke test always uses `-nographic`.

**Real hardware checklist** (x86-64 laptop or desktop):
- [ ] `make build` on host machine (requires Docker)
- [ ] `make run-gui DISPLAY_BACKEND=sdl` (Linux) or `make run-gui DISPLAY_BACKEND=cocoa` (macOS)
- [ ] Confirm all 10 apps appear in the menu bar
- [ ] Alt+Tab cycles through windowed apps
- [ ] Click on app name in menu bar raises that window
- [ ] Type in shell window; confirm interactive echo
- [ ] `curl localhost:8080` from host; confirm HTTP server app responds
- [ ] `@supervisor: ota-update <app> <path>` replaces a running app without reboot

---

### Platform 6: Server (ARM64, `server-headless.toml`)

**Platform profile**: `supervisor/src/profile/profiles/server-headless.toml` (created by T008)
Runtime: wasmtime | RAM: 1 GB | No display | Supervisor modules: lifecycle, capability, ipc, net, packages, observability | No HAL drivers

**Build command** (once T042, T043 are complete):
```bash
make build PLATFORM=server-arm64
# Selects: platforms/server-arm64/Makefile and kernel.config
# Headless, network-enabled ARM64 kernel
# Produces: out/server-arm64/Image + initramfs.cpio.gz
```

**QEMU invocation**:
```bash
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a72 \
  -kernel out/server-arm64/Image \
  -initrd out/server-arm64/initramfs.cpio.gz \
  -nographic \
  -append "console=ttyAMA0 quiet" \
  -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
  -device virtio-net-pci,netdev=net0 \
  -virtfs local,path=data,mount_tag=vyoma-data,security_model=mapped-xattr \
  -m 1G \
  -no-reboot
```

**Run command**:
```bash
make run PLATFORM=server-arm64
```

**Test command**:
```bash
make smoke PLATFORM=server-arm64
# Looks for: [lifecycle] all apps spawned on ttyAMA0

# Cross-arch parity with desktop x86-64:
# Run compute-bench.wasm on both; diff stdout (Layer 6)

# Profile behavior test (T041, T045):
# Deploy doc-viewer.wasm to server-arm64 profile; confirm HTTP mode activates
curl http://localhost:8080/ | grep "doc-viewer"

# Capability enforcement on server profile (T046):
# App with network=false in vyoma.toml cannot open sockets even on server profile
```

**Layers applicable**: 1, 2, 3, 4, 5 (IPC, OTA, profile behavior), 6 (cross-arch parity with x86-64), 7, 8 (startup time, concurrent apps)

**Known limitations / caveats**:
- QEMU `virt` machine ARM64 requires `qemu-system-aarch64` to be installed. On Ubuntu CI runners, install via: `sudo apt-get install -y qemu-system-aarch64`
- The ARM64 server profile uses the same Wasmtime binary as desktop. Wasmtime builds for ARM64 Linux are available as pre-compiled releases from the Bytecodealliance; no cross-compilation of Wasmtime itself is needed.
- Cross-arch parity testing between server-arm64 and desktop-x86 is the primary validation for Invariant B on server workloads. The `compute-bench.wasm` (SHA-256 hash chain, T050) is the reference deterministic workload.
- `qemu-system-aarch64` emulates ARM64 in software on x86-64 CI runners, which is significantly slower than native or KVM-accelerated execution. Allow 3–5× the desktop smoke test timeout for ARM64 smoke tests.

**Real hardware checklist** (AWS Graviton 3, Ampere Altra, or ARM64 server):
- [ ] Flash or deploy image to ARM64 host (bare metal or VM)
- [ ] Confirm `[lifecycle] all apps spawned` within 2 s of systemd starting the supervisor
- [ ] Deploy `doc-viewer.wasm` with server profile; confirm HTTP endpoint responds
- [ ] Deploy same `doc-viewer.wasm` binary from desktop build; confirm same HTTP behavior
- [ ] Test OTA update: push new wasm, verify health check, verify commit or rollback
- [ ] Measure: 200 concurrent WASM apps launch without OOM (SC-009)

---

### Platform 7: HPC (x86-64 + ARM64)

**Platform profile**: HPC uses the `server-headless.toml` profile on both x86-64 and ARM64 nodes. The distinguishing feature is wasm64 support and byte-identical output verification across architectures.

**Context**: wasm64 (`wasm64-wasip2`) toolchain support in Rust/LLVM must be verified before implementing T048–T050. As of 2026-05-25, the wasm64 target is experimental in LLVM. If the toolchain is unavailable, T050 uses wasm32 for the compute benchmark and T047/T048/T049 use mock wasm64 validation logic. See `specs/043-universal-modular-os/targets/hpc/tasks.md`, Notes section.

**Build command**:
```bash
# x86-64 node:
make build PLATFORM=desktop-x86
# (or server-arm64 if using headless server profile)

# ARM64 node:
make build PLATFORM=server-arm64

# Build compute benchmark (wasm64 or wasm32 proxy):
cargo build --manifest-path apps/compute-bench/Cargo.toml \
  --target wasm64-wasip2 --release
# If wasm64 unavailable, use: --target wasm32-wasip2
```

**QEMU invocation — x86-64 node**:
```bash
qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -nographic \
  -append "console=ttyS0 quiet" \
  -m 512M \
  -no-reboot
```

**QEMU invocation — ARM64 node** (run in parallel with x86-64):
```bash
qemu-system-aarch64 \
  -machine virt \
  -cpu cortex-a72 \
  -kernel out/server-arm64/Image \
  -initrd out/server-arm64/initramfs.cpio.gz \
  -nographic \
  -append "console=ttyAMA0 quiet" \
  -m 512M \
  -no-reboot
```

**Run command** (both nodes in parallel):
```bash
make run-hpc-parity
# Runs x86-64 and ARM64 QEMU instances in background
# Captures stdout from compute-bench on both
# Diffs the output lines prefixed with "RESULT:"
# Prints: PARITY: PASS or PARITY: FAIL
```

**Test command**:
```bash
make hpc-parity-test
# Equivalent to Layer 6 cross-arch parity test automation
# Invokes: make run + output capture + diff
# Timeout: 120 s (ARM64 QEMU is slower)

# wasm64 loading test (T047):
make unit-test
# supervisor/tests/wasm64_loading.rs tests runtime accepts wasm64 binary
# supervisor/tests/runtime_parity.rs tests wasm3 vs Wasmtime equivalence

# wasm32 rejection test (T048 inverse):
# Verify wasm32 runtime adapter returns clear error for wasm64 binary:
# This is covered by wasm64_loading.rs test case "wasm32_rejects_wasm64"
```

**Layers applicable**: 1, 2, 3, 4 (smoke on each arch), 5 (none — no OTA or hot-swap specific to HPC), 6 (cross-arch parity is the primary HPC test), 7, 8 (identical resource usage across architectures per SC-007)

**Known limitations / caveats**:
- wasm64 toolchain (`wasm64-wasip2` Rust target): as of 2026-05-25, this target is not in stable Rust. Monitor `https://github.com/rust-lang/rust/issues/` for wasm64 target tracking. Until available, the compute-bench app uses wasm32 and the HPC parity test validates architecture-independence at the wasm32 level.
- Running two QEMU instances simultaneously in CI requires a runner with sufficient memory (at least 2 GB). Use matrix strategy in GitHub Actions with separate jobs rather than running both instances in the same job.
- HPC in the real sense (thousands of nodes) cannot be simulated in CI. The parity test validates the invariant on 2 architectures; the SC-007 acceptance criterion of "3+ architectures" requires adding a RISC-V QEMU instance when `qemu-system-riscv64` is confirmed stable enough.

**Real hardware checklist** (HPC cluster or cloud multi-arch setup):
- [ ] Deploy same `compute-bench.wasm` to x86-64 and ARM64 nodes
- [ ] Run both simultaneously: `ssh x86-node 'vyoma run compute-bench' > x86.out &; ssh arm64-node 'vyoma run compute-bench' > arm64.out &; wait`
- [ ] `diff x86.out arm64.out` produces no output (byte-identical)
- [ ] Confirm resource usage bounded: `compute-bench` does not exceed declared capabilities
- [ ] Test rejection: submit `compute-bench.wasm` (wasm64) to a wasm32-only node; confirm clear error message, not hang or silent failure

---

## 4. CI/CD Pipeline Design

### When to Run Each Layer

| Layer | PR commit | PR merge to develop | Nightly |
|-------|-----------|---------------------|---------|
| 1. Unit tests | Always | Always | Always |
| 2. Cargo check/clippy | Always | Always | Always |
| 3. Manifest validation | Always | Always | Always |
| 4. Desktop smoke test | Always | Always | Always |
| 4. Per-platform smoke tests | On file changes in platforms/ | Always | Always |
| 5. Integration tests | On changes to supervisor/src/ | Always | Always |
| 6. Cross-arch parity | On changes to apps/ or runtime/ | Always | Always |
| 7. Capability denial | Always (runs as part of unit tests) | Always | Always |
| 8. Performance baseline | Nightly only | Nightly | Always |

**Rationale**: Layer 4 per-platform smoke tests are expensive (QEMU emulation for non-native architectures can take 3–5 minutes per platform). Running all 7 platforms on every PR commit would make CI impractically slow. Filters on relevant file paths keep the inner loop fast.

### GitHub Actions YAML Skeleton

```yaml
# .github/workflows/ci.yml
name: VyomaOS CI

on:
  push:
    branches: [main, develop, 'feat/**', 'refactor/**']
  pull_request:
    branches: [main, develop]

env:
  RUSTFLAGS: "-D warnings"

jobs:
  # ── Layer 1+2+3: fast gates (no QEMU) ─────────────────────────────────────
  unit-check:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4

      - name: Install QEMU (for smoke test in Docker)
        run: sudo apt-get update -qq && sudo apt-get install -y qemu-system-x86

      - name: Set up Docker Buildx
        uses: docker/setup-buildx-action@v3

      - name: Build Docker builder image
        run: make image

      - name: Cargo check (supervisor, no QEMU)
        run: |
          docker run --rm -v "$PWD":/work -w /work \
            -u $(id -u):$(id -g) \
            vyomaos-builder:latest \
            env RUSTFLAGS="-D warnings" cargo check \
              --manifest-path supervisor/Cargo.toml \
              --target x86_64-unknown-linux-musl

      - name: Cargo clippy
        run: |
          docker run --rm -v "$PWD":/work -w /work \
            -u $(id -u):$(id -g) \
            vyomaos-builder:latest \
            env RUSTFLAGS="-D warnings" cargo clippy \
              --manifest-path supervisor/Cargo.toml \
              --target x86_64-unknown-linux-musl \
              -- -D clippy::all

      - name: 500-line file limit check
        run: |
          fail=0
          while IFS= read -r f; do
            lines=$(wc -l < "$f")
            if [ "$lines" -gt 500 ]; then
              echo "FAIL: $f has $lines lines (limit: 500)"
              fail=1
            fi
          done < <(find supervisor/src apps -name '*.rs' 2>/dev/null)
          exit $fail

      - name: Unit tests (Layer 1)
        run: make unit-test

      - name: Manifest validation (Layer 3)
        run: make check-manifests

  # ── Layer 4: Desktop smoke test ────────────────────────────────────────────
  smoke-desktop:
    runs-on: ubuntu-latest
    timeout-minutes: 8
    needs: unit-check
    steps:
      - uses: actions/checkout@v4

      - name: Install QEMU
        run: sudo apt-get update -qq && sudo apt-get install -y qemu-system-x86

      - name: Build Docker builder image
        run: make image

      - name: Full build (kernel + supervisor + apps + rootfs + disk)
        run: make build

      - name: Smoke test (Layer 4)
        run: make smoke

      - name: Upload build artifacts
        uses: actions/upload-artifact@v4
        if: always()
        with:
          name: desktop-build-${{ github.sha }}
          path: |
            out/bzImage
            out/initramfs.cpio.gz
          retention-days: 7

  # ── Layer 4: Per-platform smoke tests (matrix, path-filtered) ─────────────
  smoke-platforms:
    runs-on: ubuntu-latest
    timeout-minutes: 15
    needs: unit-check
    # Only run when platform configs or runtime source changes
    if: |
      github.event_name == 'push' && (
        contains(github.event.commits[0].modified, 'platforms/') ||
        contains(github.event.commits[0].modified, 'supervisor/src/runtime/') ||
        contains(github.event.commits[0].modified, 'supervisor/src/profile/')
      )
    strategy:
      fail-fast: false
      matrix:
        platform:
          - name: iot-rpi
            qemu_pkg: qemu-system-arm
            arch: aarch64
            timeout: 60
          - name: server-arm64
            qemu_pkg: qemu-system-arm
            arch: aarch64
            timeout: 90
          - name: robotics-arm64
            qemu_pkg: qemu-system-arm
            arch: aarch64
            timeout: 60
    steps:
      - uses: actions/checkout@v4

      - name: Install QEMU for ${{ matrix.platform.arch }}
        run: |
          sudo apt-get update -qq
          sudo apt-get install -y ${{ matrix.platform.qemu_pkg }} qemu-user-static

      - name: Build Docker image
        run: make image

      - name: Build platform ${{ matrix.platform.name }}
        run: make build PLATFORM=${{ matrix.platform.name }}
        timeout-minutes: 10

      - name: Smoke test ${{ matrix.platform.name }}
        run: make smoke PLATFORM=${{ matrix.platform.name }}
        timeout-minutes: 3

  # ── Layer 6: Cross-arch parity test ────────────────────────────────────────
  cross-arch-parity:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    needs: [smoke-desktop]
    # Run nightly or on changes to runtime / apps
    if: |
      github.event_name == 'schedule' ||
      contains(github.event.commits[0].modified, 'supervisor/src/runtime/') ||
      contains(github.event.commits[0].modified, 'apps/compute-bench/')
    steps:
      - uses: actions/checkout@v4

      - name: Install QEMU (x86 and ARM)
        run: |
          sudo apt-get update -qq
          sudo apt-get install -y qemu-system-x86 qemu-system-arm qemu-user-static

      - name: Build Docker image
        run: make image

      - name: Build desktop (x86-64) + compute-bench
        run: make build

      - name: Build ARM64 server image
        run: make build PLATFORM=server-arm64

      - name: Cross-arch parity test (Layer 6)
        run: make cross-arch-test PLATFORM_A=desktop-x86 PLATFORM_B=server-arm64
        timeout-minutes: 10

  # ── Layer 5: Integration tests ─────────────────────────────────────────────
  integration:
    runs-on: ubuntu-latest
    timeout-minutes: 20
    needs: [smoke-desktop]
    if: |
      github.event_name == 'schedule' ||
      contains(github.event.commits[0].modified, 'supervisor/src/')
    steps:
      - uses: actions/checkout@v4

      - name: Install QEMU
        run: sudo apt-get update -qq && sudo apt-get install -y qemu-system-x86

      - name: Build Docker image
        run: make image

      - name: Build
        run: make build

      - name: Integration tests (IPC, OTA, hot-swap)
        run: make integration-test
        timeout-minutes: 15

  # ── Layer 8: Performance baseline (nightly only) ───────────────────────────
  performance:
    runs-on: ubuntu-latest
    timeout-minutes: 30
    needs: [smoke-desktop]
    if: github.event_name == 'schedule'
    steps:
      - uses: actions/checkout@v4

      - name: Install QEMU
        run: sudo apt-get update -qq && sudo apt-get install -y qemu-system-x86

      - name: Build Docker image
        run: make image

      - name: Full build
        run: make build

      - name: Performance baseline
        run: make perf-baseline
        timeout-minutes: 20

      - name: Upload performance report
        uses: actions/upload-artifact@v4
        with:
          name: perf-report-${{ github.sha }}
          path: out/perf-report.json
          retention-days: 30

# ── Nightly schedule ──────────────────────────────────────────────────────────
  # Triggered by a separate workflow file (.github/workflows/nightly.yml):
  # schedule:
  #   - cron: '0 2 * * *'   # 02:00 UTC daily
```

### QEMU User-Mode Emulation for Faster Cross-Arch Unit Tests

For unit tests that must run on a non-native architecture (e.g., running the ARM64 supervisor unit tests on an x86-64 CI runner), QEMU user-mode emulation is faster than full system emulation:

```bash
# Install QEMU user-mode emulators and binfmt support:
sudo apt-get install -y qemu-user-static binfmt-support
sudo update-binfmts --enable

# Cross-compile supervisor for ARM64:
docker run --rm --platform linux/arm64 \
  -v "$PWD":/work -w /work \
  --entrypoint cargo \
  vyomaos-builder:latest \
  test --manifest-path supervisor/Cargo.toml \
  --target aarch64-unknown-linux-musl

# The ARM64 binary runs transparently via QEMU user-mode on the x86-64 runner.
# This is ~3× faster than full system QEMU for unit tests.
# Limitation: does not test kernel, display, or virtio devices.
```

### Artifact Retention

| Artifact | Condition | Retention |
|----------|-----------|-----------|
| Desktop build (`bzImage`, `initramfs.cpio.gz`) | Every push to develop/main | 7 days |
| Platform-specific builds | Per-platform smoke jobs | 7 days |
| Test logs (`/tmp/smoke-*.log`) | On smoke failure | 30 days |
| Performance report JSON | Nightly performance job | 30 days |
| Coverage report (when added) | Nightly | 14 days |

### Timeout Budgets per Job

| Job | Budget |
|-----|--------|
| `unit-check` (cargo check + clippy + unit tests + manifests) | 10 min |
| `smoke-desktop` (build + smoke) | 8 min |
| `smoke-platforms` per platform (build + smoke) | 15 min |
| `cross-arch-parity` | 20 min |
| `integration` | 20 min |
| `performance` (nightly) | 30 min |

### wasm3 Feature Flag in CI

The wasm3 runtime integration (Phase 7) requires a Cargo feature flag to conditionally compile the wasm3 adapter:

```bash
# Without wasm3 (default — uses Wasmtime only):
cargo test --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl

# With wasm3 enabled (after T054):
cargo test --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl \
  --features wasm3

# CI matrix approach — test both configurations:
strategy:
  matrix:
    features: ["", "--features wasm3"]
```

Until the `wasm3` crate is added as an optional dependency (T054), the feature flag does not exist and only the default (Wasmtime-only) build is tested. The `wasm3_adapter.rs` test file will be gated with `#[cfg(feature = "wasm3")]` to avoid compilation errors in the default configuration.

---

## 5. Test Data and Fixtures

### WASM Test Binaries Needed

The following test WASM binaries are needed across all platforms. They should be built from source in `apps/` and included in the relevant platform rootfs images.

| Binary | Purpose | Test layers |
|--------|---------|-------------|
| `compute-bench.wasm` | Deterministic SHA-256 hash chain for cross-arch parity. Produces `RESULT:<hex-hash>` lines. No non-deterministic WASI calls. | 6, 8 |
| `capability-probe.wasm` | Attempts each capability (network, filesystem, GPIO, I2C, touch) and reports which calls succeed vs fail. Used to verify denial. | 7 |
| `ota-canary.wasm` | Reports its version string on stdout, then runs a simple heartbeat loop. Used as the "healthy" module in OTA rollback tests. | 5 |
| `ota-crasher.wasm` | Exits with code 1 after 5 seconds. Used as the "unhealthy" module that triggers OTA rollback. | 5 |
| `touch-demo.wasm` | Renders tap circles on the framebuffer at touch event coordinates. | 4 (mobile smoke), 5 |
| `motor-driver.wasm`, `sensor-reader.wasm`, `path-planner.wasm`, `lidar-sim.wasm` | Robotics hot-swap test modules. Each sends IPC heartbeats and logs its name. | 5 |
| `doc-viewer.wasm` | Dual-mode app: VYOMA_DRAW on desktop profile, HTTP on server profile. | 5 |

All test binaries must be deterministic (given no wall-clock input) and minimal (< 50 KB compiled). They must NOT depend on third-party crates that introduce non-determinism (timestamps, randomness, thread IDs).

### Generating Deterministic Test Fixtures for Cross-Arch Parity

The cross-arch parity test requires a reference output that is known-correct. Procedure:

1. Build `compute-bench.wasm` from source on the desktop platform.
2. Run it on the x86-64 QEMU instance and capture stdout to `fixtures/compute-bench-x86.expected`.
3. Commit this file to the repository as the canonical reference output.
4. In CI, run the same binary on each target architecture and diff against `fixtures/compute-bench-x86.expected`. Any deviation is a test failure.

The fixture must be regenerated whenever the `compute-bench` app source changes. The CI pipeline detects fixture staleness by hashing the app source and comparing against the hash embedded in the fixture file header:

```
# compute-bench-x86.expected
# source-hash: sha256:abc123...
# generated: 2026-05-25
RESULT:8f14e45fceea167a5a36dedd4bea2543
RESULT:23e9c471ea78d57b...
```

### Simulating OTA Update Scenarios in Tests

OTA update tests use a local file-based delivery mechanism rather than a real network:

```bash
# In the running VM (via QEMU serial console or IPC):
# Step 1: Stage new version in /data
cp /apps/ota-canary/ota-canary.wasm /data/ota-canary-v2.wasm

# Step 2: Trigger OTA via IPC
echo "@supervisor: ota-update ota-canary /data/ota-canary-v2.wasm"

# Expected supervisor sequence:
# 1. Copy new wasm to slot B
# 2. Hash-verify the new binary
# 3. Spawn new version in slot B
# 4. Monitor for 60 s (health_check_secs from profile)
# 5. If healthy: commit slot B as active, mark slot A as backup
# 6. If unhealthy: terminate slot B, restore slot A
```

For unit tests (T021), the A/B slot manager is tested with mock binaries: `ota_rollback.rs` creates two temporary `.wasm` files, invokes `AbSlot::deploy()` with the new binary, advances simulated time past the health check window, and verifies the slot state machine transitions correctly.

### Simulating Touch Events via QEMU Monitor

Touch events are injected in QEMU using the `virtio-tablet-pci` device and the QEMU Human Monitor Protocol (HMP):

```bash
# Launch QEMU with monitor socket:
qemu-system-aarch64 ... \
  -monitor unix:/tmp/qemu-mon.sock,server,nowait

# In a separate terminal, inject a tap at screen coordinate (540, 1000):
# QEMU tablet reports coordinates in 0–32767 range; scale to screen pixels.
# For 1080×2340 screen: x_raw = 540 * 32767 / 1080 ≈ 16384, y_raw = 1000 * 32767 / 2340 ≈ 14003

printf "mouse_move 16384 14003\nmouse_button 1\nmouse_button 0\n" \
  | nc -q1 -U /tmp/qemu-mon.sock

# Expected: supervisor reads virtio-input event, translates to pixel coords,
# emits VYOMA_INPUT:touch:tap:540,1000 to focused app stdin.
```

For automated CI testing without a QEMU monitor, inject touch events by writing them directly to the supervisor's mock input pipe when running integration tests with a test harness rather than a full QEMU boot.

### Simulating Peripheral Conflicts in Unit Tests (PeripheralRegistry Mock)

The `PeripheralRegistry` tracks which pins/buses/ports are claimed by which modules. Unit tests for peripheral conflict detection (T032, `exclusive_peripheral.rs`) use an in-memory mock:

```rust
// In supervisor/tests/exclusive_peripheral.rs:
let mut registry = PeripheralRegistry::new_mock();

// First app claims GPIO pin 5 as output: succeeds
let result1 = registry.claim_gpio(5, Direction::Output, "motor-driver");
assert!(result1.is_ok());

// Second app also tries to claim GPIO pin 5 as output: rejected
let result2 = registry.claim_gpio(5, Direction::Output, "sensor-reader");
assert!(result2.is_err());
assert!(result2.unwrap_err().contains("already claimed by motor-driver"));

// Different app claims GPIO pin 5 as input: also rejected (exclusive access)
let result3 = registry.claim_gpio(5, Direction::Input, "lidar-sim");
assert!(result3.is_err());
```

The `PeripheralRegistry::new_mock()` constructor creates a registry with no actual hardware backing. All `claim_*` calls operate on an in-memory `HashMap<(PeripheralType, u8), &str>` and return errors if the requested resource is already claimed. This mock is test-only and is compiled with `#[cfg(test)]`.

---

## 6. Acceptance Test Checklists

### US1 (IoT): Secure Edge Firmware Deployment

**User Story**: An IoT manufacturer deploys sandboxed WASM firmware to a fleet of heterogeneous devices using a single binary format with OTA updates.

**Checklist**:

- [ ] **Cross-arch binary parity**: Build `ota-canary.wasm` once. Deploy to QEMU x86-64 and QEMU ARM64 instances. Both produce identical stdout (Layer 6 parity test passes).
- [ ] **Capability denial — network**: Create a manifest with `network = false`. Attempt to open a TCP connection from the app. Verify supervisor logs `[capability] network denied for <app>` and no connection is established.
- [ ] **Capability denial — filesystem**: Create a manifest with `filesystem = false`. Attempt to open `/data/test.txt`. Verify WASI returns an access error (not a kernel panic or silent success).
- [ ] **Capability denial — GPIO**: Create a manifest without `gpio_pins`. Attempt to call the GPIO host function. Verify WASM trap is raised and app exits with non-zero code.
- [ ] **OTA update — happy path**: Deploy `ota-canary-v2.wasm` via `@supervisor: ota-update`. Verify new version is running within 2 s (SC-003). Other apps continue without disruption.
- [ ] **OTA rollback**: Deploy `ota-crasher.wasm` (exits after 5 s). Verify health check detects failure within 60 s (SC-011). Verify system automatically reverts to previous slot with no manual intervention.
- [ ] **OTA mid-transfer loss** (manual test only): Interrupt OTA network transfer at 50%. Verify device remains on previous working version. Slot A must be unmodified.
- [ ] **Boot time** (Layer 8): Time boot-to-ready on embedded QEMU. Must be < 1 s with preloaded image (SC-004). Note: this target requires pre-staged initramfs on flash storage, not network-loaded kernel.

---

### US2 (Robotics): Modular Sensor/Actuator Stack

**User Story**: A robotics engineer hot-swaps individual WASM modules without rebooting the system or disrupting peer modules.

**Checklist**:

- [ ] **4-module concurrent startup**: Launch `motor-driver`, `sensor-reader`, `path-planner`, `lidar-sim`. All 4 appear in `ps` output within 5 s. IPC heartbeats visible in logs.
- [ ] **Hot-swap planner** (SC-003): Issue `@supervisor: ota-update path-planner /data/planner-v2.wasm`. Within 2 s, `ps` shows new planner version. `motor-driver` and `sensor-reader` continue sending/receiving IPC messages without gap.
- [ ] **Peripheral isolation — GPIO vs no-GPIO**: `motor-driver` (gpio=true) successfully calls GPIO host function. `sensor-reader` (gpio=false) receives WASM trap when attempting GPIO call.
- [ ] **Exclusive peripheral conflict**: Two apps both declare `gpio_pins = [5]`. Second app spawn is rejected at startup with a logged error: `[capability] GPIO pin 5 already claimed by motor-driver`. First app continues normally.
- [ ] **Module independence**: Kill `path-planner` via `@supervisor: kill path-planner`. Verify `motor-driver`, `sensor-reader`, and `lidar-sim` continue operating. Confirm supervisor does not trigger cascading restarts.
- [ ] **HAL function isolation** (Layer 7): `sensor-reader` (i2c=true) successfully reads from I2C bus 1. `path-planner` (i2c=false) receives access denied when attempting I2C call. Verified by examining supervisor capability log.

---

### US3 (Enterprise): Cross-Platform Desktop + Server Apps

**User Story**: An enterprise team deploys the same productivity app binary to desktop (VYOMA_DRAW display) and server (HTTP headless) using a single build.

**Checklist**:

- [ ] **Desktop rendering**: Deploy `doc-viewer.wasm` to `desktop-full` profile. Verify `VYOMA_DRAW:draw_text` commands appear in supervisor display output. App title visible in menu bar.
- [ ] **Server HTTP mode**: Deploy same `doc-viewer.wasm` binary (no recompilation) to `server-headless` profile. Verify `curl http://localhost:8080/` returns document content. No VYOMA_DRAW calls emitted.
- [ ] **Binary identity**: SHA-256 hash of `doc-viewer.wasm` is identical on desktop and server deployments. Confirm with: `sha256sum /apps/doc-viewer/doc-viewer.wasm` on both VMs.
- [ ] **Capability enforcement on server** (network=false): Create doc-viewer copy with `network = false` in manifest. Deploy to server profile. Verify app cannot open sockets even though the server profile has network drivers available.
- [ ] **Capability enforcement on desktop** (filesystem=false): Deploy doc-viewer with `filesystem = false`. Verify it cannot read `/data/` even on a desktop with a mounted data disk.
- [ ] **Profile TOML validation**: Run `make check-manifests` after adding `doc-viewer/vyoma.toml`. Confirm OK with no warnings.
- [ ] **`make build PLATFORM=server-arm64`**: Produces a bootable ARM64 image. QEMU ARM64 smoke test passes.

---

### Mobile: Touch Events and Profile Correctness

**User Story**: A tablet deployment runs touch-aware WASM apps with correct event routing and no spurious mouse events.

**Checklist**:

- [ ] **Touch event delivery**: Inject `VYOMA_INPUT:touch:tap:540,1000` to supervisor (via QEMU monitor or mock pipe). Verify `touch-demo.wasm` (touch=true) receives event and renders a circle at (540,1000).
- [ ] **Touch denial** (touch=false): Create a test app with `touch = false`. Inject touch event. Verify the app does NOT receive the event in its stdin.
- [ ] **Mouse absence on mobile profile**: Confirm `mobile.toml` has `mouse = false`. Verify no `VYOMA_INPUT:mouse:` events are generated or routed in the mobile supervisor instance.
- [ ] **Profile loads correctly**: `mobile_profile.rs` unit test (TM02) passes: display=true, touch=true, network=true, mouse=false, screen_w=1080, screen_h=2340, orientation=portrait.
- [ ] **Binary portability**: `touch-demo.wasm` SHA-256 matches on desktop build and mobile deployment. No recompilation. Only capability wiring differs.
- [ ] **`make build PLATFORM=mobile-arm64`**: Produces bootable ARM64 image with virtio-gpu at 1080×2340 and virtio-touchscreen device.
- [ ] **Swipe event**: Inject `VYOMA_INPUT:touch:swipe:0,50` (50 px downward swipe). Verify `touch-demo.wasm` scrolls canvas by 50 pixels.

---

### US4 (HPC): Reproducible Compute Jobs

**User Story**: A research computing operator runs deterministic, byte-identical compute jobs across thousands of nodes of different architectures.

**Checklist**:

- [ ] **wasm64 binary accepted** (T047): `wasm64_loading.rs` unit test passes: Wasmtime adapter accepts a wasm64 binary without error on platforms where `wasm64_enabled = true` in profile.
- [ ] **wasm32 rejects wasm64** (T048 inverse): Wasmtime adapter configured without 64-bit memory returns a clear `Err("wasm64 not supported on this runtime configuration")` when presented with a wasm64 binary. No hang, no silent corruption.
- [ ] **Cross-arch byte-identical output** (T051): `compute-bench.wasm` run on x86-64 and ARM64 QEMU instances produces identical `RESULT:` lines. `diff` of captured outputs produces no output.
- [ ] **Deterministic across 3 architectures** (SC-007): Add RISC-V64 QEMU instance (`qemu-system-riscv64`, machine=virt, cpu=rv64) to parity test. All three outputs are byte-identical. Note: RISC-V QEMU stability must be verified before enabling in CI.
- [ ] **Resource bounds**: `compute-bench.wasm` declared capabilities are stdio only (`network=false`, `filesystem=false`). Confirm no capability escalation occurs during the hash chain computation.
- [ ] **wasm64 toolchain gap acknowledgment**: If `wasm64-wasip2` Rust target is unavailable, document the gap in the test report. Use wasm32 for all cross-arch parity tests until the target is stabilized in Rust nightly. File a tracking issue referencing the Rust RFC or issue number for wasm64 support.

---

## 7. Regression Test Policy

### 500-Line File Limit

Every `.rs` file in `supervisor/src/` and `apps/*/src/` must remain under 500 lines. This limit is enforced in CI by a shell check in the `unit-check` job (see Section 4). When a file approaches the limit, it must be split into focused submodules before any new code is added.

CI enforcement script (runs on every PR):
```bash
fail=0
while IFS= read -r f; do
  lines=$(wc -l < "$f")
  if [ "$lines" -gt 500 ]; then
    echo "FAIL: $f has $lines lines (limit: 500)"
    fail=1
  fi
done < <(find supervisor/src apps -name '*.rs' 2>/dev/null)
exit $fail
```

Exclusions: `supervisor/tests/` files, the `.tessl/` plugin cache, generated code.

### Zero-Warning Policy

`RUSTFLAGS=-D warnings` is set for all `make unit-test` invocations and all `cargo build` commands in the Makefile. A single compiler warning is a build failure. This is non-negotiable. Suppressing a warning with `#[allow(...)]` requires a code comment explaining why it is safe to suppress for this specific case.

### Regression Test Policy for Bug Fixes

Every bug fix must include a regression test committed in the same PR that:
1. Fails on the state of the code before the fix (demonstrating the test catches the bug).
2. Passes after the fix.
3. Is named with a `test_regression_<description>` prefix or includes a comment linking to the issue.

This applies to all layers: if a bug is found in production that manifests as a smoke test failure, the regression test may be an integration test. If it manifests as a logic error in the supervisor, it must be a unit test.

### New Capability Type Policy

Any new capability type added to the supervisor must have:
1. A unit test in `manifest_tests.rs` confirming the field parses correctly.
2. A unit test in `manifest_tests.rs` confirming that an unknown field adjacent to the new field still returns an error.
3. A capability denial test confirming that an app without the new capability cannot access the corresponding resource.
4. A documentation update in `docs/vyoma-manifest-schema.md`.

This policy is enforced by code review, not by automated CI tooling. The PR author is responsible for including all four items.

### Cross-Platform Invariant Maintenance

When a new platform is added:
1. A smoke test for that platform must pass in CI before the platform support is merged to `develop`.
2. The per-platform testing guide in this document (Section 3) must be updated with the exact QEMU invocation and test commands.
3. The cross-arch parity fixture set must be extended to include the new architecture if it differs from existing tested architectures.

### Known Gaps and Future Work

- **wasm64 toolchain**: Not yet available in stable Rust. All wasm64 tests use mock validation or wasm32 as a proxy. Track at the Rust issue tracker. When the target becomes available, T047–T051 must be unblocked and the proxy tests replaced.
- **Real-time kernel testing**: The `PREEMPT_RT` patch is not tested. Real-time latency guarantees for robotics require a separate kernel build and hardware-in-the-loop test bench that is out of scope for CI.
- **wasm3 crate ARM32 support**: The `wasm3` Rust crate's compatibility with `arm-unknown-linux-musleabihf` (ARM32 musl) is unverified. This affects MCU platform testing. Evaluate before T054; if incompatible, switch to `wamr` for the MCU target.
- **Multi-touch on mobile**: Pinch-to-zoom and two-finger gestures are out of scope for the initial mobile testing phase. They will require QEMU multitouch support or a hardware-in-the-loop test.
- **RISC-V architecture**: SC-007 and FR-013 reference RISC-V support. No QEMU RISC-V smoke test exists yet. Add once the `riscv32`/`riscv64` cross-compilation toolchain is integrated into the Docker builder image.
- **Coverage reporting**: No code coverage tooling is currently configured. `cargo-tarpaulin` or `cargo-llvm-cov` should be added as a nightly CI step to track coverage trends over time. Target: > 70% line coverage for supervisor modules.

---

## Appendix: Performance Baselines

The following ranges are **target values** from `specs/043-universal-modular-os/spec.md` (success criteria SC-001 through SC-013). They are not measured values from a benchmark run — they are the thresholds that Layer 8 performance tests must eventually verify. A performance regression is any result that crosses a threshold in the wrong direction on a nightly CI run.

### Boot Time (SC-004)

| Platform | Target | Condition |
|----------|--------|-----------|
| Desktop / Server | < 5 s | From kernel start to `[lifecycle] all apps spawned` on serial |
| Embedded (IoT, Robotics) | < 5 s | From kernel start in QEMU VM |
| Embedded with preloaded image | < 1 s | Supervisor + apps pre-staged in flash, no network load, no disk init |

"Preloaded image" means the initramfs is stored on flash/eMMC and the kernel is loaded directly into RAM by the bootloader. Boot time under QEMU emulation is expected to be 2–3× slower than real hardware.

### WASM Cold Start (SC-004, FR-012)

| Metric | Target | Notes |
|--------|--------|-------|
| Time from supervisor `spawn` command to first app stdout line | < 5 ms | Wasmtime JIT on desktop/server |
| Time from supervisor `spawn` command to first app stdout line | < 50 ms | wasm3 interpreter on IoT/MCU (no JIT) |

Cold start is measured from the moment the supervisor invokes `wasmtime run <app>.wasm` (or equivalent) to the first byte of app output on its stdout pipe. This includes WASM parsing, compilation (Wasmtime only), and main() initialization overhead.

### Supervisor and Runtime Memory (SC-006)

| Component | Target | Platform |
|-----------|--------|----------|
| Supervisor binary (static musl) | < 1 MB | All platforms |
| Supervisor + wasm3 runtime + 1 minimal app | < 8 MB total RSS | MCU / IoT embedded profiles |
| Supervisor + Wasmtime + 10 apps | < 64 MB total RSS | Desktop / Server |
| 200 concurrent WASM apps (SC-009) | No OOM, stable RSS | Desktop / Server |

The 8 MB embedded target applies to the `mcu-minimal` and `iot-edge` profiles. Wasmtime's JIT compiler cache grows with the number of unique apps loaded; the < 64 MB figure assumes 10 apps of approximately 5–50 KB each.

### GUI Frame Rate (SC from PR #65)

| Metric | Target | Platform |
|--------|--------|----------|
| VYOMA_DRAW flush rate (GUI dashboard) | >= 60 fps | Desktop `desktop-full` profile |
| VYOMA_DRAW flush rate (minimal display) | >= 30 fps | Embedded with display (`mobile` profile) |

60 fps at the `desktop-full` profile was achieved in PR #65 (double-buffered compositor with back-buffer blit). The 30 fps mobile target applies to the `virtio-gpu` device at 1080×2340 resolution in QEMU.

### OTA Update Timing (SC-003, SC-008)

| Metric | Target |
|--------|--------|
| Module hot-swap (stop old, start new) | < 2 s end-to-end |
| OTA update on IoT device (standard connectivity) | < 30 s from command to slot B running |
| Health check window (configurable, default) | 60 s |
| Automatic rollback from health check failure to slot A restored | < 5 s |

### Measurement Commands (Reference)

These commands will be used by the Layer 8 nightly CI job once `make perf-baseline` is implemented:

```bash
# Boot time:
time make smoke  # measures wall time from QEMU start to SMOKE: PASS

# WASM cold start (inside VM via log timestamps):
# Supervisor logs [spawn] <app> at time T1 and app writes first stdout at T2
# T2 - T1 = cold start latency per app

# Memory footprint (inside VM):
# After boot: cat /proc/meminfo | grep MemFree
# Supervisor RSS: cat /proc/1/status | grep VmRSS

# Frame rate (GUI profile):
# Count VYOMA_DRAW:flush calls per second in supervisor display loop
# make run-gui 2>&1 | grep -c 'VYOMA_DRAW:flush' over a 1-second window
```
