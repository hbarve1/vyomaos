# AGENTS.md

## Current release plan

Read [TODOS.md](TODOS.md) before selecting implementation work. R1 is a terminal-first desktop OS; that checklist controls release scope, task dependencies, acceptance evidence, and handoff state. The graphical desktop and broader device roadmap remain later work. See [docs/codex-work-loop.md](docs/codex-work-loop.md) for the proposed sustained-work process; documenting it does not activate a scheduler or grant merge/release authority.

## Local validation while Actions is blocked

GitHub Actions is blocked by billing (maintainer instruction, 2026-09-29). Run all build/test validation locally, using Docker for the production musl target and host QEMU/Python for VM tests. Do not dispatch or rerun Actions as a substitute. Include commands, toolchain, results and known failures in each PR; never count a skipped test as passing. Preserve logs under `out/validation/` and commit a concise evidence report. See [the T0-01 baseline](docs/validation/2026-09-29-t0-01.md) for commands and current harness failures.

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

- **Workspace target directory**: Supervisor and CLI outputs are under root `target/`; `base/modules/rootfs.sh` already uses that path. Release profiles belong in root `Cargo.toml`.
- **musl linking**: Use Rust's default linker with its bundled musl CRT. Forcing `musl-gcc` as Cargo's linker in the current builder produced binaries with a dynamic interpreter that crashed at startup. The compiler remains available for native C dependencies.
- **Disposable test state**: Some Rust tests write absolute `/data` and `/apps` paths. Run them in a non-root container with writable tmpfs mounts for those paths, not against host data. Run serially because tests share global state.
- **Docker and KVM**: Check the current host rather than assuming daemon/KVM availability. The T0-01 run used an existing Docker daemon; Python VM tests used KVM, while shell smoke/GUI scripts used TCG.
- **WASM protocol tests**: Set `XDG_CACHE_HOME` to a writable temporary directory in a non-root builder, and provide the verified Wasmtime binary through `WASMTIME`.
- **Python harness**: The full suite has API mismatches and shared `/tmp` paths; its harness unit tests can unlink a running VM's serial log. Record full-suite failures and run VM-only tests separately for diagnosis until T0-05/T0-06 repairs this.
