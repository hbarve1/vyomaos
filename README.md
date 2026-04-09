# VyomaOS

A **WASM-first operating system** with the long-term goal of becoming a lightweight but fully capable general-purpose OS — on par with Windows, macOS, Android, and Ubuntu — built from the ground up on a capability-secure WebAssembly foundation.

## The Vision

Most operating systems carry decades of accumulated complexity: C runtimes, shared libraries, POSIX quirks, shell injection surfaces. Every app inherits all of it. Android made progress — apps run in a managed runtime with a permission model — but native code still bypasses it entirely.

VyomaOS starts over with one rule: **the runtime is the OS boundary**.

- The Linux kernel handles hardware, drivers, and process isolation. Nothing else.
- Every application is a `wasm32-wasip2` binary. No native userland, no shell, no C runtime exposed to apps.
- A Rust PID 1 supervisor manages app lifecycle, IPC, and capability enforcement.
- Capabilities (filesystem, network, display, stdio) are declared per-app in a manifest and enforced at boot. Undeclared capabilities are not filtered — they are never wired up.

The result scales from an 18 MB embedded appliance today to a full desktop OS tomorrow, with the same security model at every scale.

## Long-Term Goals

| Feature | Goal |
|---|---|
| **Package manager** | `vyoma-pkg` — installs, updates, removes signed `wasm32-wasip2` app bundles; no native binaries required |
| **App store** | Curated registry of signed `.wasm` bundles; one-command install, sandboxed by default |
| **Windowed GUI** | Per-app windows managed by a WASM compositor; keyboard/mouse routing via focus manager |
| **Desktop shell** | App launcher, taskbar, notifications — all WASM, rendered on DRM/virtio-gpu |
| **Networking** | Full TCP/IP via WASI sockets; HTTP/HTTPS/DNS available to apps with `network = true` |
| **Multi-user / auth** | User identity via capability tokens; user-scoped filesystem without UNIX uid/gid dependency |
| **Developer tools** | WASM-native compiler toolchain, debugger, REPL — all installable as packages |
| **Hardware support** | USB, audio, camera, sensors — each exposed as a typed WASI interface |
| **Accessibility** | Screen reader, high-contrast, input assistance — first-class WASM apps |
| **OTA updates** | Atomic supervisor + kernel updates with rollback; app updates via registry diff |

## Why WebAssembly

- **Portability**: the same `.wasm` binary runs identically on any VyomaOS instance, any architecture.
- **Safety**: strong sandbox; no app can access resources not explicitly granted in its manifest.
- **Language-agnostic**: Rust, Go, C, Swift, Python, JS/TS — any language with a WASM target works.
- **Small footprint**: apps are 71–136 KB today. No shared library sprawl.
- **Determinism**: WASM bytecode is byte-identical across builds and hosts.

## Current State (Phase 9 of 12)

VyomaOS boots in QEMU in under 5 seconds to a Rust supervisor running 7 concurrent WASM apps:

```
Linux 5.10 (allnoconfig, 2.3 MB)
  └── Rust supervisor (697 KB, static musl, PID 1)
        ├── hello-world.wasm    (85 KB)
        ├── calculator.wasm     (113 KB)
        ├── factorial.wasm      (87 KB)
        ├── ping.wasm ←──IPC──→ pong.wasm  (92 KB each)
        ├── storage-demo.wasm   (136 KB, 9P persistent storage)
        └── gui-demo.wasm       (71 KB, DRM framebuffer display)
```

Working features: concurrent scheduler, supervisor IPC broker, seccomp BPF denylist, 9P virtio persistent storage, DRM/virtio-gpu display, VYOMA_DRAW framebuffer protocol.

## Roadmap

| Phase | Feature | Status |
|---|---|---|
| P01 | Reproducible builds (Makefile + Docker) | complete |
| P02 | Minimal allnoconfig kernel (virtio + 9P + DRM) | complete |
| P03 | Rust PID 1 supervisor | complete |
| P04 | Wasmtime WASI Preview 2 | complete |
| P05 | App manifest + capability model | complete |
| P06 | Multi-app concurrent scheduler + IPC broker | complete |
| P07 | 9P virtio persistent storage | complete |
| P08 | seccomp BPF denylist + capability audit log | complete |
| P09 | DRM/virtio-gpu display + VYOMA_DRAW protocol | complete |
| P10 | Embedded bitmap font + `draw_text` rendering | pending |
| P11 | virtio-net + WASI sockets + HTTP server app | pending |
| P12 | Interactive shell + keyboard routing + focus manager | pending |

Full implementation plans: [`.context/plans/plan-vyomaos/`](.context/plans/plan-vyomaos/README.md)

## Getting Started

```sh
# Prerequisites: Docker, QEMU
make          # build kernel + initramfs
make run      # boot in QEMU (serial output)
make run-gui DISPLAY_BACKEND=cocoa   # boot with virtio-gpu display (macOS)
make run-gui DISPLAY_BACKEND=sdl     # boot with virtio-gpu display (Linux)
```

## Guiding Principles

- **WASM-first**: applications are WebAssembly modules, not ELF binaries.
- **Capability-secure by default**: every app declares what it needs; everything else is inaccessible.
- **Minimal kernel**: allnoconfig base, only the drivers the system actually uses.
- **Reproducible**: deterministic builds via Docker; any engineer can reproduce the exact same image.
- **Scale up, not out**: the same architecture that boots in 18 MB today targets a full desktop OS tomorrow.

## See Also

- [Comparison matrix & performance tracker](docs/comparison-matrix.md) — VyomaOS vs Alpine, Flatcar, MirageOS, containers; phase-by-phase metrics
- [App manifest schema](docs/vyoma-manifest-schema.md) — capability manifest reference
- [Build system](base/README.md)
- [Apps](apps/README.md)
