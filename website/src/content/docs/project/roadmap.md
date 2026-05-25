---
title: Roadmap
description: 17 phases complete. Building toward a full desktop OS.
---

VyomaOS development is organized into numbered phases, each adding a specific capability to the system.

## Completed Phases

### Foundation (P01–P08)

| Phase | Feature |
|-------|---------|
| P01 | Linux kernel (allnoconfig, 2.3 MB) |
| P02 | Rust supervisor (PID 1, static musl) |
| P03 | WASM runtime integration (Wasmtime, WASI P2) |
| P04 | App manifest model (vyoma.toml, capabilities) |
| P05 | Seccomp BPF denylist for Wasmtime children |
| P06 | Persistent storage (9P virtio, /data mount) |
| P07 | Unit test infrastructure |
| P08 | CI pipeline (build + test + smoke) |

### Display & Input (P09–P12)

| Phase | Feature |
|-------|---------|
| P09 | DRM/virtio-gpu framebuffer display |
| P10 | Bitmap font rendering (8x16, three sizes) |
| P11 | Networking (HTTP server, WASI sockets) |
| P12 | Interactive shell + keyboard routing |

### System Services (P13–P17)

| Phase | Feature |
|-------|---------|
| P13 | Process management (ps, kill, restart) |
| P14 | Package manager (install, update, remove) |
| P15 | Persistent logs |
| P16 | Real-time TTY input |
| P17 | Windowing + mouse input |

## Current Work

### P18–P19: Display Enhancements

- Font scaling improvements
- GPU-accelerated display pipeline
- Dirty region tracking for reduced redraws
- Reduced lock contention in compositor

### P20–P21: Platform Features

- Multi-user authentication (capability tokens)
- App store with signed WASM bundles

## Future Vision

### P22+: Full Desktop OS

- **Desktop Shell**: App launcher, taskbar, notifications — all WASM
- **Hardware Support**: USB, audio, camera, sensors as typed WASI interfaces
- **OTA Updates**: Atomic supervisor + kernel updates with rollback
- **Accessibility**: Screen reader, high-contrast, input assistance
- **Developer Tools**: WASM-native compiler toolchain, debugger, REPL

## App Ecosystem Growth

The app ecosystem has grown from a handful of demos to 200+ apps across categories:

- **Productivity**: text-editor, notes, kanban, calendar, spreadsheet
- **Developer Tools**: code-editor, hex-editor, json-viewer, diff-viewer, terminal
- **System**: system-monitor, process-inspector, file-manager, settings
- **Games**: chess, tetris, snake, minesweeper, asteroids, wordle
- **Visualization**: fractal, fourier, oscilloscope, fluid, ray-march
- **Creativity**: paint-pro, music-composer, pixel-art, photo-editor

Each app is 1–10 KB, sandboxed, and runs on any VyomaOS instance.
