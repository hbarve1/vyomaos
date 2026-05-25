---
title: Quick Start
description: Build and boot VyomaOS in minutes.
---

## Prerequisites

- **Docker** (for hermetic builds)
- **QEMU** (for running the VM)

## Build & Boot

```bash
# Clone the repo
git clone https://github.com/hbarve1/vyomaos.git
cd vyomaos

# Full build: kernel + supervisor + apps + rootfs
make build

# Boot headless (serial console)
make run

# Boot with GUI display (macOS)
make run-gui DISPLAY_BACKEND=cocoa

# Boot with GUI display (Linux)
make run-gui DISPLAY_BACKEND=sdl
```

## What Happens at Boot

1. Linux 5.10 kernel starts (allnoconfig, ~2.3 MB)
2. Rust supervisor launches as PID 1
3. Supervisor reads `/etc/vyoma/boot.toml` for app list
4. Each app's `vyoma.toml` manifest is parsed for capabilities
5. One Wasmtime process spawns per app with only declared WASI imports
6. All 200+ apps are running within 5 seconds

## Inside the VM

Once booted, the supervisor console accepts commands:

```bash
ps                          # List running apps
log <name>                  # Print app logs
logf <name>                 # Tail app logs (follow mode)
kill <name>                 # Terminate app
restart <name>              # Restart app
@supervisor: list           # List apps via IPC
@supervisor: focus <name>   # Switch keyboard focus
@supervisor: ping           # Health check
@supervisor: uptime         # System uptime
```

## Build Targets

| Command | What it does |
|---------|-------------|
| `make build` | Full build: kernel + supervisor + apps + rootfs + disk |
| `make kernel` | Compile Linux kernel only |
| `make supervisor` | Compile Rust supervisor only |
| `make apps` | Compile all WASM apps (incremental) |
| `make rootfs` | Create initramfs from artifacts |
| `make run` | Boot headless |
| `make run-gui` | Boot with virtio-gpu display |
| `make run-net` | Boot with virtio-net networking |
| `make run-gui-net` | Display + networking |
| `make test` | Full CI: build + unit tests + smoke test |

## Networking

To enable networking (e.g., for the HTTP server app):

```bash
make run-net            # Headless + port 8080 forwarded
make run-gui-net        # GUI + networking
```

Then access `http://localhost:8080` from your host machine.
