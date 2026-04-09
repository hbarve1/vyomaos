# VyomaOS — Comparison Matrix & Performance Tracker

## Positioning

VyomaOS is a **WASM-first operating system**: the Linux kernel provides hardware abstraction only. There is no native userland, no shell, no package manager. Every application is a `wasm32-wasip2` binary executed by Wasmtime under a Rust PID 1 supervisor. Capabilities (filesystem, network, display, stdio) are declared per-app in a manifest and enforced at runtime.

**Core thesis**: shrink the attack surface to kernel + Wasmtime + supervisor. No C userland, no interpreters, no shell injection surface. Apps are byte-identical across machines (WASM is portable bytecode). The runtime enforces capability isolation without Linux namespaces or cgroups.

---

## Comparison Matrix

| Dimension | **VyomaOS** | Alpine Linux | Flatcar / Bottlerocket | MirageOS (unikernel) | OSv (unikernel) | Container (Docker/OCI) |
|---|---|---|---|---|---|---|
| **App execution model** | WASM (Wasmtime WASI Preview 2) | Native ELF | Native ELF (systemd minimal) | Native OCaml/MirageLib (no kernel) | JVM / native (single process) | Native ELF (namespaced) |
| **Kernel** | Linux 5.10 LTS (allnoconfig, 2.3 MB) | Linux (generic, ≥5 MB) | Linux (generic, ≥5 MB) | None (runs on hypervisor directly) | None (libOS on hypervisor) | Host kernel (shared) |
| **PID 1** | Rust supervisor (697 KB static musl) | OpenRC / musl init | systemd | N/A (single process = OS) | N/A | containerd shim |
| **Native shell** | None (by design) | ash/busybox | None (immutable) | None | None | sh (via image) |
| **Package manager** | None (by design) | apk | None (image-based) | opam (build time) | None | apt/apk (via layer) |
| **App isolation** | WASM capability sandbox | Linux DAC + optional namespaces | Linux namespaces + seccomp | Single process, no isolation | Single process, no isolation | Linux namespaces + seccomp + cgroups |
| **Capability model** | Manifest-declared, runtime-enforced | Discretionary (file permissions) | Discretionary + SELinux | None (all-or-nothing) | None | OCI spec (optional) |
| **Attack surface** | Kernel + Wasmtime + supervisor | Kernel + musl libc + full userland | Kernel + systemd + full userland | Hypervisor only | Hypervisor only | Kernel + container runtime + image |
| **App portability** | Any language → wasm32-wasip2 (Rust, Go, C, Python, JS) | Native only (arch-specific ELF) | Native only | OCaml / C (limited) | JVM / C | Native (arch-specific ELF) |
| **Binary determinism** | WASM byte-identical across hosts | ELF varies by libc/arch | ELF varies | varies | varies | Layer hash = reproducibility |
| **Image size (base)** | 18 MB initramfs (Wasmtime dominates at 61 MB total rootfs) | 3 MB (base) | 300–600 MB | 1–10 MB (single app image) | ~10–50 MB | 5–200 MB (varies by image) |
| **Kernel size** | 2.3 MB (allnoconfig + virtio + DRM) | 5–10 MB (generic) | 5–10 MB (generic) | N/A | N/A | N/A (host kernel) |
| **Boot time** | < 5 s (QEMU target) | 2–5 s (bare metal) | 3–10 s | < 1 s | < 1 s (hypervisor) | < 1 s (container start, kernel already up) |
| **GPU / Display** | DRM/virtio-gpu + fbcon + VYOMA_DRAW framebuffer protocol | Full DRM stack | Headless (server-oriented) | None | None | None (server-oriented) |
| **Networking** | virtio-net (planned, P11) | Full stack (Alpine default) | Full stack | Full stack (mirage-tcpip) | Full stack | Full stack |
| **Persistent storage** | 9P virtio (per-app capability-gated) | Full VFS | Full VFS | Block device (mirage-block) | Full VFS (LibC mapping) | Volume mounts |
| **IPC** | Supervisor-brokered mpsc channels | D-Bus / sockets / pipes | systemd / sockets | Direct function calls | Direct function calls | Unix sockets / pipes |
| **Language for apps** | Any WASM-targeting language | Any (C, Go, Python, etc.) | Any | OCaml primary (C possible) | JVM primary | Any |
| **Hot-code update** | Restart app (WASM binary swap) | Package update + restart | Image update + reboot | Recompile + rebuild image | Recompile + rebuild image | Image layer update |
| **Security model strength** | High (WASM sandbox + capability manifest) | Medium (DAC + optional MAC) | High (immutable image + seccomp) | Medium (single process, no isolation) | Medium (single process) | High (namespaces + seccomp + cgroups) |
| **Developer ergonomics** | Moderate (WASI ecosystem maturing) | High (familiar Linux) | Low (immutable, no shell) | Low (OCaml expertise required) | Low (JVM/C only) | High (familiar Linux + Docker) |
| **Use case fit** | Edge appliances, embedded displays, capability-secure single-purpose devices | General server / embedded | Cloud VM workloads (immutable infra) | Network services (high performance) | JVM microservices | Cloud microservices |

---

## Performance Baseline — Phase 09 (current)

Measured in QEMU on Apple Silicon (M-series) via `make run-gui`.

### Binary Sizes

| Component | Size | Notes |
|---|---|---|
| `bzImage` (kernel) | 2.3 MB | allnoconfig + virtio + 9P + DRM + fbcon |
| `initramfs.cpio.gz` | 18 MB | compressed; rootfs is 66 MB uncompressed |
| `wasmtime` binary | 61 MB | glibc variant, stripped; dominates rootfs |
| `supervisor` binary | 697 KB | static musl, Rust |
| `busybox` | 1.1 MB | retained for mount/poweroff helpers |
| `hello-world.wasm` | 85 KB | WASI P2 |
| `factorial.wasm` | 87 KB | WASI P2 |
| `ping.wasm` | 92 KB | WASI P2 + IPC |
| `pong.wasm` | 92 KB | WASI P2 + IPC |
| `calculator.wasm` | 113 KB | WASI P2 |
| `storage-demo.wasm` | 136 KB | WASI P2 + 9P filesystem |
| `gui-demo.wasm` | 71 KB | WASI P2 + VYOMA_DRAW |

### Timing (QEMU, not yet instrumented)

| Metric | Target | Measured | Phase |
|---|---|---|---|
| Boot to supervisor ready | < 5 s | TBD (P10) | P10 |
| First app output | < 6 s | TBD (P10) | P10 |
| App startup time (hello-world) | < 500 ms | TBD (P10) | P10 |
| HTTP GET latency (localhost) | < 10 ms | TBD (P11) | P11 |

---

## Phase-by-Phase Performance Tracker

Track how metrics evolve as features are added. Fill in each column after completing the phase.

| Metric | P01 | P02 | P03 | P04 | P05 | P06 | P07 | P08 | P09 | P10 | P11 | P12 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| Kernel size (MB) | — | 1.3 | 1.3 | 1.3 | 1.3 | 1.3 | 1.3 | 1.3 | **2.3** | 2.3 | 2.3 | 2.3 |
| initramfs.cpio.gz (MB) | — | — | 18 | 18 | 18 | 18 | 18 | 18 | **18** | ~18 | ~18 | ~18 |
| Rootfs uncompressed (MB) | — | — | 66 | 66 | 66 | 66 | 66 | 66 | **66** | ~66 | ~66 | ~66 |
| Supervisor binary (KB) | — | — | ~400 | ~400 | ~450 | ~550 | ~600 | ~650 | **697** | ~700 | ~750 | ~850 |
| App count | 0 | 0 | 1 | 1 | 3 | 5 | 6 | 6 | **7** | 7 | 8 | 9 |
| Boot to supervisor (s) | — | — | — | — | — | — | — | — | TBD | **TBD** | TBD | TBD |
| App startup (ms) | — | — | — | — | — | — | — | — | TBD | **TBD** | TBD | TBD |
| HTTP GET latency (ms) | — | — | — | — | — | — | — | — | N/A | N/A | **TBD** | TBD |
| IPC round-trip (ms) | — | — | — | — | — | — | — | — | TBD | TBD | TBD | **TBD** |

Notes:
- P02: kernel grew from ~1.3 MB (no DRM) to 2.3 MB in P09 when DRM/virtio-gpu config was added
- Wasmtime 61 MB dominates rootfs; initramfs compression ratio ≈ 3.7:1
- Timing rows to be filled with `dmesg` timestamps + supervisor instrumentation (P10 task P10T03)

---

## VyomaOS Unique Strengths

1. **Zero native userland attack surface.** No shell, no libc-linked user processes beyond the supervisor, no package manager. Shell injection attacks have no target.

2. **Capability-secure by default.** Every app declares what it needs (filesystem, network, display, stdio). Undeclared capabilities are inaccessible — not filtered, literally not wired up. The supervisor never passes an FD it didn't explicitly open for the app.

3. **Language-agnostic apps.** Any language with a WASM target (Rust, Go, C, Swift, Python, JS/TS via WASM, Kotlin) produces a portable binary that runs identically on any VyomaOS instance.

4. **Reproducible builds.** Docker-containerized Makefile with explicit stamps. Any engineer with Docker can reproduce the exact same `bzImage` + `initramfs.cpio.gz` from scratch.

5. **Minimal kernel.** 2.3 MB `allnoconfig` kernel with only the config entries needed: virtio-blk, virtio-9p, DRM/virtio-gpu, fbcon. No loadable modules. Smaller attack surface, faster boot.

6. **Lightweight IPC.** Supervisor-brokered mpsc channels. No D-Bus, no sockets for inter-app communication. Apps write `@appname: message` to stdout; supervisor routes to target's stdin.

---

## Known Limitations (vs alternatives)

| Limitation | Impact | Mitigation path |
|---|---|---|
| Wasmtime 61 MB binary | initramfs large (18 MB compressed) | Explore wasmtime-cranelift split / lazy loading in P11+ |
| No WASM JIT on aarch64 QEMU | Slower app startup under emulation | Irrelevant on native hardware; profile after bare-metal test |
| WASI P2 ecosystem still maturing | Some libraries not yet WASM-compatible | Growing; most Rust crates support `wasm32-wasip2` |
| No process namespaces yet | Apps share kernel namespace | P08 deferred tasks: user namespaces per app |
| Line-buffered IPC | Shell responsiveness limited | P12 raw tty mode planned |
| No WASM binary signing | Supply chain trust unverified | P08 deferred: WASM signature verification at load time |
