# Contract: Makefile Public Targets

**Date**: 2026-05-24

This document specifies the interface contract for the new and modified `make` targets added in the production refinement feature. All targets run inside the `vyomaos-builder` Docker container unless noted.

---

## New Targets

### `make test`

**Purpose**: Full test suite — unit tests + integration smoke test.

**Runs**:
1. `cargo test --manifest-path supervisor/Cargo.toml --target x86_64-unknown-linux-musl`
2. `bash base/scripts/smoke-test.sh` (headless QEMU boot, 30s timeout)

**Exit codes**:
- `0`: All unit tests pass AND smoke test passes
- Non-zero: Any unit test failure OR smoke test failure (with reason on stderr)

**Prerequisites**: `make build` must have completed successfully (supervisor + initramfs must exist).

**Usage**: `make test`

---

### `make unit-test`

**Purpose**: Run only `cargo test` for the supervisor crate (faster, no QEMU).

**Runs**: `cargo test --manifest-path supervisor/Cargo.toml --target x86_64-unknown-linux-musl`

**Exit codes**:
- `0`: All tests pass, zero warnings
- Non-zero: Any test failure or compiler warning (`RUSTFLAGS=-D warnings` is set)

**Usage**: `make unit-test`

---

### `make smoke`

**Purpose**: Run only the headless QEMU boot test (no unit tests).

**Runs**: `bash base/scripts/smoke-test.sh` inside Docker.

**Exit codes**:
- `0`: Supervisor reaches "all apps spawned" state within 30 seconds
- `1`: Timeout exceeded, QEMU error, or kernel panic detected

**Output**: `SMOKE: PASS` or `SMOKE: FAIL: <reason>` to stdout.

**Prerequisites**: `make build` must have completed.

**Usage**: `make smoke`

---

### `make check-manifests`

**Purpose**: Validate all `apps/*/vyoma.toml` files against the manifest schema without building.

**Runs**: Standalone Rust validation binary (compiled on first use) that parses every `vyoma.toml` and reports errors.

**Exit codes**:
- `0`: All manifests valid; prints per-app `OK` summary
- Non-zero: One or more manifests invalid; prints one error line per violation

**Output format** (one line per error):
```
ERROR apps/bad-app/vyoma.toml [bad-app] UnknownField: capabilities.network_v2
ERROR apps/dup-app/vyoma.toml [dup-app] DuplicateName: 'hello-world' already registered
OK    apps/hello-world/vyoma.toml [hello-world]
```

**Usage**: `make check-manifests`

---

## Modified Targets

### `make build` (modified)

**Change**: Now enforces `RUSTFLAGS=-D warnings`. Any new compiler warning causes non-zero exit.

**Behavior otherwise unchanged**: kernel + supervisor + apps + rootfs + disk.

---

### `make apps` (modified)

**Change**: Per-app stamp files replace the single `APPS_STAMP`. Touching one app's source only recompiles that app.

**Stamp locations**: `out/.apps/<app-name>.stamp` (one per app)

**Behavior otherwise unchanged**: compiles all `wasm32-wasip2` app binaries.

---

### `make supervisor` (modified)

**Change**: Now runs with `RUSTFLAGS=-D warnings`. New warnings are a build failure.

**Behavior otherwise unchanged**: compiles the musl supervisor binary.
