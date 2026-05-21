# VyomaOS

A **WASM-first operating system** with the long-term goal of becoming a lightweight but fully capable general-purpose OS — on par with Windows, macOS, Android, and Ubuntu — built from the ground up on a capability-secure WebAssembly foundation.

> **This is an open research project. We're actively looking for contributors.** See [CONTRIBUTING.md](CONTRIBUTING.md) to get started.

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

## Current State (Phase 77+)

VyomaOS boots in QEMU in under 5 seconds to a macOS-like desktop with 77+ WASM apps including a Menu Bar, Dock, Spotlight search, App Switcher, Notification Center, Mission Control, and a growing suite of productivity tools:

```
Linux 5.10 (allnoconfig, ~2.3 MB)
  └── Rust supervisor (static musl, PID 1)
        ├── menu-bar.wasm        — top menu bar (clock, focused app, system tray)
        ├── dock.wasm            — bottom dock with running indicators
        ├── spotlight.wasm       — Cmd+Space app search overlay
        ├── app-switcher.wasm    — Tab-cycle running apps
        ├── mission-control.wasm — bird's-eye view of all windows
        ├── notification-center.wasm — slide-in notification panel
        ├── shell.wasm           — interactive command shell
        ├── file-manager.wasm    — file browser
        ├── text-editor.wasm     — full editor with save/load
        ├── browser.wasm         — basic HTTP browser
        ├── app-store.wasm       — package manager GUI
        └── 65+ more apps...
```

**Working features:**
- macOS-like desktop: Menu Bar, Dock, Spotlight, App Switcher, Mission Control
- Concurrent scheduler with restart policies (`never` / `always`)
- Bidirectional IPC broker (`@<app>: <message>` routing)
- Z-ordering and window focus management
- Double-buffered compositor (back buffer → mmap blit)
- seccomp BPF denylist + capability audit log
- 9P virtio persistent storage (`/data`, survives reboots)
- DRM/virtio-gpu display at 1440×900
- `VYOMA_DRAW:` framebuffer protocol with font scaling (S/M/L)
- virtio-net + WASI sockets
- Process management, package manager, OTA updates, session manager
- 77+ WASM apps covering productivity, dev tools, networking, media

## Getting Started

```sh
# Prerequisites: Docker, QEMU
make          # build kernel + initramfs
make run      # boot in QEMU (serial output)
make run-gui DISPLAY_BACKEND=cocoa   # boot with virtio-gpu display (macOS)
make run-gui DISPLAY_BACKEND=sdl     # boot with virtio-gpu display (Linux)
```

## Contributing

**VyomaOS is looking for contributors who are excited about rethinking the OS from scratch.**

The project is at a genuinely interesting point: the foundation is solid (boot, IPC, display, security), and the surface area for new work is large. You don't need to understand the full codebase to contribute — each WASM app is self-contained in its own directory.

**Where to start:**
- Read [CONTRIBUTING.md](CONTRIBUTING.md) for setup, architecture, and contribution guidelines
- Browse [`apps/`](apps/) — each app is an independent Rust crate targeting `wasm32-wasip2`
- Check [`.context/plans/plan-vyomaos/README.md`](.context/plans/plan-vyomaos/README.md) for planned phases
- Open an issue to discuss a new app, feature, or design question

**Good first contributions:**
- A new WASM app (any utility, tool, or game — see CONTRIBUTING.md for the template)
- Improving an existing app (better UX, new features, bug fixes)
- Supervisor improvements (new `@supervisor:` commands, capability types)
- Documentation (architecture writeups, tutorials, diagrams)
- Testing infrastructure (boot smoke tests, CI setup)

## Roadmap

| Phase | Feature | Status |
|---|---|---|
| P01–P08 | Kernel, supervisor, WASM runtime, IPC, seccomp, storage | complete |
| P09–P12 | DRM display, bitmap font, HTTP server, interactive shell | complete |
| P13–P17 | Process management, package manager, persistent logs, real-time TTY | complete |
| P18–P23 | Shell UX, watchdog, font scaling, window regions, mouse input, TUI primitives | complete |
| P24–P51 | App ecosystem (file manager, editor, browser, system monitor, 27+ apps) | complete |
| P52–P69 | More apps (virtual keyboard, color picker, process inspector, 17+ apps) | complete |
| P70–P77 | CSV/JSON viewers, Menu Bar, Dock, Spotlight, App Switcher, Notification Center, Mission Control | complete |
| P78–P80 | Desktop Icons, Context Menu, Finder v2 | in progress |
| P81+ | Wayland compositor, GPU acceleration, multi-user, real hardware | planned |

Full phase details: [`.context/plans/plan-vyomaos/`](.context/plans/plan-vyomaos/README.md)

## Guiding Principles

- **WASM-first**: applications are WebAssembly modules, not ELF binaries.
- **Capability-secure by default**: every app declares what it needs; everything else is inaccessible.
- **Minimal kernel**: allnoconfig base, only the drivers the system actually uses.
- **Reproducible**: deterministic builds via Docker; any engineer can reproduce the exact same image.
- **Scale up, not out**: the same architecture that boots in 18 MB today targets a full desktop OS tomorrow.

## See Also

- [CONTRIBUTING.md](CONTRIBUTING.md) — how to contribute
- [Comparison matrix & performance tracker](docs/comparison-matrix.md) — VyomaOS vs Alpine, Flatcar, MirageOS, containers
- [App manifest schema](docs/vyoma-manifest-schema.md) — capability manifest reference
- [Build system](base/README.md)
- [Apps](apps/README.md)
