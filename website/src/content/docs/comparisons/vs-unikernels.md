---
title: VyomaOS vs Unikernels
description: Comparing VyomaOS's shared supervisor model with MirageOS, Unikraft, and other unikernels on compilation, flexibility, multi-app support, and debugging.
order: 3
---

# VyomaOS vs Unikernels (MirageOS / Unikraft)

Unikernels are specialized, single-purpose machine images that compile an application directly with a minimal OS library into a bootable image. MirageOS (OCaml), Unikraft (C/C++/Go), IncludeOS (C++), and OSv (JVM/C) are the most notable implementations. Each produces a lean VM image with no general-purpose OS overhead.

VyomaOS shares the unikernel philosophy of eliminating unnecessary OS layers, but takes a fundamentally different approach: instead of compiling one application per kernel, VyomaOS runs a shared Rust supervisor on a single minimal Linux kernel and executes 200+ concurrent WASM applications in isolated sandboxes.

## At a Glance

| Dimension | VyomaOS | MirageOS | Unikraft |
|---|---|---|---|
| **Model** | Shared supervisor, many WASM apps | One app compiled into one kernel | One app linked with library OS |
| **Kernel** | Linux 5.10 allnoconfig (2.3 MB) | None (runs on hypervisor) | Custom unikernel (libOS) |
| **Apps per instance** | 200+ concurrent | 1 | 1 |
| **App format** | `wasm32-wasip2` bytecode | Native OCaml / C | Native ELF (C/C++/Go/Rust) |
| **Language support** | Any with WASM target | OCaml primary, C possible | C, C++, Go, Rust, Python |
| **Image size** | 18 MB (full OS, all apps) | 1-10 MB per app-VM | 1-10 MB per app-VM |
| **Boot time** | < 5 s (full OS) | < 1 s (hypervisor) | < 1 s (hypervisor) |
| **Isolation** | WASM sandbox per app | Hypervisor per VM | Hypervisor per VM |
| **IPC** | Supervisor-brokered channels | Not applicable (single app) | Not applicable (single app) |
| **Hot update** | Swap WASM binary, restart app | Rebuild entire image | Rebuild entire image |
| **Debugging** | Standard tools (logs, inspector) | Limited (no OS, no shell) | Limited (no full OS) |
| **Maturity** | Alpha | Research-grade (since 2013) | Active development (since 2017) |

## Compilation Model

### Unikernels: One App, One Kernel

The unikernel model compiles the application, its dependencies, and a minimal OS library (networking, filesystem, memory management) into a single bootable image. There is no general-purpose OS, no shell, no other processes. The result is a tiny, fast-booting VM image purpose-built for one workload.

MirageOS takes this furthest: the entire network stack, TLS implementation, and storage drivers are OCaml libraries linked directly into the application. The image runs on a hypervisor (Xen, KVM, or Solo5) with no kernel at all.

Unikraft is more flexible, offering a library OS approach where you select which OS components to include (libc, network stack, filesystem) and link them with your application against a POSIX-compatible interface.

### VyomaOS: Shared Supervisor, Many Apps

VyomaOS keeps a real (but minimal) Linux kernel for hardware abstraction and runs a single Rust supervisor as PID 1. Applications are WASM bytecode executed by Wasmtime, not compiled into the kernel. This means:

- Adding a new app is deploying a `.wasm` file, not rebuilding the kernel.
- Multiple apps run concurrently with independent lifecycle management.
- The supervisor handles IPC, display, input routing, and process management.

The architectural difference is stark: a system with 10 services requires 10 unikernel VMs (each with its own hypervisor overhead) but only 1 VyomaOS instance.

## Multi-App Support

This is VyomaOS's primary structural advantage over unikernels.

### The Unikernel Problem

Unikernels are designed for single-purpose deployments. Running 10 microservices requires 10 VMs, each with:

- Its own boot process
- Its own memory allocation
- Its own network stack instance
- Its own hypervisor overhead (CPU rings, page tables, virtual devices)

Inter-service communication requires going through the hypervisor's virtual network, adding latency and complexity. There is no shared filesystem, no shared IPC, and no centralized management.

### VyomaOS: 200+ Apps, One Instance

VyomaOS runs all apps in a single OS instance. The supervisor provides:

- **Concurrent execution**: Each app gets its own Wasmtime instance and thread.
- **IPC**: Apps communicate via supervisor-brokered channels (`@appname: message`), avoiding network round-trips.
- **Shared resources**: One kernel, one display server, one input router, one network stack.
- **Centralized management**: `ps`, `kill`, `restart`, `log` commands for all apps from a single console.

For a system with many cooperating services, this eliminates enormous infrastructure complexity.

## Language Flexibility

| Language | VyomaOS | MirageOS | Unikraft |
|---|---|---|---|
| Rust | Yes (primary) | Limited | Yes |
| C / C++ | Yes (via wasi-sdk) | Limited | Yes (primary) |
| Go | Yes (TinyGo / Go 1.21+) | No | Yes |
| Python | Yes (via Componentize-py) | No | Yes (via unikernel Python) |
| JavaScript | Yes (via wasm runtimes) | No | Yes (via QuickJS) |
| OCaml | Yes (via wasm target) | Yes (primary) | No |
| Swift | Yes (via SwiftWasm) | No | No |
| Kotlin | Yes (via Kotlin/Wasm) | No | No |

MirageOS is deeply tied to OCaml. Its strength (type-safe OS libraries) is also its limitation: you must write your application in OCaml or wrap C libraries carefully. The OCaml ecosystem for systems programming is small.

Unikraft supports more languages through POSIX compatibility, but each language runtime must be ported to the library OS environment, which is ongoing work.

VyomaOS supports any language that can compile to `wasm32-wasip2`. As the WASI ecosystem matures, this covers most mainstream languages without per-language OS porting effort.

## Image Size

Unikernels excel at single-app image size:

| Scenario | MirageOS | Unikraft | VyomaOS |
|---|---|---|---|
| Single HTTP server | 1-5 MB (one VM) | 2-8 MB (one VM) | 18 MB (full OS) |
| 10 microservices | 10-50 MB (10 VMs) | 20-80 MB (10 VMs) | 18 MB (full OS, same image) |
| 50 services | 50-250 MB (50 VMs) | 100-400 MB (50 VMs) | 18 MB (full OS + ~5 MB of .wasm files) |

For a single service, unikernels are smaller. But as the number of services grows, VyomaOS's shared-OS model amortizes the base image cost across all apps. At 10+ services, VyomaOS is typically smaller in total footprint.

## Debugging and Development

### Unikernels: Hard to Debug

Debugging unikernels is notoriously difficult:

- No shell to log into.
- No `strace`, `gdb`, or standard Linux debugging tools.
- Crashes often produce only a register dump from the hypervisor.
- Reproducing issues requires rebuilding the image with debug symbols.
- No runtime inspection or hot-patching.

MirageOS has some tooling (mirage-trace, Solo5 debugging), but it is far from the Linux ecosystem's maturity.

### VyomaOS: Standard Debugging Workflow

VyomaOS apps produce stdout/stderr that the supervisor captures:

- `log <appname>` prints the app's output history.
- `logf <appname>` tails output in real time.
- Apps can be killed and restarted individually without rebooting.
- The supervisor itself runs on a real Linux kernel, so kernel-level debugging tools work.
- WASM modules can be inspected with standard WASM tooling (wasm-tools, wasm2wat).

The development cycle is: edit code, compile to `.wasm`, deploy to running system (or restart app). No image rebuild, no VM reboot.

## Security Comparison

Both approaches reduce attack surface compared to a full Linux distribution, but through different mechanisms:

| Security Property | VyomaOS | Unikernels |
|---|---|---|
| Attack surface | Kernel + Wasmtime + supervisor | Hypervisor only |
| App-to-app isolation | WASM sandbox per app | Hypervisor per VM |
| Memory safety | WASM bounds checking | Language-dependent (OCaml: yes, C: no) |
| Capability model | Manifest-declared, per-app | All-or-nothing (single app = full access) |
| Kernel CVE exposure | Minimal kernel, but present | None (no kernel) |

Unikernels have a smaller trusted computing base for single-app deployments -- there is no kernel to exploit. But they provide no isolation between components within a single image. If the unikernel application has a vulnerability, the attacker has full control of the VM.

VyomaOS's WASM sandbox provides per-app isolation within a shared OS, which is more relevant for multi-service deployments.

## When to Choose Unikernels

- You are deploying a single, well-defined network service (DNS, TLS terminator, load balancer).
- You need the absolute smallest possible trusted computing base.
- Your workload is written in OCaml (MirageOS) or C/C++ (Unikraft) and will not change languages.
- You have hypervisor infrastructure (Xen, KVM) already deployed.
- Boot time under 100ms matters (unikernels boot faster than VyomaOS).

## When to Choose VyomaOS

- You need multiple cooperating services on a single device or VM.
- You want to deploy apps in multiple languages without per-language OS porting.
- You need runtime app management (restart, update, inspect) without image rebuilds.
- You want IPC between services without network round-trips.
- Your target includes embedded devices, kiosks, or edge nodes where running many VMs is impractical.
- Developer experience matters: you want familiar debugging tools and fast iteration.

## Honest Assessment

Unikernels are an elegant solution for single-purpose VMs, and MirageOS's type-safe approach to OS libraries is genuinely innovative. For narrow, well-defined workloads on hypervisor infrastructure, unikernels deliver the smallest possible attack surface.

However, unikernels have struggled to achieve mainstream adoption after over a decade of development. The debugging difficulty, single-language constraints, and one-app-per-VM limitation have kept them niche.

VyomaOS is younger and less mature, but its multi-app shared-supervisor model addresses the practical limitations that have held unikernels back. The question is whether WASM sandboxing provides sufficient isolation to substitute for hypervisor-level separation. For most threat models, the answer is yes -- WASM's memory safety guarantees are strong, and the capability manifest system provides fine-grained access control that unikernels lack entirely.
