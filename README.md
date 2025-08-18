# Vyoma OS

A minimal operating system that pairs a Linux kernel with a WebAssembly (WASM) runtime to run applications. Vyoma OS explores what an OS looks like when WASM is the primary application platform: portable, safe by default, and small.

## Idea

- Use the Linux kernel for hardware, drivers, and process isolation.
- Keep user space minimal; let the WASM runtime be the application platform.
- Treat apps as WASM modules with clear capabilities and stable interfaces.
- Prefer simplicity and determinism over feature breadth.

## Why WebAssembly

- Portability: run the same module across environments and hardware.
- Safety: strong sandboxing and capability-driven access.
- Small footprint: minimal user space and simple distribution.
- Clear interfaces: WASI and host shims define predictable behavior.

## Guiding Principles

- Minimal first: build the smallest useful system, then extend.
- WASM-first: applications are WebAssembly modules, not ELF binaries.
- Composable: features arrive as opt-in modules, not a monolith.
- Reproducible: deterministic builds and predictable boot.
- Observable: introspection without heavyweight agents.
- Open and educational: simple enough to learn from.

## Concept (high level)

Linux kernel → minimal userspace (init) → WASM runtime → WASM apps

## Roadmap

- MVP
  - Boot Linux, start a WASM runtime, run a single module at boot.
  - Basic stdio, args/env, and exit codes via WASI.
- Short term
  - Choose and stabilize a runtime (e.g., Wasmtime/Wasm3) with WASI Preview 2.
  - Simple app handoff and lifecycle (start/stop, restart on failure).
  - Config-driven selection of the module to run at boot.
- Mid term
  - Multi-app supervisor with simple scheduling and isolation.
  - Capability model for filesystem, network, and clock access.
  - Minimal IPC between modules (channels or message passing).
  - Basic networking and optional persistent storage.
- Long term
  - Package format and registry for WASM apps.
  - Observability (logs, metrics, traces) with minimal overhead.
  - Resource controls (CPU/memory quotas) and cgroup integration.
  - Security hardening (seccomp, namespaces, signing/attestation).
  - Optional accelerators (e.g., WASI-NN) for specialized workloads.

## Use Cases

- Education: a clear, minimal stack to learn modern OS + WASM concepts.
- Edge/embedded: small footprint, safe execution of portable modules.
- Deterministic compute: reproducible tasks and batch jobs.
- Research: a sandbox to explore WASI, capabilities, and isolation.

## Non‑Goals (for now)

- Full desktop environment or general-purpose distribution.
- POSIX completeness in user space.
- Shipping a broad set of kernel drivers beyond what’s needed to boot.

## How to Engage

Feedback, ideas, and design discussions are welcome. The aim is to evolve a simple, understandable WASM-first OS together with the community.
