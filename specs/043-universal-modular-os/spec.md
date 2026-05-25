# Feature Specification: VyomaOS Universal Modular Operating System

**Feature Branch**: `043-universal-modular-os`

**Created**: 2026-05-25

**Status**: Draft

**Input**: User description: "VyomaOS — A WASM-first, capability-secure, modular operating system targeting universal deployment: IoT, robotics, microcontrollers, mobile, embedded systems, tablets, desktops, servers, and supercomputers. Built on a plug-and-play modular architecture adaptable to any CPU architecture and hardware platform. Currently targets wasm32-wasip2, expanding to wasm64. Positioned as an extended Linux kernel replacement where every app and system service runs as a sandboxed WebAssembly module with declared capabilities."

---

## Clarifications

### Session 2026-05-25

- Q: What is the platform rollout priority order? → A: MCU → IoT → Robotics → Mobile → Desktop → Server → HPC (bottom-up, start from most constrained)
- Q: What WASM runtime strategy for constrained devices? → A: Tiered — wasm3/WAMR for MCU/IoT, Wasmtime for Desktop/Server; supervisor abstracts the runtime difference
- Q: How should hardware peripheral access be modeled? → A: Per-peripheral capabilities in vyoma.toml (e.g., `gpio_pins = [2,4]`, `i2c_bus = 1`), fine-grained and enforced by supervisor; ReBAC (Relationship-Based Access Control) planned as future security extension for inter-module relationship enforcement
- Q: What observability and diagnostics strategy? → A: Structured logs + health heartbeat as baseline for all platforms; full OpenTelemetry-compatible telemetry available on devices with sufficient resources (desktop/server/rich IoT gateways); platform profile determines active tier
- Q: What OTA update safety model? → A: A/B slot with automatic rollback — two module slots, supervisor validates new version on first boot, rolls back if health check fails

---

## Market Analysis

### Target Market Segments

| Segment | Market Size (2026 est.) | Key Players | VyomaOS Opportunity |
|---------|------------------------|-------------|---------------------|
| **IoT / Embedded** | $1.1T (IoT ecosystem) | FreeRTOS, Zephyr, RIOT OS, NuttX | Unified app model across constrained and rich devices |
| **Robotics** | $75B | ROS 2 + Linux, VxWorks, QNX | Sandboxed sensor/actuator modules, safe hot-swap |
| **Microcontrollers** | $28B (MCU market) | FreeRTOS, Mbed OS, Zephyr | WASM apps on 32-bit MCUs with capability isolation |
| **Mobile** | $450B (mobile OS) | Android (Linux), iOS (XNU) | Secure-by-default app model, no legacy C userland |
| **Tablets / Desktop** | $180B (PC market) | Windows, macOS, Linux distros | Deterministic binaries, cross-arch portability |
| **Servers** | $130B (server OS) | Linux, Windows Server | Lightweight WASM microservices, minimal attack surface |
| **Supercomputers** | $15B (HPC market) | Linux (100% of TOP500) | Reproducible compute jobs, sandboxed workloads |

### Industry Trends Favoring VyomaOS

1. **WASM beyond the browser**: WASI Preview 2 is production-ready; Kubernetes is adopting WASM as a first-class compute citizen (Kube-Wasm, CNCF incubating)
2. **Supply chain security**: Demand for deterministic, reproducible builds is exploding post-SolarWinds/Log4j
3. **Edge computing**: Sub-5ms cold starts and 10x memory savings over containers drive WASM adoption at the edge
4. **Capability-based security**: Google Fuchsia and seL4 validate the market for fine-grained permission models
5. **Hardware heterogeneity**: RISC-V proliferation means more CPU architectures to support — WASM's arch-neutrality becomes critical

---

## Comprehensive OS Comparison Matrix

### Category 1: Architecture & Design

| Feature | VyomaOS | Linux | Windows | Fuchsia | seL4 | FreeRTOS | Zephyr | RIOT OS | NuttX | MirageOS |
|---------|---------|-------|---------|---------|------|----------|--------|---------|-------|----------|
| **Kernel type** | Monolithic (Linux) + WASM supervisor | Monolithic | Hybrid | Microkernel (Zircon) | Microkernel | Micro-kernel | Micro-kernel | Micro-kernel | Micro-kernel (POSIX) | Unikernel |
| **App runtime** | WASM/WASI P2 | Native ELF | Native PE/UWP | Native (Fuchsia packages) | Native | Native (C tasks) | Native (C tasks) | Native (C/C++) | Native (C) | OCaml/native |
| **App sandboxing** | Mandatory (WASM) | Optional (containers/seccomp) | Optional (AppContainer) | Mandatory (namespaces) | Mandatory (capabilities) | None | Optional (MPU) | None | None | N/A (single app) |
| **Modularity** | Plug-and-play modules | Loadable kernel modules | Drivers/services | Component framework | Minimal (by design) | Libraries | Kconfig subsystems | Modules | Configurable | Library OS |
| **Binary format** | WASM (portable) | ELF (arch-specific) | PE (arch-specific) | Fuchsia packages | ELF (arch-specific) | Firmware blob | Firmware blob | ELF (arch-specific) | Firmware blob | Unikernel image |
| **Binary determinism** | Byte-identical | Varies | Varies | Hermetic packages | Varies | Varies | Varies | Varies | Varies | Varies |

### Category 2: Hardware & Architecture Support

| Feature | VyomaOS | Linux | Windows | Fuchsia | seL4 | FreeRTOS | Zephyr | RIOT OS | NuttX | MirageOS |
|---------|---------|-------|---------|---------|------|----------|--------|---------|-------|----------|
| **CPU architectures** | Any (via WASM) | 30+ arch | x86, ARM | x86-64, ARM64 | ARM, x86, RISC-V | 40+ MCU | 500+ boards | 32-bit MCUs | 8-bit to 64-bit | x86, ARM |
| **Microcontroller** | Planned (wasm32) | Not suited | No | No | Limited | Primary target | Primary target | Primary target | Primary target | No |
| **Desktop/Laptop** | Yes (current) | Yes | Yes | Planned | No | No | No | No | No | No |
| **Server** | Planned | Yes | Yes | No | No | No | No | No | No | No (cloud only) |
| **Supercomputer** | Planned | Yes (100% TOP500) | No | No | No | No | No | No | No | No |
| **Mobile** | Planned | Yes (Android) | No | Yes | No | No | No | No | No | No |
| **IoT / Edge** | Yes | Yes (Yocto/Buildroot) | Yes (IoT Core) | No | Yes | Yes | Yes | Yes | Yes | No |
| **Real-time capable** | Planned | PREEMPT_RT patch | No | No | Yes (formally verified) | Yes (core feature) | Yes | Yes | Yes | No |
| **Hardware driver model** | WASM driver modules | Kernel drivers (C) | WDM/WDF (C/C++) | Component drivers | Minimal | BSP/HAL | Devicetree + Kconfig | Board support | HAL | None (hypervisor) |

### Category 3: Security Model

| Feature | VyomaOS | Linux | Windows | Fuchsia | seL4 | FreeRTOS | Zephyr | RIOT OS | NuttX |
|---------|---------|-------|---------|---------|------|----------|--------|---------|-------|
| **Security approach** | Deny-by-default capabilities | Allow-by-default + filters | ACL + integrity levels | Capability-based | Capability-based (verified) | None (trusted code) | MPU-based | None | None |
| **App isolation** | WASM sandbox + capabilities | Process + namespaces | Process + AppContainer | Namespace + capabilities | Formal capability model | None (shared memory) | Thread + MPU | Thread isolation | Process (optional) |
| **Formal verification** | No | No | No | No | Yes (full proof) | No | No | No | No |
| **Supply chain security** | Deterministic WASM binaries | Package signing (varies) | Code signing | Hermetic builds | N/A | N/A | Signed firmware | N/A | N/A |
| **Attack surface** | Kernel + Wasmtime + Supervisor | Full POSIX userland | Full Win32 userland | Zircon + runners | Minimal kernel only | Entire firmware | Entire firmware | Entire firmware | Entire firmware |
| **C userland exposure** | None | Full | Full | Limited | Minimal | Full (C firmware) | Full (C firmware) | Full (C/C++) | Full (C) |
| **Privilege escalation risk** | Structurally eliminated (WASM) | Possible (CVEs common) | Possible (CVEs common) | Reduced (capabilities) | Eliminated (proven) | N/A (single privilege) | Reduced (MPU) | N/A | N/A |

### Category 4: Developer Experience

| Feature | VyomaOS | Linux | Windows | Fuchsia | seL4 | FreeRTOS | Zephyr |
|---------|---------|-------|---------|---------|------|----------|--------|
| **App languages** | Any → WASM (Rust, Go, C, Python, etc.) | Any native | C/C++/C#/.NET | C++/Rust/Dart | C/C++ | C | C |
| **App size** | 1–10 KB per app | MB–GB | MB–GB | MB | KB–MB | KB | KB |
| **Cross-platform apps** | Write once, run everywhere | Arch-specific rebuild | Windows-only | Fuchsia-only | Platform-specific | MCU-specific | Board-specific |
| **Hot reload / update** | Replace .wasm module at runtime | Requires restart (usually) | Requires restart | Component update | N/A | OTA firmware flash | OTA firmware flash |
| **Package manager** | Built-in (WASM-native) | apt/yum/pacman etc. | winget/Store | fpm | N/A | N/A | West |
| **Build reproducibility** | Guaranteed (WASM bytecode) | Best-effort | No | Hermetic (GN+Ninja) | Manual | Toolchain-dependent | CMake-based |

### Category 5: Performance & Resource Usage

| Feature | VyomaOS | Linux | Windows | Fuchsia | FreeRTOS | Zephyr | RIOT OS |
|---------|---------|-------|---------|---------|----------|--------|---------|
| **Minimum RAM** | ~4 MB (supervisor + Wasmtime) | ~8 MB (embedded) | ~2 GB | ~512 MB | < 10 KB | < 8 KB | < 1.5 KB |
| **Kernel size** | 2.3 MB (allnoconfig) | 5–100 MB | ~30 MB | ~10 MB | < 10 KB | < 100 KB | < 50 KB |
| **Boot time** | < 5s (QEMU) | 1–30s | 15–60s | ~5s | Instant (MCU) | Instant (MCU) | Instant (MCU) |
| **Cold start (app)** | < 5ms (WASM) | ~50ms (process fork) | ~100ms | ~100ms | N/A | N/A | N/A |
| **App overhead** | WASM interpreter/JIT ~10–30% | Native (baseline) | Native (baseline) | Native (baseline) | Native (baseline) | Native (baseline) | Native (baseline) |
| **Power efficiency** | Good (minimal kernel) | Good | Poor (background services) | Good | Excellent (bare metal) | Excellent | Excellent |

### Category 6: Ecosystem & Maturity

| Feature | VyomaOS | Linux | Windows | Fuchsia | seL4 | FreeRTOS | Zephyr |
|---------|---------|-------|---------|---------|------|----------|--------|
| **First release** | 2025 | 1991 | 1985 | 2016 | 2009 | 2003 | 2016 |
| **Maturity** | Early (Phase 17) | Production (35 years) | Production (40 years) | Beta | Production (niche) | Production | Production |
| **Community size** | 1 contributor | Millions | Microsoft | Google | seL4 Foundation | Amazon + community | Linux Foundation |
| **Production deployments** | Development | Billions | Billions | Google Nest Hub | Defense/aerospace | Millions of devices | Growing |
| **App ecosystem** | 200+ WASM apps | Millions | Millions | Hundreds | Dozens | Libraries only | Growing |
| **Commercial support** | None yet | Red Hat, Canonical, etc. | Microsoft | Google | Proofcraft, others | Amazon (AWS IoT) | Nordic, Intel, etc. |
| **License** | Open source | GPL-2.0 | Proprietary | BSD-3 + Apache-2.0 | GPL-2.0 (kernel) | MIT | Apache-2.0 |

---

## VyomaOS Competitive Advantages

### Unique Value Propositions

1. **Universal app format**: One `.wasm` binary runs on any VyomaOS instance — from microcontroller to supercomputer — without recompilation
2. **Structural security**: WASM sandbox eliminates entire classes of vulnerabilities (buffer overflow, use-after-free, ROP/JOP) at the bytecode level, not through bolted-on filters
3. **Zero-config isolation**: Unlike Linux (where sandboxing requires explicit setup), VyomaOS apps are sandboxed by default with no configuration
4. **Deterministic deployments**: Byte-identical binaries across builds and hosts eliminate "works on my machine" and supply chain uncertainty
5. **Language-agnostic**: Any language with a WASM target works — Rust, Go, C, Python, Swift, JS/TS — unlike RTOS platforms locked to C
6. **Modular plug-and-play**: Swap hardware drivers, system services, and apps as WASM modules without kernel recompilation

### Where VyomaOS Must Improve

1. **WASM performance gap**: 10–30% overhead vs native for compute-intensive workloads
2. **No multi-threading in WASI**: WASM threads spec is not yet stable — limits HPC and database use cases
3. **Real-time guarantees**: No RTOS-grade deterministic scheduling yet
4. **Minimum RAM floor**: Wasmtime runtime requires ~4 MB — too heavy for sub-$1 MCUs that FreeRTOS/Zephyr serve
5. **Driver ecosystem**: No existing hardware driver base; everything must be written from scratch
6. **Maturity gap**: 1 contributor vs established ecosystems with millions of users

---

## User Scenarios & Testing *(mandatory)*

### User Story 1 - IoT Device Manufacturer Deploys Secure Edge Firmware (Priority: P1)

An IoT device manufacturer wants to deploy sandboxed application firmware across a fleet of heterogeneous edge devices (ARM, RISC-V, x86) using a single binary format. They need capability-declared apps that can be updated over-the-air without reflashing the entire firmware.

**Why this priority**: IoT/embedded is the highest-volume market segment, and VyomaOS's WASM portability + capability security directly addresses the #1 pain point (firmware fragmentation + security vulnerabilities in C-based RTOS firmware).

**Independent Test**: Can be tested by deploying the same `.wasm` app binary to two different architecture targets (e.g., ARM and x86 QEMU instances) and verifying identical behavior with declared capabilities enforced.

**Acceptance Scenarios**:

1. **Given** a WASM app built targeting wasm32-wasip2, **When** deployed to an ARM-based device and an x86-based device, **Then** the app produces identical output on both without recompilation
2. **Given** an app with `network = false` in its manifest, **When** the app attempts to open a TCP connection, **Then** the supervisor blocks the operation because no network interface was wired up
3. **Given** a running device in the field, **When** a new `.wasm` app version is pushed via OTA, **Then** the app is replaced and restarted without rebooting the device or affecting other running apps

---

### User Story 2 - Robotics Engineer Deploys Modular Sensor/Actuator Stack (Priority: P2)

A robotics engineer wants to compose a robot's software stack from independent WASM modules (camera driver, LIDAR processor, path planner, motor controller) that can be individually updated, tested, and hot-swapped without rebooting the system.

**Why this priority**: Robotics requires the plug-and-play modularity that is VyomaOS's core differentiator — each module runs sandboxed with declared capabilities, enabling safe live updates on running hardware.

**Independent Test**: Can be tested by running 4+ WASM modules concurrently, hot-swapping one module (e.g., replacing the path planner) while the others continue operating, and verifying no disruption to other modules.

**Acceptance Scenarios**:

1. **Given** 4 WASM modules running (camera, LIDAR, planner, motor), **When** the planner module is replaced with a new version, **Then** the other 3 modules continue operating without interruption
2. **Given** a motor controller module with `gpio = true` capability, **When** deployed alongside a camera module with `gpio = false`, **Then** only the motor controller can access GPIO pins

---

### User Story 3 - Enterprise Deploys Cross-Platform Desktop + Server Apps (Priority: P3)

An enterprise IT team wants to deploy the same productivity applications across desktops (x86, ARM), tablets, and servers using a single build pipeline, with the OS enforcing security policies through capability manifests rather than external MDM tools.

**Why this priority**: Desktop/server is the long-term vision and validates the "universal" claim, but requires more maturity (windowing, networking, filesystem) than IoT/robotics paths.

**Independent Test**: Can be tested by building a productivity app once and deploying it to a desktop VyomaOS instance and a server VyomaOS instance, verifying consistent behavior and capability enforcement on both.

**Acceptance Scenarios**:

1. **Given** a document editor app built as wasm32-wasip2, **When** deployed to a desktop instance and a headless server instance, **Then** it renders via VYOMA_DRAW on desktop and serves via HTTP on server, using the same binary
2. **Given** an enterprise security policy requiring `network = false` for document apps, **When** the app manifest omits network capability, **Then** the app cannot exfiltrate data regardless of any vulnerability in the app code

---

### User Story 4 - Supercomputer Operator Runs Reproducible Compute Jobs (Priority: P4)

A research computing operator wants to run deterministic, reproducible compute jobs across thousands of nodes, where every job produces byte-identical results regardless of the host hardware architecture.

**Why this priority**: Supercomputing validates the extreme scalability claim and leverages WASM's determinism, but requires wasm64 support and threading — both planned but not yet implemented.

**Independent Test**: Can be tested by running the same computational WASM job on two different architecture nodes and comparing output byte-for-byte.

**Acceptance Scenarios**:

1. **Given** a compute job compiled to wasm64, **When** run on an x86-64 node and an ARM64 node, **Then** both produce byte-identical output
2. **Given** a cluster of 100 nodes, **When** 100 instances of the same WASM job are launched, **Then** all 100 complete with identical results and resource usage is bounded by declared capabilities

---

### Edge Cases

- What happens when a WASM app exceeds its declared memory capability on a constrained MCU device?
- How does the system handle hardware interrupts that require sub-microsecond response on real-time robotics platforms?
- What happens when a wasm32 app is deployed to a wasm64-only instance (or vice versa)?
- How does the supervisor handle a malicious app that spin-loops to consume CPU without producing output (watchdog evasion)?
- What happens when OTA update connectivity is lost mid-transfer on an IoT device?
- How does the system handle conflicting capability declarations when two apps require exclusive access to the same hardware resource (e.g., a single UART)?

## Requirements *(mandatory)*

### Functional Requirements

- **FR-001**: System MUST execute wasm32-wasip2 binaries on any supported CPU architecture without recompilation
- **FR-002**: System MUST enforce capability isolation — apps without a declared capability MUST have no access to the corresponding resource interface
- **FR-003**: System MUST support a modular, plug-and-play architecture where system services, drivers, and apps can be added, removed, or replaced as individual WASM modules without kernel recompilation
- **FR-004**: System MUST support concurrent execution of multiple isolated WASM apps with independent lifecycle management (start, stop, restart, update)
- **FR-005**: System MUST support hot-swapping of individual WASM modules without rebooting the system or disrupting other running modules
- **FR-006**: System MUST produce deterministic, byte-identical WASM binaries across builds regardless of host build environment
- **FR-007**: System MUST support OTA (over-the-air) updates of individual WASM apps on deployed devices
- **FR-008**: System MUST support wasm64 as a compilation target for memory-intensive workloads (servers, HPC)
- **FR-009**: System MUST support inter-module communication through a supervisor-mediated IPC broker
- **FR-010**: System MUST support declarative capability manifests (vyoma.toml) for every app, declaring: stdio, filesystem, network, display, shell, mouse, GPIO, sensors, actuators, and future hardware interfaces
- **FR-011**: System MUST provide a hardware abstraction layer (HAL) that allows WASM driver modules to interface with hardware without architecture-specific code in the app
- **FR-012**: System MUST boot to a functional supervisor within 5 seconds on standard hardware and within 1 second on embedded platforms with preloaded images
- **FR-013**: System MUST support at least the following CPU architectures: x86-64, ARM64 (AArch64), ARM32, RISC-V (32/64-bit)
- **FR-014**: System MUST provide a package manager capable of installing, updating, and removing signed WASM app bundles
- **FR-015**: System MUST support seccomp BPF (or equivalent) as a defense-in-depth layer on platforms that support it
- **FR-016**: System MUST support running on bare metal, in virtual machines, and in containers
- **FR-017**: System MUST provide a supervisor API for process management (list, kill, restart, logs, resource usage)
- **FR-018**: System MUST support capability-declared access to hardware peripherals (GPIO, I2C, SPI, UART, ADC) on embedded/robotics platforms, with per-peripheral granularity in the manifest (e.g., `gpio_pins = [2,4]`, `i2c_bus = 1`)
- **FR-019**: System MUST support a tiered WASM runtime model — lightweight interpreters (wasm3/WAMR) for MCU/IoT platforms and JIT/AOT runtimes (Wasmtime) for desktop/server platforms, with the supervisor abstracting the runtime difference from apps
- **FR-020**: System MUST implement A/B slot OTA updates with automatic rollback — two module slots per app, supervisor validates new version health on first boot, and rolls back to the previous slot if health check fails
- **FR-021**: System MUST emit structured log events and periodic health heartbeats from all running modules, with the supervisor aggregating and exporting them via network when available
- **FR-022**: System SHOULD support full OpenTelemetry-compatible telemetry (traces, metrics, logs) on platforms with sufficient resources, selectable via platform profile
- **FR-023**: System SHOULD support ReBAC (Relationship-Based Access Control) as a future extension, enabling inter-module relationship declarations (e.g., `controls`, `reads_from`, `administered_by`) enforced by the supervisor at IPC routing time

### Key Entities

- **WASM Module**: A compiled wasm32-wasip2 or wasm64-wasip2 binary representing an app, driver, or system service. Key attributes: name, version, hash, capabilities, restart policy
- **Capability Manifest (vyoma.toml)**: Per-module declaration of required capabilities. Determines what WASI interfaces are wired up at spawn time
- **Supervisor**: The native PID 1 binary that manages module lifecycle, IPC, display, input, and capability enforcement
- **Hardware Abstraction Layer (HAL)**: Platform-specific layer that exposes hardware interfaces as typed WASI imports consumable by WASM modules
- **Platform Profile**: A configuration describing a deployment target (e.g., "iot-minimal", "desktop-full", "server-headless", "robotics-rt") that determines which supervisor modules and HAL drivers are included
- **Package Registry**: A signed repository of WASM modules for discovery, installation, and updates

## Success Criteria *(mandatory)*

### Measurable Outcomes

- **SC-001**: The same WASM app binary runs without modification on at least 4 different CPU architectures (x86-64, ARM64, ARM32, RISC-V)
- **SC-002**: Apps without a declared capability have zero access to the corresponding resource — verified by attempting every undeclared capability and confirming denial
- **SC-003**: Individual WASM modules can be replaced on a running system within 2 seconds without disrupting other modules
- **SC-004**: System boots to a functional state in under 5 seconds on desktop/server and under 1 second on embedded platforms
- **SC-005**: 95% of developers can create, build, and deploy a new WASM app within 15 minutes following the documentation
- **SC-006**: Supervisor + minimal system services operate within 8 MB RAM on embedded platforms
- **SC-007**: The same WASM compute job produces byte-identical output across 3+ different host architectures
- **SC-008**: OTA app updates complete in under 30 seconds on IoT devices with standard connectivity
- **SC-009**: System supports at least 200 concurrent WASM apps on desktop/server platforms without resource exhaustion
- **SC-010**: The modular architecture allows a deployment target to be configured (e.g., IoT vs desktop vs server) by selecting a platform profile, with no code changes to the supervisor core
- **SC-011**: OTA updates with A/B rollback complete successfully — if a new module version fails its health check within 60 seconds, the system automatically reverts to the previous working version with zero manual intervention
- **SC-012**: Structured health heartbeats from all running modules are receivable by a remote monitoring endpoint within 5 seconds of module startup on network-capable devices
- **SC-013**: The same WASM app binary runs identically on wasm3 (MCU) and Wasmtime (desktop) runtimes, producing equivalent output regardless of the underlying execution engine

## Assumptions

- The Linux kernel continues to be used as the hardware abstraction layer for the initial deployment targets; native bare-metal supervisors for MCUs are a future phase
- Wasmtime (or compatible WASM runtime) remains the primary execution engine; alternative runtimes (WAMR, wasm3) will be evaluated for constrained platforms
- WASI Preview 2 specification is stable enough to build on; VyomaOS will track spec evolution but not depend on unstable proposals
- wasm64 support in toolchains (Rust, LLVM) will be available by the time server/HPC deployment targets are prioritized
- Real-time guarantees require PREEMPT_RT kernel patches or a future native RTOS supervisor — this is out of scope for the initial modular architecture work
- GPIO, I2C, SPI, UART, and ADC hardware interfaces will be exposed through custom WASI host functions defined by VyomaOS, not through upstream WASI proposals (which do not cover embedded hardware)
- Sub-$1 MCUs with < 256 KB RAM are out of scope for the initial WASM-based architecture; these will require a lightweight interpreter (wasm3/WAMR) adaptation in a future phase
- The plug-and-play module system applies to userspace WASM modules; the kernel itself remains a monolithic Linux build with minimal config per platform
- Platform rollout follows a bottom-up strategy: MCU → IoT → Robotics → Mobile → Desktop → Server → HPC — starting from the most constrained ensures the architecture scales upward naturally
- The WASM runtime is a pluggable component of the supervisor — wasm3/WAMR on constrained platforms, Wasmtime on rich platforms — apps are unaware of which runtime executes them
- OTA updates use an A/B slot model with automatic rollback; partial or failed updates never leave a device in an unrecoverable state
- ReBAC-style relationship-based access control is a planned extension; the initial system uses flat per-peripheral capability declarations
