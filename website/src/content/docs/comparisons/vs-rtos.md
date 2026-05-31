---
title: VyomaOS vs Real-Time Operating Systems
description: Comparing VyomaOS's WASM sandbox model with FreeRTOS, Zephyr, and NuttX on isolation, portability, dynamic loading, and update mechanisms.
order: 4
---

# VyomaOS vs RTOS (FreeRTOS / Zephyr / NuttX)

Real-Time Operating Systems (FreeRTOS, Zephyr, NuttX) dominate the microcontroller and embedded device market. They provide task scheduling, hardware abstraction, and peripheral drivers for resource-constrained devices running on Cortex-M, RISC-V, and other MCU architectures.

VyomaOS's `mcu-minimal` platform profile targets the same hardware class. Its approach differs fundamentally: instead of running native tasks compiled per-architecture, VyomaOS runs WASM applications in a sandbox on top of a minimal supervisor, providing portable bytecode execution and capability-based isolation on MCU-class devices.

## At a Glance

| Dimension | VyomaOS (MCU profile) | FreeRTOS | Zephyr | NuttX |
|---|---|---|---|---|
| **Architecture** | WASM supervisor + wasm3 interpreter | Task scheduler + HAL | Task scheduler + HAL + subsystems | POSIX-like RTOS |
| **Target hardware** | Cortex-M4+, RISC-V | Cortex-M0+, RISC-V, Xtensa | Cortex-M, RISC-V, x86, ARC | Cortex-M, RISC-V, x86, SIM |
| **Minimum RAM** | 128 KB | 4-16 KB | 8-32 KB | 32-64 KB |
| **App format** | `wasm32-wasip2` bytecode | Native C/C++ tasks | Native C/C++ threads | Native C/C++ POSIX tasks |
| **App isolation** | WASM sandbox per app | None (shared memory space) | Optional MPU regions | Optional MPU regions |
| **Portability** | Cross-architecture (wasm32) | Per-architecture recompile | Per-architecture recompile | Per-architecture recompile |
| **Dynamic loading** | Yes (WASM binary swap) | Limited (loadable modules) | Limited (LLEXT) | Limited (ELF loader) |
| **Update mechanism** | WASM binary OTA, no reboot | Full firmware OTA, reboot required | MCUboot + firmware OTA | Full firmware OTA |
| **Real-time guarantees** | No hard real-time | Yes (configurable) | Yes (configurable) | Yes (POSIX real-time) |
| **Language support** | Any with WASM target | C, C++ | C, C++ (Rust experimental) | C, C++ |
| **Maturity** | Alpha | Production (since 2003) | Production (since 2016) | Production (since 2007) |
| **Ecosystem** | WASI standard + Rust crates | AWS IoT, massive HAL ecosystem | Nordic, NXP, Intel backing | POSIX compatibility layer |

## Isolation Model

### RTOS: Shared Memory, Shared Risk

Traditional RTOSes run all tasks in a shared address space. FreeRTOS tasks, Zephyr threads, and NuttX processes all share the same physical memory. A buffer overflow in one task can corrupt another task's stack or data.

Some RTOSes offer Memory Protection Unit (MPU) support:

- **Zephyr**: User-mode threads with MPU-enforced memory regions. Effective but limited by MPU hardware (typically 8-16 regions on Cortex-M).
- **FreeRTOS**: MPU port available for Cortex-M, providing task-level memory isolation.
- **NuttX**: Protected build mode with MPU, but most deployments use the flat memory model.

MPU isolation is coarse: it protects memory regions but does not control access to peripherals, network interfaces, or storage at the API level. A task with access to any I2C bus typically has access to all I2C buses.

### VyomaOS: WASM Sandbox + Capability Manifest

VyomaOS runs each app in a WASM sandbox (wasm3 interpreter on MCU targets). The sandbox provides:

- **Memory isolation**: Each WASM instance has its own linear memory. No shared address space, no buffer overflow across apps.
- **API-level capability control**: The `vyoma.toml` manifest declares specific peripherals. An app requesting GPIO pins 4 and 17 cannot access pin 22.
- **No raw hardware access**: Apps interact with peripherals through WASI interfaces, never through direct register manipulation.

```toml
# Example: MCU app manifest with specific peripheral access
[app]
name = "sensor-reader"
version = "1.0.0"
wasm = "sensor-reader.wasm"

[capabilities]
stdio = true

[capabilities.i2c]
bus = 1

[capabilities.gpio]
pins = [4, 17]
direction = "input"

# SPI, UART, ADC -- not declared, not accessible
```

This granularity exceeds what MPU hardware can enforce. The trade-off is performance: WASM interpretation is slower than native execution.

## Portability

### RTOS: Compile Per Architecture

RTOS applications are compiled to native machine code for each target architecture. Moving an application from a Cortex-M4 to a RISC-V MCU requires:

- Recompilation with a different toolchain.
- Potential source code changes for architecture-specific features.
- Retesting on the new hardware.
- Maintaining separate firmware images per board variant.

Zephyr's devicetree and Kconfig system makes this more manageable than FreeRTOS's per-port HAL, but the fundamental constraint remains: native code is architecture-specific.

### VyomaOS: Write Once, Run Anywhere

VyomaOS WASM apps compile to `wasm32-wasip2` once. The same binary runs on:

- Cortex-M4 MCU (via wasm3 interpreter, `mcu-minimal` profile)
- ARM64 SBC (via WAMR AOT, `iot-edge` profile)
- x86-64 workstation (via Wasmtime JIT, `desktop-full` profile)

No recompilation, no architecture-specific code paths, no per-board firmware images. The supervisor's Hardware Abstraction Layer (HAL) maps WASI peripheral interfaces to the underlying hardware.

This is particularly valuable for IoT device fleets with heterogeneous hardware. One `.wasm` binary deploys to every device regardless of its MCU architecture.

## Dynamic Loading and Updates

### RTOS: Monolithic Firmware Updates

Most RTOS deployments ship as a single monolithic firmware image. Updating any component requires:

1. Building a new firmware image.
2. Transferring the entire image to the device (often MB-sized, over slow links).
3. Flashing and rebooting.
4. Hope the new firmware boots correctly (or rely on a bootloader like MCUboot for rollback).

Zephyr's LLEXT and FreeRTOS's loadable modules offer partial dynamic loading, but they are limited and not widely adopted.

### VyomaOS: Per-App WASM Updates

VyomaOS can update individual apps without touching the base system:

1. Transfer the new `.wasm` file (typically 1-150 KB).
2. Supervisor swaps the binary and restarts the app.
3. Other apps continue running uninterrupted.
4. Rollback is instant: restore the previous `.wasm` file.

The OTA update manager supports A/B slot updates for the supervisor itself, with health-check and automatic rollback if the new version fails.

For a fleet of 1,000 devices, updating a single app means transferring ~100 KB per device instead of a multi-MB firmware image. Over constrained networks (LoRa, NB-IoT, satellite), this difference is critical.

| Update Scenario | RTOS | VyomaOS |
|---|---|---|
| Single app update | Full firmware rebuild + flash | Swap .wasm file (~100 KB) |
| OS update | Full firmware rebuild + flash | Supervisor binary swap + reboot |
| Rollback | MCUboot A/B (if configured) | Instant file swap or A/B slot |
| Downtime during update | Full reboot required | Only affected app restarts |
| Network transfer | 1-4 MB firmware image | 1-150 KB WASM binary |

## Real-Time Performance

This is the most significant trade-off. Traditional RTOSes provide hard real-time guarantees:

- **FreeRTOS**: Configurable tick rate, priority-based preemptive scheduling, deterministic interrupt latency.
- **Zephyr**: Tickless kernel, priority-based scheduling, kernel-level synchronization primitives.
- **NuttX**: POSIX real-time scheduling, priority inheritance, deterministic behavior.

VyomaOS on the MCU profile uses the wasm3 interpreter, which adds interpretation overhead and non-deterministic execution time. This makes VyomaOS unsuitable for hard real-time requirements (motor control, safety-critical timing, sub-millisecond interrupt response).

| Performance Metric | RTOS (native) | VyomaOS (wasm3) |
|---|---|---|
| Interrupt latency | < 1 us (deterministic) | Not applicable (no direct interrupts) |
| Task switch time | < 5 us | N/A (supervisor-managed) |
| Compute throughput | Native speed | 5-20x slower (interpreted) |
| Memory overhead per task | 1-4 KB stack | 64+ KB (WASM linear memory) |

For applications where timing is advisory rather than safety-critical (sensor polling, data aggregation, UI updates, network communication), the interpretation overhead is acceptable.

## Memory Footprint

RTOSes are designed for extremely constrained devices:

| Component | FreeRTOS | Zephyr | VyomaOS (MCU) |
|---|---|---|---|
| Kernel/scheduler | 6-12 KB | 8-32 KB | ~40 KB (supervisor) |
| WASM runtime | N/A | N/A | ~50 KB (wasm3) |
| Minimum useful system | ~16 KB | ~32 KB | ~128 KB |
| Per-task/app overhead | 1-4 KB | 1-4 KB | 64+ KB |

VyomaOS requires more memory than a bare RTOS. The 128 KB minimum means it fits on Cortex-M4 and above but not on the smallest Cortex-M0 devices where FreeRTOS thrives. For devices with 256 KB+ RAM (increasingly common and inexpensive), the overhead is acceptable.

## When to Choose an RTOS

- You need hard real-time guarantees (motor control, safety-critical systems, sub-millisecond timing).
- Your device has less than 128 KB RAM.
- You are building a single-purpose firmware with no need for app isolation.
- You need direct hardware register access for maximum performance.
- You require certifications (IEC 62304, DO-178C) that rely on RTOS-specific tooling.
- You need production stability today with extensive vendor support.

## When to Choose VyomaOS

- You manage a fleet of heterogeneous MCU devices and want one app binary for all.
- You need per-app isolation on embedded devices without MMU hardware.
- You want to update individual apps over constrained networks (LoRa, NB-IoT) without full firmware OTA.
- Your application is not hard real-time (sensor aggregation, display, communication).
- You want to write embedded apps in Rust, Go, or Python instead of only C/C++.
- You want fine-grained peripheral capability control per app (specific GPIO pins, I2C buses).

## Honest Assessment

FreeRTOS, Zephyr, and NuttX are production-grade systems with decades of deployment history, extensive hardware support, and certifications for safety-critical applications. They are the right choice for most embedded projects today, especially those with hard real-time requirements or extreme memory constraints.

VyomaOS's MCU profile is experimental. Its value proposition is strongest in scenarios that traditional RTOSes handle poorly: multi-app isolation on devices without an MMU, cross-architecture portability, and incremental OTA updates. The WASM interpretation overhead and higher memory floor are real costs.

The bet is that MCU capabilities continue to grow (Cortex-M33 and M55 with 1 MB+ RAM are increasingly common) while the need for portable, updatable, isolated embedded applications increases with IoT fleet management. If your devices have the resources and your workload tolerates interpretation overhead, VyomaOS offers a genuinely different approach to embedded software deployment.
