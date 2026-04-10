# Implementation Plan — vyomaos

## Goal

Build a minimal, production-quality OS that uses a Linux kernel for hardware abstraction and Wasmtime (WASI Preview 2) as the sole application platform. Progress from the current bash-scripted prototype to a Rust-supervised, capability-secure, multi-app WASM OS. Phases 1–4 constitute the minimal working prototype: a bootable system with a real Rust PID 1 supervisor executing real WASI apps.

## Phases

| # | Phase | Status | Tasks |
|---|---|---|---|
| [01 — Build Foundation](phases/phase-01-build-foundation/README.md) | Reproducible, incremental builds via Makefile + Docker | **complete** | 3 |
| [02 — Kernel Hardening](phases/phase-02-kernel-hardening/README.md) | Minimal allnoconfig kernel with virtio + 9p + DRM | **complete** | 3 |
| [03 — Rust Supervisor](phases/phase-03-rust-supervisor/README.md) | Rust PID 1 that mounts filesystems + discovers + runs apps | **complete** | 4 |
| [04 — WASM Runtime](phases/phase-04-wasm-runtime/README.md) | Real Wasmtime static binary with WASI Preview 2 | **complete** | 3 |
| [05 — App Model](phases/phase-05-app-model/README.md) | wasm32-wasip2 apps + capability manifests + config-driven boot | **complete** | 3 |
| [06 — Multi-App & IPC](phases/phase-06-multi-app-ipc/README.md) | Concurrent apps, restart policies, supervisor IPC broker (P06T01–02 done; P06T03–04 deferred) | **partial** | 4 |
| [07 — Networking & Storage](phases/phase-07-networking-storage/README.md) | 9P virtio persistent storage done; virtio-net + sockets pending | **partial** | 4 |
| [08 — Observability & Security](phases/phase-08-observability-security/README.md) | seccomp BPF denylist + capability audit log done; namespaces + signing pending | **partial** | 4 |
| [09 — GUI Display](phases/phase-09-gui-display/README.md) | DRM/virtio-gpu + fbcon + VYOMA_DRAW framebuffer protocol + gui-demo | **complete** | 5 |
| [10 — Text Rendering](phases/phase-10-text-rendering/README.md) | Embedded 8×16 bitmap font, `draw_text` VYOMA_DRAW command, labelled gui-demo dashboard | **complete** | 3 |
| [11 — Networking](phases/phase-11-networking/README.md) | virtio-net + WASI sockets + `http-server` WASM app serving live status page at localhost:8080 | **complete** | 4 |
| [12 — Interactive Shell](phases/phase-12-interactive-shell/README.md) | `/dev/tty0` keyboard routing, focus manager, `@supervisor:` commands, `shell` WASM app | **complete** | 4 |

## Constraints

- Kernel: Linux 5.10.x LTS, `allnoconfig` base (not tinyconfig — tinyconfig silently drops forced deps), no loadable modules, static build
- Runtime: Wasmtime 43.0.0 **glibc** variant (not musl — musl build unavailable); glibc runtime bundled in initramfs
- Supervisor: Rust, static musl binary, replaces BusyBox shell as PID 1
- Apps: `wasm32-wasip2` target (WASI Preview 2), zero glibc dependencies
- Build: Reproducible via Docker; incremental via Makefile dependency tracking
- Boot time target: < 5 seconds in QEMU on standard laptop hardware
- Initramfs size target: < 25 MB (Wasmtime 43 glibc variant dominates at 61M stripped)

## References

- [Project README](../../README.md)
- [Base build system](../../base/README.md)
- [Apps README](../../apps/README.md)
- [Wasmtime WASI docs](../../.tessl/tiles/tessl/pypi-wasmtime/docs/wasi.md)
