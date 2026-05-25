---
title: Supervisor (PID 1)
description: The Rust binary that runs everything.
---

The supervisor is a 697 KB static Rust binary that serves as PID 1 — the first and only native process in VyomaOS. It manages every aspect of the system above the kernel.

## Modules

### Manifest Parser

Reads `vyoma.toml` files at startup and validates capability declarations against the schema. Invalid manifests are rejected with clear error messages.

```toml
[app]
name    = "shell"
version = "0.1.0"
wasm    = "shell.wasm"

[capabilities]
stdio      = true
filesystem = true
shell      = true
```

### Concurrent Scheduler

Spawns one thread per WASM app, each running a Wasmtime process. Supports restart policies:

- **`never`**: One-shot apps that exit and stay stopped
- **`always`**: Apps that restart automatically on exit (e.g., shell, gui-demo)

### IPC Broker

Intercepts messages written to stdout by apps and routes them:

- **Unicast**: `@app-name: message` — routes to a specific app
- **Broadcast**: `@broadcast: message` — delivers to all running apps
- **Reply**: `@reply: message` — routes back to the last sender
- **Supervisor**: `@supervisor: command` — built-in control commands

### Display Driver

Opens `/dev/fb0` (DRM/virtio-gpu framebuffer) and renders:

- Parses `VYOMA_DRAW:` protocol commands from app stdout
- Renders text using a built-in 8x16 bitmap font
- Manages window chrome: title bars, focus borders, status strips
- Tracks per-app flush rate (FPS)
- Supports double-buffered compositing

### Input Router

Puts the TTY in raw mode and dispatches input events:

- **Keyboard**: Per-keypress dispatch to the focused app via `VYOMA_INPUT:key:` prefix
- **Mouse**: Click, move, and drag events via `VYOMA_INPUT:mouse:` prefix
- **Hotkeys**: Alt+? for shortcut overlay, click-to-focus in menu bar

### Process Manager

Runtime process management via console commands and IPC:

| Command | Description |
|---------|-------------|
| `ps` | List running apps with PID, name, uptime |
| `log <name>` | Print app's captured log output |
| `logf <name>` | Tail app logs in real-time |
| `kill <name>` | Terminate an app |
| `restart <name>` | Restart an app |
| `reload <name>` | Reload app from disk |

### Supervisor IPC Commands

Apps (or the console) can send commands to the supervisor:

| Command | Response |
|---------|----------|
| `@supervisor: list` | List of running app names |
| `@supervisor: apps` | Running app name list |
| `@supervisor: ping` | `pong` with timestamp |
| `@supervisor: uptime` | System uptime since boot |
| `@supervisor: version` | VyomaOS version string |
| `@supervisor: focus <name>` | Switch keyboard focus to app |
| `@supervisor: loglevel <level>` | Set per-app log level filter |
| `@supervisor: kill <name>` | Terminate an app |

## Build

The supervisor compiles as a static musl binary:

```bash
cd supervisor
cargo build --target x86_64-unknown-linux-musl --release
```

Output: `supervisor/target/x86_64-unknown-linux-musl/release/supervisor` (697 KB)

## Code Organization

The supervisor follows a 500-line-per-file rule. Major subsystems are split into focused modules:

- `main.rs` — Entry point, boot sequence, app spawn loop
- `chrome.rs` — Window chrome rendering (title bars, borders)
- `display/` — Framebuffer driver, compositor
- `draw_cmd.rs` — VYOMA_DRAW protocol parser
- `ipc_handlers.rs` — IPC message routing
- `ipc_commands/` — Built-in supervisor commands
- `input_keys.rs` — Keyboard input handling
- `mouse_input.rs` — Mouse event processing
- `app_threads.rs` — Per-app thread management
- `mount.rs` — Filesystem mounting
- `net.rs` — Network setup
- `packages.rs` — Package manager
- `seccomp.rs` — Seccomp BPF setup
- `toast.rs` — Toast overlay rendering
