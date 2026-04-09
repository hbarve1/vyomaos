# Implementation Plan — vyomaos

## Goal

Build a minimal, production-quality OS that uses a Linux kernel for hardware abstraction and Wasmtime (WASI Preview 2) as the sole application platform. Progress from the current bash-scripted prototype to a Rust-supervised, capability-secure, multi-app WASM OS. Phases 1–4 constitute the minimal working prototype: a bootable system with a real Rust PID 1 supervisor executing real WASI apps.

## Phases

| # | Phase | Status | Tasks |
|---|---|---|---|
| [01 — Build Foundation](phases/phase-01-build-foundation/README.md) | Reproducible, incremental builds via Makefile + Docker | pending | 3 |
| [02 — Kernel Hardening](phases/phase-02-kernel-hardening/README.md) | Minimal tinyconfig kernel with virtio + 9p | pending | 3 |
| [03 — Rust Supervisor](phases/phase-03-rust-supervisor/README.md) | Rust PID 1 that mounts filesystems + discovers + runs apps | pending | 4 |
| [04 — WASM Runtime](phases/phase-04-wasm-runtime/README.md) | Real Wasmtime static binary with WASI Preview 2 | pending | 3 |
| [05 — App Model](phases/phase-05-app-model/README.md) | wasm32-wasip2 apps + capability manifests + config-driven boot | pending | 3 |
| [06 — Multi-App & IPC](phases/phase-06-multi-app-ipc/README.md) | Concurrent apps, restart policies, Component Model typed IPC | pending | 4 |
| [07 — Networking & Storage](phases/phase-07-networking-storage/README.md) | virtio-net + WASI sockets, virtio-blk + WASI filesystem | pending | 4 |
| [08 — Observability & Security](phases/phase-08-observability-security/README.md) | Structured logging, seccomp, Linux namespaces, WASM signing | pending | 4 |
| [09 — GUI Display](phases/phase-09-gui-display/README.md) | DRM/virtio-gpu framebuffer, `vyoma:display` WIT host interface, WASM drawing API | pending | 5 |

## Constraints

- Kernel: Linux 5.10.x LTS, `tinyconfig` base, no loadable modules, static build
- Runtime: Wasmtime statically linked against musl libc
- Supervisor: Rust, static musl binary, replaces BusyBox shell as PID 1
- Apps: `wasm32-wasip2` target (WASI Preview 2), zero glibc dependencies
- Build: Reproducible via Docker; incremental via Makefile dependency tracking
- Boot time target: < 5 seconds in QEMU on standard laptop hardware
- Initramfs size target: < 10 MB (excluding kernel)

## References

- [Project README](../../README.md)
- [Base build system](../../base/README.md)
- [Apps README](../../apps/README.md)
- [Wasmtime WASI docs](../../.tessl/tiles/tessl/pypi-wasmtime/docs/wasi.md)
