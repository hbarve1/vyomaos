# Phase 06 — Multi-App & IPC (Component Model)

## Goal

Run multiple WASM apps concurrently, each in its own Wasmtime instance. Add restart policies (on-failure, always, never). Introduce WASM Component Model typed IPC via `.wit` interface definitions so apps can call each other's exported functions with compile-time type safety.

## Gate

```sh
# Two apps run concurrently and both produce output
qemu-system-x86_64 -kernel out/bzImage -initrd out/initramfs.cpio.gz \
  -append "console=ttyS0 quiet" -nographic -m 512M 2>&1 | \
  grep -c "app:" | awk '{if ($1 >= 2) print "OK"; else print "FAIL"}'

# Failed app restarts (simulate crash app)
# Supervisor log shows "restarting app crash-test after exit code 1"

# WIT interface compiles without error
wasm-tools component wit apps/math/math.wit 2>&1 | grep -c "error" | grep "^0$"
```

## Dependencies

- Phase 05 (app model and manifests in place)
- `wasmtime` Rust crate with component-model feature
- `wasm-tools` CLI for WIT compilation

## Tasks

### P06T01 — Multi-App Scheduler
[task-P06T01-multi-app-scheduler.md](tasks/task-P06T01-multi-app-scheduler.md)

### P06T02 — App Restart Policy
[task-P06T02-app-restart-policy.md](tasks/task-P06T02-app-restart-policy.md)

### P06T03 — WIT Interface Definitions
[task-P06T03-wit-interface-definitions.md](tasks/task-P06T03-wit-interface-definitions.md)

### P06T04 — Component Model Linking
[task-P06T04-component-model-linking.md](tasks/task-P06T04-component-model-linking.md)
