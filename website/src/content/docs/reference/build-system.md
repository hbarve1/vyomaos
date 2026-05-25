---
title: Build System
description: Docker-based hermetic builds for reproducible OS images.
---

VyomaOS uses a Docker-based build system to ensure fully reproducible builds across any development machine.

## Build Pipeline

```
make kernel       → out/bzImage (2.3 MB)
make supervisor   → supervisor binary (697 KB, static musl)
make apps         → 200+ .wasm binaries (1-10 KB each)
make rootfs       → out/initramfs.cpio.gz (18 MB compressed)
make run          → QEMU boot
```

## Docker Environment

All compilation happens inside a Docker container:

- **Base image**: Ubuntu 22.04
- **Kernel build deps**: gcc, make, bc, flex, bison
- **Rust toolchain**: Stable, with `x86_64-unknown-linux-musl` and `wasm32-wasip2` targets
- **Verified binaries**: Wasmtime 43.0.0 and BusyBox 1.35.0 (SHA-256 checked)

Build the Docker image:

```bash
make image
```

## Build Targets

| Target | Description |
|--------|-------------|
| `make build` | Full build: kernel + supervisor + apps + rootfs + disk |
| `make kernel` | Compile Linux 5.10 kernel (allnoconfig) |
| `make supervisor` | Compile Rust supervisor for musl |
| `make apps` | Compile all WASM apps (incremental) |
| `make rootfs` | Package initramfs from all artifacts |
| `make run` | Boot headless in QEMU |
| `make run-gui` | Boot with virtio-gpu display |
| `make run-net` | Boot with virtio-net networking |
| `make run-gui-net` | Display + networking |
| `make shell` | Open bash inside builder container |
| `make clean` | Remove `out/` directory |
| `make clean-image` | Remove Docker builder image |

## Incremental Builds

Per-app stamp files (`out/.apps/<name>.stamp`) ensure only changed apps recompile:

```bash
# Only the touched app recompiles
touch apps/snake/src/main.rs
make apps
```

## Build Artifacts

| Artifact | Path | Size |
|----------|------|------|
| Linux kernel | `out/bzImage` | 2.3 MB |
| Initramfs | `out/initramfs.cpio.gz` | 18 MB |
| Data disk | `out/disk.img` | 64 MB (ext4, created once) |
| Supervisor | `supervisor/target/.../supervisor` | 697 KB |
| WASM apps | `apps/*/target/.../release/*.wasm` | 1-10 KB each |

## Testing

```bash
# Full CI suite
make test

# Unit tests only (cargo test in Docker)
make unit-test

# Manifest validation
make check-manifests

# Headless QEMU smoke test (30s timeout)
make smoke
```

### Smoke Test

`make smoke` boots QEMU headless, waits for `[lifecycle] all apps spawned` on serial output, then exits. Pass/fail is printed as `SMOKE: PASS` or `SMOKE: FAIL: <reason>`.

### Unit Tests

Unit tests live in `supervisor/tests/` and cover manifest parsing, IPC routing, process lifecycle, display commands, font rendering, input handling, mouse events, watchdog, and window management.

## Development Workflow

### Supervisor changes

```bash
cd supervisor
cargo build --target x86_64-unknown-linux-musl --release
cd ..
make rootfs && make run
```

### Single app changes

```bash
cd apps/my-app
cargo build --target wasm32-wasip2 --release
cd ../..
make rootfs && make run
```

### Interactive debugging

```bash
make shell    # Opens bash in the Docker container
```
