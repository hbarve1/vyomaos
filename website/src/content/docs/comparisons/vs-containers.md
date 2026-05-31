---
title: VyomaOS vs Linux Containers
description: Comparing VyomaOS's WASM-first isolation with Docker, Podman, and Kubernetes container-based approaches to security, size, and portability.
order: 2
---

# VyomaOS vs Linux Containers (Docker / Podman / Kubernetes)

Linux containers (Docker, Podman, containerd, Kubernetes) are the dominant deployment model for cloud-native applications. They use Linux kernel features -- namespaces, cgroups, seccomp, and overlay filesystems -- to provide process isolation while sharing a single host kernel.

VyomaOS provides isolation through a fundamentally different mechanism: WebAssembly sandboxing with capability-declared manifests. Instead of carving up a shared kernel's resources, VyomaOS runs each application in a WASM sandbox where undeclared capabilities simply do not exist.

## At a Glance

| Dimension | VyomaOS | Linux Containers |
|---|---|---|
| **Isolation mechanism** | WASM sandbox + capability manifest | Namespaces + cgroups + seccomp |
| **Kernel** | Dedicated minimal kernel (2.3 MB) | Shared host kernel |
| **Runtime** | Wasmtime (WASI Preview 2) | containerd / CRI-O / runc |
| **App format** | `wasm32-wasip2` bytecode (KB) | OCI image layers (MB-GB) |
| **Typical app size** | 1-150 KB | 50 MB - 1 GB+ |
| **Startup time** | Sub-second (after runtime init) | Sub-second (kernel already running) |
| **Boot time** | < 5 s (full OS) | N/A (host already running) |
| **CPU overhead** | WASM JIT compilation | Near-native (shared kernel) |
| **Memory overhead** | Wasmtime instance per app | Kernel structures per namespace |
| **Security boundary** | WASM memory safety + capabilities | Kernel namespace isolation |
| **Portability** | Cross-architecture (wasm32) | Per-architecture (amd64/arm64 images) |
| **Orchestration** | Supervisor built-in | Kubernetes / Docker Compose / Nomad |
| **Networking** | Per-app capability-gated | Full network stack per container |
| **Maturity** | Alpha | Production (industry standard) |

## Isolation Model

### Containers: Kernel Feature Partitioning

Linux containers do not use a hypervisor or a separate kernel. Instead, they partition the host kernel's resources using:

- **PID namespaces**: Each container sees its own process tree.
- **Network namespaces**: Each container gets its own network stack.
- **Mount namespaces**: Each container has its own filesystem view.
- **cgroups**: CPU, memory, and I/O limits per container.
- **seccomp**: System call filtering to reduce kernel attack surface.

This is effective but the isolation depends entirely on kernel correctness. A kernel vulnerability can break out of any container. The attack surface includes every syscall the container is permitted to make, and the default seccomp profile still allows ~300+ syscalls.

### VyomaOS: WASM Sandbox + Capability Declaration

VyomaOS isolates apps at the WebAssembly level. Each app runs in a Wasmtime instance with:

- **Memory safety**: WASM enforces bounds-checked linear memory. No buffer overflows, no use-after-free, no raw pointer access.
- **No direct syscalls**: Apps interact only through WASI imports explicitly wired by the supervisor.
- **Capability manifests**: The `vyoma.toml` file declares what the app can access. Undeclared capabilities are not available -- there is no interface to call, no syscall to attempt.

The kernel is still in the stack, but apps never interact with it directly. The attack surface is reduced to Wasmtime's WASI implementation, which exposes a much smaller API than the Linux syscall interface.

### Key Difference

Containers start with full access and subtract (seccomp denylists, dropped capabilities, read-only filesystems). VyomaOS starts with no access and adds only what is declared (capability manifests). This is the difference between a denylist and an allowlist approach to security.

## Image Size

This is where VyomaOS has a dramatic advantage at the application level.

| Example | Container Image | VyomaOS WASM |
|---|---|---|
| Hello world | ~5 MB (Alpine-based) | 85 KB |
| HTTP server | ~15 MB (Alpine + deps) | ~120 KB |
| Calculator app | ~10 MB (Alpine + deps) | 113 KB |
| Web application | 200 MB - 1 GB+ | 50-500 KB |
| Base runtime | N/A (host kernel) | 18 MB (full OS image) |

A typical container image includes an OS base layer (Alpine ~5 MB, Ubuntu ~30 MB, Debian ~50 MB), shared libraries, and the application binary. Even "distroless" images are several MB.

VyomaOS WASM apps are pure bytecode with no OS layer, no shared libraries, no filesystem hierarchy. The trade-off is that the Wasmtime runtime itself is 61 MB (included once in the OS image, shared across all apps).

For deploying many small services, VyomaOS's model is far more space-efficient. For deploying a few large applications with complex native dependencies, containers are more practical today.

## Startup Time

Container startup (creating namespaces, mounting overlayfs, starting the entrypoint) takes milliseconds to low seconds. The host kernel is already running, so there is no boot cost.

VyomaOS app startup within a running system is also sub-second. Wasmtime JIT-compiles the WASM module on first load. However, if you are comparing "time to first request," you must include VyomaOS's full boot time (< 5 seconds in QEMU), since it is a standalone OS.

For latency-critical cold-start scenarios (serverless, FaaS), WASM modules have an advantage: a 100 KB module compiles and starts faster than inflating a 200 MB container image and launching a JVM or Python interpreter.

## Attack Surface

| Layer | Containers | VyomaOS |
|---|---|---|
| Kernel | Shared host kernel (full syscall surface) | Minimal kernel (allnoconfig, 2.3 MB) |
| Runtime | containerd + runc (~20 MB) | Wasmtime (~61 MB) |
| Init | containerd-shim | Rust supervisor (~700 KB) |
| System libraries | libc + deps per container | None (apps have no libc) |
| Shell | Often included (sh, bash) | None |
| Package manager | Often included (apt, apk) | None in base |
| App dependencies | Bundled in image layers | Compiled into WASM bytecode |
| Network stack | Full per-container | Per-app capability-gated |

Containers inherit the host kernel's full attack surface. Every container shares the same kernel, and kernel CVEs affect all containers on the host. Container escapes (e.g., CVE-2019-5736 in runc, CVE-2024-21626 in runc) have been demonstrated repeatedly.

VyomaOS's attack surface is narrower: the minimal kernel, Wasmtime, and the supervisor. Apps cannot escape the WASM sandbox without a bug in Wasmtime itself. There is no shared libc, no shell to inject into, and no package manager to exploit.

## Portability

Container images are architecture-specific. An `amd64` image does not run on `arm64` without emulation (QEMU user-mode) or multi-arch manifests. Building for multiple architectures requires cross-compilation or multi-platform build pipelines.

VyomaOS WASM apps are architecture-independent by design. A `wasm32-wasip2` binary compiled on an x86_64 developer machine runs identically on an ARM64 edge device, an x86 server, or an MCU (via the wasm3 interpreter). No recompilation, no multi-arch builds.

## Orchestration

Kubernetes, Docker Compose, Nomad, and other orchestrators provide sophisticated scheduling, service discovery, load balancing, and rolling updates for containers. This ecosystem is mature and battle-tested at enormous scale.

VyomaOS's supervisor handles app lifecycle (start, stop, restart, watchdog) for apps on a single node. It includes IPC brokering and capability enforcement but does not provide distributed orchestration. For multi-node deployments, VyomaOS would need to integrate with external orchestration (this is planned but not yet implemented).

## When to Choose Containers

- You are deploying at scale with Kubernetes or similar orchestration.
- Your applications have complex native dependencies that cannot compile to WASM.
- You need the mature ecosystem of container tooling (CI/CD, registries, monitoring).
- Your team is experienced with container workflows.
- You need production stability today.

## When to Choose VyomaOS

- You are building single-purpose appliances or edge devices.
- You want structurally stronger isolation than kernel namespaces provide.
- Your apps can compile to `wasm32-wasip2` and you want KB-sized deployments.
- You need cross-architecture portability without multi-arch build pipelines.
- You want a deny-by-default security model without configuring seccomp profiles.
- Cold-start latency matters and you want sub-millisecond module load times.

## Resource Efficiency at Scale

For microservices architectures, the cumulative overhead matters. Consider deploying 20 services:

| Resource | 20 Containers | 20 VyomaOS Apps |
|---|---|---|
| Total image storage | 1-10 GB (20 images) | 18 MB + ~2 MB of .wasm files |
| Memory overhead (runtime) | 20 container shims + namespaces | 1 supervisor + 20 Wasmtime instances |
| Kernel structures | 20 sets of namespace data | 1 set (shared kernel) |
| Network stacks | 20 virtual NICs + bridges | 1 network stack, per-app capability gating |
| Filesystem layers | 20 overlay mounts | 1 filesystem, per-app sandboxed access |
| Process trees | 20+ processes per container | 1 supervisor process + internal threads |

Containers were designed for isolation, not efficiency. Each container carries its own filesystem view, network stack, and process namespace. VyomaOS amortizes all OS overhead across apps.

The counter-argument is that container orchestrators optimize for this at scale with shared base layers and resource limits. But the structural overhead of per-container namespaces remains.

## Hybrid Approaches

VyomaOS and containers are not mutually exclusive. VyomaOS can run inside a VM managed by Kubernetes (similar to Kata Containers or Firecracker). In this model, Kubernetes handles orchestration while VyomaOS provides the runtime isolation within each node.

The WASM ecosystem is also being adopted within the container world. Projects like Spin, wasmCloud, and Fermyon bring WASM workloads to Kubernetes. VyomaOS differs by making WASM the entire OS runtime, not just a workload type alongside containers.

## Honest Assessment

Linux containers are the industry standard for application deployment. The tooling, ecosystem, and operational knowledge are unmatched. For most production workloads in 2026, containers are the pragmatic choice.

VyomaOS offers a cleaner security model and dramatically smaller application footprints, but it is an alpha-stage OS with a fraction of the ecosystem. Its advantage is architectural: the isolation model is fundamentally stronger because it does not depend on kernel namespace correctness. Whether that architectural advantage translates to practical superiority depends on the maturity of the WASI ecosystem and your specific deployment requirements.
