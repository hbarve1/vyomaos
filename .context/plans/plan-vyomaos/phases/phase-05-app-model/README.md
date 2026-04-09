# Phase 05 — App Model (wasm32-wasip2 + Capability Manifests)

## Goal

Migrate all Rust apps from `wasm32-unknown-unknown` to `wasm32-wasip2` so they use standard WASI interfaces instead of custom `extern "C"` exports. Introduce a TOML capability manifest per app that declares which WASI interfaces it needs, and a `boot.toml` that lists which apps to run at startup.

## Gate

```sh
# Apps compile to wasip2 target
cargo build --target wasm32-wasip2 --release --manifest-path apps/hello-world/Cargo.toml
ls apps/hello-world/target/wasm32-wasip2/release/hello_world.wasm

# Manifest schema validates
cat apps/hello-world/vyoma.toml | toml-validator 2>&1 | grep -c "error" | grep "^0$"

# Boot.toml drives which apps run
grep "hello-world" /etc/vyoma/boot.toml
```

## Dependencies

- Phase 04 (Wasmtime with WASI Preview 2 is running)
- `wasm32-wasip2` rustup target
- `toml` crate in supervisor for manifest parsing

## Tasks

### P05T01 — wasm32-wasip2 App Migration
[task-P05T01-wasip2-app-migration.md](tasks/task-P05T01-wasip2-app-migration.md)

### P05T02 — Capability Manifest Schema
[task-P05T02-capability-manifest-schema.md](tasks/task-P05T02-capability-manifest-schema.md)

### P05T03 — Config-Driven Boot
[task-P05T03-config-driven-boot.md](tasks/task-P05T03-config-driven-boot.md)
