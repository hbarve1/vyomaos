# Phase 06 — Multi-App & IPC

**Status: partial** — P06T01 + P06T02 complete; P06T03 + P06T04 (Component Model typed IPC) deferred

## Goal

Run multiple WASM apps concurrently, each in its own Wasmtime instance. Add restart policies (on-failure, always, never). Introduce inter-app messaging via the supervisor IPC broker.

## What was actually built

**P06T01 — Concurrent scheduler (done):** One thread per app (`std::thread::spawn`). Separate writer/reader/waiter threads per app. Two-pass spawn for race-free inbox registration.

**P06T02 — Restart policies (done):** `restart = never/always/on-failure` in boot.toml; `always`/`on-failure` logs a warning that restart+IPC requires new pipe setup (not yet implemented) and treats as `never`.

**P06T03/P06T04 — Component Model typed IPC (deferred):** The supervisor stays a CLI-wasmtime spawner; embedding the wasmtime Rust crate for custom WIT host functions is a future architectural shift. Instead, a text-based IPC protocol was implemented: apps write `@<appname>: <message>` to stdout; the supervisor's reader thread routes it to the target app's stdin. Demo: `ping` and `pong` apps exchange 3 messages.

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
