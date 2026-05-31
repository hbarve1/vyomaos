---
title: Why WebAssembly as the OS Application Model
description: A deep technical analysis of why VyomaOS chose WebAssembly (wasm32-wasip2) as its universal application binary format — covering sandboxing, portability, determinism, and the trade-offs involved.
order: 1
---

# Why WebAssembly as the OS Application Model

Operating systems have shipped native binaries for fifty years. ELF on Linux, PE on Windows, Mach-O on macOS — each format is architecture-specific, links against platform-specific libraries, and runs with the full privilege of its user account. VyomaOS breaks from this model entirely. Every application is a WebAssembly binary compiled to the `wasm32-wasip2` target, executed by a sandboxed runtime under supervisor control.

This document explains why.

---

## The Problem with Native Binaries

Native executable formats carry three fundamental problems that no amount of tooling has solved in fifty years of trying.

### Architecture Lock-In

An ELF binary compiled for x86-64 cannot run on ARM64. An ARM64 binary cannot run on RISC-V. This means every OS distribution must maintain separate builds for every architecture it supports. Debian maintains builds for amd64, arm64, armhf, i386, mips64el, ppc64el, riscv64, and s390x. That is eight complete rebuilds of over 50,000 packages, each producing different byte sequences from the same source code.

VyomaOS targets six hardware platforms — from ARM Cortex-M4 microcontrollers to x86-64 workstations. Maintaining native binaries for each would mean six separate toolchains, six CI pipelines, and six sets of bugs that only reproduce on specific architectures. WebAssembly eliminates this entirely: one `.wasm` file runs on all six.

### Ambient Authority

A native binary inherits the full authority of the user that launched it. If you run a text editor as your user account, that text editor can read your SSH keys, exfiltrate your browser cookies, and write to any file you own. The binary does not declare what it needs — it gets everything.

Operating systems have layered mitigations on top: file permissions (DAC), mandatory access controls (SELinux, AppArmor), seccomp-bpf syscall filtering, namespaces, and containers. Each adds complexity. Each has had privilege escalation vulnerabilities. The fundamental problem remains: native binaries start with full access and must be constrained after the fact.

### Non-Deterministic Builds

Compile the same C program on two machines with the same compiler version and flags. The resulting binaries will differ. Timestamps embedded in object files, address space layout differences, link ordering variations, and compiler-internal state all contribute. This makes reproducible builds an ongoing research project rather than a solved problem.

---

## Why WebAssembly

WebAssembly was designed as a compilation target for the browser. Its design constraints — sandboxing, portability, and determinism — turn out to be exactly what an OS application model needs.

### Sandboxed by Default

A WebAssembly module has no capabilities at load time. It cannot access the filesystem. It cannot open network connections. It cannot read environment variables. It cannot even read the system clock. Every capability must be explicitly provided by the host runtime through imported functions.

This is not a policy layered on top of an existing execution model. It is the execution model. The WebAssembly specification defines a Harvard architecture with separate code and data spaces, linear memory that cannot overflow into host memory, and a call stack that cannot be corrupted by buffer overflows. There is no `mmap`, no `dlopen`, no `exec`, no way to escape the sandbox without the host explicitly providing an escape hatch.

```
Native binary:  starts with everything, must be constrained
WASM module:    starts with nothing, must be granted capabilities
```

### Portable Across Architectures

WebAssembly defines a single 32-bit virtual instruction set (with a 64-bit extension in progress). A `.wasm` file compiled on an x86-64 workstation runs identically on an ARM64 Raspberry Pi, an ARM Cortex-M4 microcontroller, or a RISC-V board. The runtime (Wasmtime, wasm3, WAMR) handles translation to native code on each platform.

VyomaOS exploits this directly. The same `hello-world.wasm` binary that runs on the desktop profile runs on the IoT edge profile and the MCU minimal profile. No recompilation. No cross-compilation toolchains. No architecture-specific bugs.

### Deterministic Compilation

Compile a Rust program to `wasm32-wasip2` on any machine with the same compiler version. The resulting `.wasm` file is byte-identical. This is not aspirational — it is a property of the compilation target. WebAssembly has no address space layout to vary, no timestamps to embed, and no platform-specific linking decisions to make.

For VyomaOS, this means:

- **Reproducible builds**: any contributor can verify that a `.wasm` binary matches its source code
- **Content-addressable caching**: SHA-256 of the `.wasm` file uniquely identifies a build
- **Simplified distribution**: no need to trust a build farm when the output is deterministic

### Small Binary Size

WebAssembly binaries are compact. A typical VyomaOS app compiles to 1-30 KB of `.wasm`. The `hello-world` app is under 2 KB. Even complex apps like the GUI dashboard with framebuffer rendering stay under 50 KB. Compare this to a statically-linked Rust binary for x86-64, which starts at 300+ KB for a trivial program.

This matters for embedded targets. The MCU minimal profile has 128 KB of RAM. Native binaries simply do not fit. WebAssembly binaries do.

---

## WASI Preview 2: Standardized System Interfaces

Raw WebAssembly provides computation but no I/O. The WebAssembly System Interface (WASI) standardizes how modules interact with the outside world.

WASI Preview 2 (the version VyomaOS targets with `wasm32-wasip2`) introduces the component model, which defines:

- **Typed interfaces**: function signatures with rich types (strings, lists, records, variants), not just integers
- **World definitions**: a manifest of which interfaces a component imports and exports
- **Composability**: components can be linked together, with one component's exports satisfying another's imports

VyomaOS apps compile against WASI Preview 2 interfaces for:

| Interface | Purpose |
|-----------|---------|
| `wasi:io/streams` | Stdin/stdout byte streams |
| `wasi:filesystem/types` | File and directory operations |
| `wasi:sockets/tcp` | TCP network connections |
| `wasi:clocks/wall-clock` | Current time |
| `wasi:random/random` | Cryptographic random bytes |

The supervisor selectively wires up these interfaces based on the app's `vyoma.toml` manifest. An app that declares `stdio = true` but not `filesystem = true` gets `wasi:io/streams` but not `wasi:filesystem/types`. The interface is simply absent — there is nothing to call.

```toml
# vyoma.toml — app declares only what it needs
[capabilities]
stdio = true
filesystem = true
# network is not declared — wasi:sockets/tcp is not wired up
```

---

## Trade-Offs

WebAssembly is not a free lunch. VyomaOS accepts specific trade-offs in exchange for the security and portability benefits.

### No Raw Syscalls

WASM modules cannot issue raw Linux syscalls. Every interaction with the host goes through WASI interfaces mediated by the runtime. This means:

- **No `mmap` for memory-mapped I/O**: apps use linear memory, which the runtime manages
- **No `ioctl` for device control**: hardware access goes through supervisor-provided interfaces
- **No `fork`/`exec`**: process creation is a supervisor responsibility

For VyomaOS, this is a feature, not a limitation. The supervisor is the only process that talks to the kernel. Apps cannot bypass it.

### Runtime Overhead

WebAssembly is not native code. Even with JIT compilation (Wasmtime on desktop/mobile/server profiles) or AOT compilation (WAMR on IoT/robotics profiles), there is overhead:

- **JIT compilation latency**: Wasmtime compiles WASM to native code at load time. For small apps (1-30 KB), this takes under 10ms. For larger apps, it can take 50-100ms.
- **Indirect call overhead**: WASM function calls through tables (used for virtual dispatch) are slower than native direct calls due to bounds checking.
- **Memory bounds checking**: every linear memory access includes a bounds check. Modern runtimes use guard pages to make this near-zero-cost on 64-bit platforms, but it remains measurable on 32-bit MCUs.
- **No SIMD on all platforms**: WASM SIMD is supported by Wasmtime but not by wasm3 (the interpreter used on the MCU profile).

Benchmarks on VyomaOS show WASM apps running at 60-90% of equivalent native performance for compute-bound workloads. For I/O-bound workloads (which most OS applications are), the difference is negligible.

### Limited Threading

WASM threads (based on `SharedArrayBuffer` semantics from the browser) are not yet part of WASI Preview 2. This means VyomaOS apps are currently single-threaded. The supervisor compensates by running each app in its own OS thread, enabling concurrent execution across apps, but individual apps cannot spawn threads internally.

The WASI threads proposal is in active development. When it stabilizes, VyomaOS will adopt it.

### Ecosystem Maturity

Not all libraries compile to `wasm32-wasip2`. Libraries that use inline assembly, platform-specific intrinsics, or `libc` FFI bindings will not work. In practice, the Rust ecosystem has strong WASM support — pure Rust crates generally compile without modification. The VyomaOS supervisor itself uses `fontdue`, `lodepng`, `serde`, `sha2`, and `toml` — all pure Rust, all WASM-compatible.

C/C++ libraries can be compiled to WASM via Emscripten or wasi-sdk, but this requires porting effort proportional to the library's reliance on POSIX APIs.

---

## The VyomaOS Approach

VyomaOS takes the position that WebAssembly is not just a runtime optimization — it is the correct application model for a capability-secure operating system.

### Every App Is wasm32-wasip2

There are no escape hatches. No "native mode" for performance-critical apps. No JNI-style foreign function interface. Every app, from `hello-world` to the HTTP server to the GUI dashboard, compiles to the same target:

```bash
cargo build --target wasm32-wasip2 --release
```

This uniformity means the supervisor has exactly one execution model to implement, one security model to enforce, and one set of interfaces to manage.

### The Supervisor Handles Hardware

Apps do not interact with hardware. The Rust supervisor (PID 1) owns:

- The framebuffer (`/dev/fb0`) — apps send `VYOMA_DRAW:` commands via stdout
- The keyboard — supervisor dispatches keypresses to the focused app's stdin
- The mouse — supervisor sends `VYOMA_INPUT:mouse:` events to apps with `mouse = true`
- The filesystem — supervisor mounts `/data` via 9P and grants access only to apps with `filesystem = true`
- The network — supervisor configures virtio-net and grants socket access only to apps with `network = true`

This design means a vulnerability in a WASM app cannot compromise hardware. The app literally has no way to address hardware — it only has the WASI interfaces the supervisor chose to provide.

```
┌──────────────┐
│   WASM App   │  Can only call WASI imports provided by supervisor
├──────────────┤
│   Wasmtime   │  Enforces memory safety, bounds checking
├──────────────┤
│  Supervisor  │  Owns all hardware, routes I/O, enforces manifests
├──────────────┤
│ Linux Kernel │  Minimal: virtio drivers, 9P, DRM
└──────────────┘
```

---

## Comparison: WASM vs Other Bytecode Formats

| Property | WASM | ELF (native) | JVM bytecode | .NET IL |
|----------|------|---------------|--------------|---------|
| Architecture-portable | Yes | No | Yes | Yes |
| Sandboxed by default | Yes | No | Partial (SecurityManager, deprecated) | Partial (CAS, deprecated) |
| Deterministic compilation | Yes | No | Partial | Partial |
| No garbage collector required | Yes | Yes | No | No |
| Binary size (hello world) | ~2 KB | ~300 KB (static) | ~1 KB + 200 MB JRE | ~3 KB + 100 MB CLR |
| Startup time | <10ms (JIT) | <1ms | 50-500ms (JVM warmup) | 20-200ms (CLR warmup) |
| Memory overhead | ~1 MB (runtime) | ~0 (no runtime) | 50-200 MB (JVM heap) | 20-100 MB (CLR) |
| Embedded viable (128 KB RAM) | Yes (wasm3 interpreter) | Yes | No | No |
| Threads | Limited (proposal) | Full | Full | Full |
| Raw hardware access | No | Yes | Via JNI | Via P/Invoke |

The JVM and .NET runtimes provide sandboxing and portability, but their garbage collectors make them unsuitable for real-time embedded systems and their memory overhead makes them impractical for microcontrollers. Native binaries have minimal overhead but no sandboxing or portability. WebAssembly is the only format that provides sandboxing, portability, and low overhead simultaneously.

---

## The Future: Component Model and Interface Types

WASI Preview 2 introduces the WebAssembly Component Model, which is the next major evolution of the platform. For VyomaOS, this enables:

### Component Composition

Today, VyomaOS apps are monolithic WASM modules. With the component model, an app could be composed of multiple components linked at load time:

```
app.wasm
  ├── ui-component.wasm    (exports rendering interface)
  ├── data-component.wasm  (exports storage interface)
  └── net-component.wasm   (exports HTTP client interface)
```

Each component gets only the capabilities it needs. The UI component gets display access but not network access. The network component gets socket access but not filesystem access. This extends capability security from the app level to the component level.

### Interface Types

The component model defines rich interface types — strings, lists, records, variants, resources — that enable language-independent APIs. A component written in Rust can export an interface consumed by a component written in Go, Python, or C, without serialization overhead. This opens VyomaOS to applications written in any language that compiles to WASM, not just Rust.

### Virtualization

Components can virtualize the interfaces they import. A testing component can intercept filesystem calls and return mock data. A debugging component can log all network traffic. This enables powerful development and debugging tools without modifying the application or the supervisor.

---

## Conclusion

WebAssembly is not a compromise. It is the only application binary format that provides sandboxing without a garbage collector, portability without a heavyweight runtime, and determinism without a reproducible build system. For an operating system that targets hardware from 128 KB microcontrollers to multi-core workstations, there is no other viable choice.

VyomaOS bets its entire application model on this foundation. Every security guarantee, every portability claim, and every architectural decision flows from the choice to make every app a `wasm32-wasip2` binary. The trade-offs — no raw syscalls, runtime overhead, limited threading — are not just acceptable; they are features of a system designed to be secure by construction rather than secure by policy.
