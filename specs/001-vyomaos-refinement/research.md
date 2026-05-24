# Research: VyomaOS Production Refinement

**Branch**: `001-vyomaos-refinement` | **Date**: 2026-05-24

## Decision 1: Structured Logging Approach

**Decision**: Custom zero-crate macro in `supervisor/src/logging.rs` writing to stderr.

**Format**:
```
[2026-05-24T12:34:56.789Z] [INFO]  [manifest]  app=hello-world  Manifest parsed OK
[2026-05-24T12:34:56.791Z] [WARN]  [capability] app=bad-app      Unknown field: network_v2
[2026-05-24T12:34:56.793Z] [ERROR] [lifecycle]  app=missing      wasm binary not found
```

Fields: ISO 8601 timestamp (ms precision via `SystemTime`), fixed-width level (`INFO /WARN /ERROR`), padded subsystem (`manifest/capability/lifecycle/ipc/display/input`), optional `app=<name>`, message.

**Rationale**: No new crate dependency (Principle III — minimal surface). `tracing` or `log` crates add 50–200 KB to the supervisor binary and require async runtime assumptions incompatible with the single-threaded startup path. `std::time::SystemTime` provides ms-precision timestamps with zero overhead.

**Alternatives considered**:
- `tracing` crate: rejected — too heavy, pulls in `pin-project-lite`, `once_cell`, async glue
- `log` + `env_logger`: rejected — `env_logger` is 80 KB overhead; env-var-driven behavior is forbidden (Principle V)
- `serde_json` log lines: rejected — binary size impact; plain text is sufficient for serial console grep

---

## Decision 2: Test Architecture for Monolithic Supervisor

**Decision**: Extract `manifest.rs` module with pure functions; add `supervisor/tests/` integration test files; keep all OS-specific code behind `#[cfg(target_os = "linux")]`.

**Rationale**: The supervisor is 2 619 lines in a single `main.rs`. Pure-logic functions (manifest parsing, capability validation, IPC routing, log formatting) can be extracted without touching Linux-specific code. Rust's integration tests in `tests/` compile as separate crates that access `pub` items — this works cleanly for musl targets where the code compiles on Linux but `#[cfg(target_os = "linux")]` guards prevent non-Linux-specific items from compiling on the host test runner. Tests inside the container (Linux) will exercise all paths.

**Modules to extract**:
- `supervisor/src/manifest.rs` — `parse_manifest()`, `validate_manifest()`, `BootConfig`, `AppManifest`, `Capabilities` structs
- `supervisor/src/logging.rs` — `log_info!`, `log_warn!`, `log_error!` macros + `Subsystem` enum

**Alternatives considered**:
- Keep all tests in `#[cfg(test)]` blocks inside `main.rs`: rejected — `main.rs` is already 2 619 lines; test modules would make it unmaintainable
- Full module decomposition (ipc.rs, lifecycle.rs, scheduler.rs): deferred — out of scope for this feature; only manifest + logging extraction is required
- Workspace with separate `vyoma-supervisor-lib` crate: rejected — adds build complexity (Principle III)

---

## Decision 3: Smoke Test Strategy

**Decision**: Shell script `base/scripts/smoke-test.sh` that runs headless QEMU inside Docker, captures serial output via `-serial stdio`, and greps for the supervisor's ready line within 30 seconds.

**Ready signal**: The existing log line `"vyoma-supervisor: {} app(s) total"` appears after all manifests are parsed. A new INFO structured log line `"all apps spawned"` from the lifecycle subsystem will serve as the definitive ready marker.

**Implementation**:
```sh
timeout 30 qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -nographic -serial stdio \
  -append "console=ttyS0 quiet" \
  2>&1 | grep -m1 "all apps spawned" && echo "SMOKE: PASS" || echo "SMOKE: FAIL"
```

**Rationale**: Simple, no external dependencies beyond QEMU (already used by `make run`). The grep-on-ready-signal approach is robust: it doesn't care about timing, only whether the supervisor reaches the ready state.

**Alternatives considered**:
- `expect` script: rejected — adds `expect` dependency; `grep -m1` with `timeout` achieves the same
- Boot log file comparison: rejected — fragile; log content changes as apps are added
- Python-based serial harness: rejected — adds Python dependency to builder image

---

## Decision 4: Per-App Incremental Build

**Decision**: Replace single `$(APPS_STAMP)` with pattern rule using per-app stamp files at `$(OUT)/.apps/<name>.stamp`, each depending only on that app's sources.

**Implementation sketch**:
```makefile
APP_NAMES := hello-world calculator factorial ping pong ...  (all 205 apps)

define APP_RULE
$(OUT)/.apps/$(1).stamp: $$(wildcard apps/$(1)/src/*.rs) apps/$(1)/Cargo.toml | image
	@mkdir -p $(OUT)/.apps
	$(DOCKER_RUN) cargo build --manifest-path apps/$(1)/Cargo.toml \
	  --target wasm32-wasip2 --release
	@touch $$@
endef

$(foreach app,$(APP_NAMES),$(eval $(call APP_RULE,$(app))))

apps: $(foreach app,$(APP_NAMES),$(OUT)/.apps/$(app).stamp)
```

**Rationale**: Current `$(APPS_STAMP)` depends on `$(shell find apps -name '*.rs')` — touching any app source rebuilds all 205 apps. Per-app stamps reduce a single-app change from ~40 min (205 × 12s container startup) to ~15s (1 app).

**Alternatives considered**:
- Cargo workspace: considered — would enable `cargo build --workspace` with natural incremental support, but requires restructuring all 205 Cargo.toml files; out of scope
- `cargo-make` / `just`: rejected — new tool dependency, complexity exceeds benefit

---

## Decision 5: Warning Enforcement Mechanism

**Decision**: Add `RUSTFLAGS := -D warnings` to Makefile for all `cargo build` and `cargo test` invocations inside Docker. Explicitly set, not via `.cargo/config.toml`, to preserve developer override capability on host.

**Rationale**: `RUSTFLAGS=-D warnings` is the idiomatic Rust CI pattern; it causes `cargo` to emit non-zero exit on any warning, which propagates to Make's exit code and fails the build. Setting it in the Makefile (not `.cargo/config.toml`) allows a developer to override on the command line: `RUSTFLAGS="" make supervisor` when working around a transient upstream warning.

**Alternatives considered**:
- `#![deny(warnings)]` in source: rejected — bakes policy into source, cannot be overridden without code change; also fires on nightly lints that change across Rust versions
- `.cargo/config.toml` approach: rejected — cannot be overridden per-invocation without a config file edit
