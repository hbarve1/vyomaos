---
title: Security Model
description: Four layers of defense — without the complexity.
---

VyomaOS achieves strong security through a simple, layered approach. Instead of filtering dangerous operations, it prevents them from existing in the first place.

## Design Principle

> **Capabilities not declared in the manifest are never wired up — no filtering layer needed.**

Traditional OS security works by granting broad access and then restricting it (allow-by-default). VyomaOS inverts this: apps get nothing unless explicitly declared (deny-by-default). There's no filter to bypass because the interfaces simply don't exist.

## Four Security Layers

### Layer 1: WASM Sandbox

Every app runs inside Wasmtime's WebAssembly sandbox:

- **Memory isolation**: Each app gets its own linear memory. No raw pointers, no address space sharing.
- **Control flow integrity**: Execution follows the WASM control flow graph. No ROP/JOP gadgets.
- **Type safety**: All function calls are type-checked at the bytecode level.
- Stack overflows, buffer overruns, use-after-free — eliminated structurally.

### Layer 2: WASI Capability Model

Each app declares capabilities in its `vyoma.toml` manifest:

```toml
[capabilities]
stdio      = true    # stdin/stdout/stderr
filesystem = true    # mount /data (9P)
network    = true    # WASI sockets (TCP)
display    = true    # framebuffer + VYOMA_DRAW
shell      = true    # @supervisor: commands
mouse      = true    # mouse input events
```

The supervisor reads this manifest and wires up **only** the declared WASI imports when spawning the Wasmtime process. If an app doesn't declare `network = true`, there is no TCP socket interface to call — not even a filtered one.

### Layer 3: Seccomp BPF Denylist

The supervisor applies a seccomp BPF denylist to all Wasmtime child processes. Even if Wasmtime were somehow compromised, dangerous syscalls are blocked at the kernel level.

This is a defense-in-depth measure — the WASM sandbox and capability model should prevent any need for it, but it provides an additional barrier.

### Layer 4: No C Userland

VyomaOS has no shell, no libc, no interpreters, no POSIX environment variables, no `/proc` filesystem. The entire attack surface is:

- Linux kernel (minimal allnoconfig build)
- Wasmtime runtime
- Rust supervisor

That's it. No shared libraries, no dynamic linker, no script interpreters, no package managers running as root. Every traditional userland attack vector is absent.

## Capability Reference

| Capability | What it provides | What's absent without it |
|-----------|-----------------|------------------------|
| `stdio` | stdin/stdout/stderr | No console I/O |
| `filesystem` | Mount `/data` (9P, persistent) | No file access |
| `network` | WASI sockets (TCP) | No network interface |
| `display` | Framebuffer + VYOMA_DRAW | No screen output |
| `shell` | `@supervisor:` commands | No process management |
| `mouse` | `VYOMA_INPUT:mouse:` events | No mouse input |
| `watchdog_secs` | Supervisor kills if silent N seconds | No watchdog |

## Comparison with Other Approaches

| Approach | VyomaOS | Linux (traditional) | Docker/OCI | Android |
|----------|---------|-------------------|------------|---------|
| Default access | Deny-all | Allow-all | Host kernel | Managed runtime |
| Filtering | Not needed | seccomp, AppArmor, SELinux | seccomp, namespaces | SELinux, permissions |
| Binary format | WASM (sandboxed) | ELF (native) | ELF (native) | DEX + native |
| Capability granularity | Per-interface | File-level | Container-level | Permission groups |
