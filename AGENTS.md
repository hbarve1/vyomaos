# AGENTS.md

## Cursor Cloud specific instructions

### Overview

VyomaOS is a WASM-first OS. The core development loop involves two Rust targets:
- **Supervisor** (`supervisor/`): `x86_64-unknown-linux-musl` — the PID 1 process
- **WASM apps** (`apps/*/`): `wasm32-wasip2` — individual applications

The hermetic build system uses Docker (`make build`), but local `cargo check` / `cargo test` works for fast iteration.

### Quick reference

| Task | Command |
|------|---------|
| Lint supervisor | `RUSTFLAGS="-D warnings" cargo check --manifest-path supervisor/Cargo.toml --target x86_64-unknown-linux-musl` |
| Lint a WASM app | `cargo check --manifest-path apps/<name>/Cargo.toml --target wasm32-wasip2` |
| Unit tests (local) | `RUSTFLAGS="-D warnings" cargo test --manifest-path supervisor/Cargo.toml --target x86_64-unknown-linux-musl` |
| Unit tests (Docker) | `make unit-test` |
| Full build (Docker) | `make build` |
| Boot OS (headless) | `make run` |

See `CLAUDE.md` and the root `Makefile` for the full command reference.

### Gotchas

- **Workspace target directory**: The root `Cargo.toml` defines a workspace with `supervisor` and `cli`. When building via `cargo build --manifest-path supervisor/Cargo.toml`, the output goes to `/workspace/target/x86_64-unknown-linux-musl/release/supervisor` (workspace root), **not** `supervisor/target/`. However, `base/modules/rootfs.sh` expects the binary at `supervisor/target/x86_64-unknown-linux-musl/release/supervisor`. After building the supervisor (either locally or via Docker `make supervisor`), you may need to copy the binary: `cp target/x86_64-unknown-linux-musl/release/supervisor supervisor/target/x86_64-unknown-linux-musl/release/supervisor`.
- **Docker daemon**: Docker must be started manually (`sudo dockerd &`) since the VM doesn't use systemd. After starting, run `sudo chmod 666 /var/run/docker.sock` to allow non-root Docker usage.
- **QEMU without KVM**: The cloud VM runs inside Firecracker, so KVM is unavailable. QEMU runs in TCG (software emulation) mode, making the smoke test ~2 minutes instead of seconds. Use a 120s+ timeout for smoke tests.
- **`tools/check-manifests`**: Has a duplicate `[workspace]` key in its `Cargo.toml` (lines 1 and 18) which newer Rust versions reject. To build apps without triggering this, build them directly rather than through `make apps` (which depends on `check-manifests`).
- **`cli/` (vyoma CLI)**: Has pre-existing build errors with current Rust stable. Not part of the core OS development loop.
