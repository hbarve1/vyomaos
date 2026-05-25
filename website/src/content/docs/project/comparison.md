---
title: Comparison Matrix
description: How VyomaOS compares to Alpine, Docker, MirageOS, and traditional Linux.
---

VyomaOS occupies a unique position in the OS landscape — it's neither a traditional distribution nor a unikernel, but a WASM-native OS with capability security built into the runtime layer.

## Feature Comparison

| Feature | VyomaOS | Alpine Linux | Docker/OCI | MirageOS |
|---------|---------|-------------|------------|----------|
| **App runtime** | WASM/WASI P2 | Native ELF | Native ELF | OCaml/native |
| **Capability model** | Manifest-declared | File permissions | OCI (optional) | None |
| **Kernel size** | 2.3 MB | 5–10 MB | Host kernel | N/A (hypervisor) |
| **App portability** | Any lang → wasm32 | Arch-specific | Arch-specific | OCaml/C limited |
| **Attack surface** | Kernel + Wasmtime + Supervisor | Full userland | Full userland | Hypervisor only |
| **Binary determinism** | Byte-identical | Varies | Layer hash | Varies |
| **Boot time** | < 5 seconds | 2–10 seconds | N/A (container) | < 1 second |
| **Image size** | 18 MB | 5–8 MB | Varies | 1–10 MB |

## Security Comparison

| Aspect | VyomaOS | Traditional Linux | Docker |
|--------|---------|-------------------|--------|
| **Default access** | Deny-all | Allow-all | Host kernel access |
| **Isolation mechanism** | WASM sandbox + capabilities | Process + DAC | Namespaces + cgroups |
| **Filtering layers** | Not needed (absent = denied) | seccomp, AppArmor, SELinux | seccomp profiles |
| **Userland attack surface** | None (no C libs, no shell) | Full POSIX userland | Full container userland |
| **Binary format** | WASM (sandboxed by design) | ELF (native, unrestricted) | ELF (native) |

## When to Use VyomaOS

**VyomaOS is a good fit when:**

- You want apps sandboxed by default with no configuration
- You need deterministic, reproducible binary deployments
- You're building an appliance or embedded system where security matters
- You want to experiment with capability-based OS design
- You're interested in the future of WASM beyond the browser

**VyomaOS is not yet suitable for:**

- Production server workloads (networking is basic)
- Desktop daily-driver use (display system is framebuffer-based)
- Running existing Linux binaries (apps must target wasm32-wasip2)
- Hardware with proprietary drivers (minimal kernel support)

## Size Comparison

| Component | VyomaOS | Alpine | Ubuntu Minimal |
|-----------|---------|--------|---------------|
| Kernel | 2.3 MB | 5.5 MB | 11 MB |
| Init system | 697 KB (Rust) | ~500 KB (OpenRC) | ~1.5 MB (systemd) |
| Package manager | Built-in (WASM) | apk (~1 MB) | apt (~5 MB) |
| Shell | 1 KB (WASM) | ~800 KB (busybox) | ~1 MB (bash) |
| Typical app | 1–10 KB | 10 KB–100 MB | 10 KB–100 MB |
| Full image | 18 MB | 5–8 MB | 200+ MB |
