---
title: Introduction
description: What is VyomaOS and why does it exist?
---

VyomaOS is a **WASM-first operating system** with the long-term goal of becoming a lightweight but fully capable general-purpose OS built from the ground up on a capability-secure WebAssembly foundation.

## The Problem

Modern OSes carry 40 years of legacy attack surface:

- **C Userland**: Shared libraries, POSIX quirks, shell injection. Every app inherits all of it.
- **Coarse Permissions**: Android/Linux DAC — either you have access, or you don't. No fine-grained capability model.
- **Non-Deterministic Binaries**: ELF binaries vary by libc/arch. No reproducibility guarantee. Supply chain attacks thrive.

## The VyomaOS Approach

VyomaOS starts with a single rule: **the runtime IS the OS boundary**.

- The **Linux kernel** handles hardware, drivers, and process isolation. Nothing else.
- Every application is a **`wasm32-wasip2` binary**. No native userland, no shell, no C runtime exposed to apps.
- A **Rust PID 1 supervisor** manages app lifecycle, IPC, and capability enforcement.
- Capabilities (filesystem, network, display, stdio) are **declared per-app in a manifest** and enforced at boot. Undeclared capabilities are not filtered — they are never wired up.

## Why WebAssembly?

| Property | Benefit |
|----------|---------|
| **Portability** | The same `.wasm` binary runs identically on any VyomaOS instance, any architecture |
| **Safety** | Strong sandbox; no app can access resources not explicitly granted in its manifest |
| **Language-agnostic** | Rust, Go, C, Swift, Python, JS/TS — any language with a WASM target works |
| **Small footprint** | Apps are 1–10 KB. No shared library sprawl |
| **Determinism** | WASM bytecode is byte-identical across builds and hosts |

## Current State

VyomaOS boots in QEMU in under 5 seconds to a Rust supervisor running 200+ concurrent WASM apps with:

- macOS-like desktop: Menu Bar, Dock, Spotlight, App Switcher, Mission Control
- Interactive shell with process management
- Bidirectional IPC broker
- DRM/virtio-gpu display at 1440x900
- Window management with focus, drag, and keyboard routing
- HTTP server with WASI sockets
- Persistent storage via 9P virtio
- seccomp BPF security hardening

The architecture scales from an 18 MB embedded appliance today to a full desktop OS tomorrow, with the same security model at every scale.
