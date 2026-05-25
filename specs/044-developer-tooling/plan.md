# Implementation Plan: VyomaOS Developer Tooling (vyoma CLI)

**Branch**: `044-developer-tooling` | **Date**: 2026-05-25 | **Spec**: `specs/044-developer-tooling/spec.md`

**Input**: Feature specification from `/specs/044-developer-tooling/spec.md`

## Summary

Build the `vyoma` host-side CLI that connects to a running VyomaOS instance over TCP (port 9090) or QEMU Unix socket and provides five commands: `ps`, `logs`, `push`, `monitor`, `exec`. Simultaneously, add a management server (`mgmt_server.rs`) to the supervisor that accepts NDJSON requests and streams structured responses. The CLI is a statically-linked Rust binary under 10 MB; the server uses `std::net::TcpListener` (no tokio) with one thread per client.

## Technical Context

**Language/Version**: Rust stable (same toolchain as supervisor)

**Primary Dependencies**:
- Supervisor (server): `serde`, `serde_json` (already in Cargo.toml)
- CLI (client): `clap v4` (derive + cargo), `serde`, `serde_json`, `crossterm`, `indicatif`

**Storage**: N/A (CLI is stateless; supervisor AppRegistry already in memory)

**Testing**: `cargo test` (unit tests in `supervisor/tests/`, CLI integration tests via TCP loopback)

**Target Platform**:
- Supervisor server: inside QEMU VM, `x86_64-unknown-linux-musl`
- CLI client: host machine, `x86_64-unknown-linux-musl` (Linux) or `aarch64-apple-darwin` (macOS)

**Project Type**: CLI tool + supervisor subsystem

**Performance Goals**: `vyoma ps` < 1s for 20 apps; `vyoma logs` < 2s latency; `vyoma push` first progress line < 5s

**Constraints**: CLI binary ≤ 10 MB; no tokio; no runtime deps beyond POSIX

**Scale/Scope**: 1–20 concurrent apps; 1–2 concurrent CLI clients; 4 commands in v1

## Constitution Check

- ✅ No source file exceeds 500 lines (cli/src/main.rs ≤ 200, each command ≤ 300, supervisor/src/mgmt_server.rs ≤ 400)
- ✅ Single binary (CLI) + single subsystem addition (supervisor server)
- ✅ No new project beyond the CLI workspace; supervisor extended in-place
- ✅ TDD: failing test before implementation for each supervisor server handler

## Project Structure

### Documentation (this feature)

```text
specs/044-developer-tooling/
├── plan.md              # This file
├── research.md          # Phase 0 output
├── data-model.md        # Phase 1 output
├── quickstart.md        # Phase 1 output
├── contracts/
│   ├── cli-commands.md  # CLI interface contract
│   └── mgmt-protocol.md # TCP wire protocol contract
└── tasks.md             # Phase 2 output (speckit-tasks)
```

### Source Code (repository root)

```text
supervisor/src/
├── mgmt_server.rs        # NEW: TcpListener + handler dispatch
├── mgmt_handlers.rs      # NEW: ps/logs/push/heartbeat_stream/exec handlers
├── mgmt_protocol.rs      # NEW: serde structs for NDJSON messages (shared types)
└── main.rs               # MODIFY: spawn MgmtServer thread at startup

supervisor/tests/
├── mgmt_ps.rs            # NEW: unit test for ps handler
├── mgmt_logs.rs          # NEW: unit test for log streaming
├── mgmt_push.rs          # NEW: unit test for push flow
└── mgmt_exec.rs          # NEW: unit test for exec handler

cli/
├── Cargo.toml            # NEW: workspace member
└── src/
    ├── main.rs           # NEW: clap CLI entry (≤ 200 lines)
    ├── connection.rs     # NEW: VyomaConnection (TCP + socket transport)
    ├── protocol.rs       # NEW: NDJSON client-side message structs
    └── commands/
        ├── ps.rs         # NEW: vyoma ps
        ├── logs.rs       # NEW: vyoma logs
        ├── push.rs       # NEW: vyoma push
        ├── monitor.rs    # NEW: vyoma monitor
        └── exec.rs       # NEW: vyoma exec

Cargo.toml (root workspace)
  → add "cli" as workspace member
```

**Structure Decision**: CLI as a separate `cli/` workspace member (different target triple and dependency set from supervisor). Supervisor server code split across three files to stay under 500 lines each.
