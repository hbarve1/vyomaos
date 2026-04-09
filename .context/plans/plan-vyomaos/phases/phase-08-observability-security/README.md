# Phase 08 — Observability & Security

## Goal

Add structured logging from the supervisor (JSON lines to ttyS1), apply a seccomp allowlist to Wasmtime child processes, isolate each app in its own Linux namespace (PID + mount), and enforce cryptographic signing of WASM binaries so unsigned apps are refused at boot.

## Gate

```sh
# Supervisor emits JSON log lines
qemu-system-x86_64 ... -serial stdio -serial file:/tmp/vyoma.log 2>&1
cat /tmp/vyoma.log | jq '.app' | grep "hello-world"

# Wasmtime child has seccomp filter applied
# (verify via strace -e seccomp on the child process in test env)

# Unsigned WASM is rejected
# create unsigned.wasm, copy to rootfs, verify supervisor logs "rejected: no signature"

# Signed WASM runs successfully
# sign hello-world.wasm with vyoma-sign, verify it runs
```

## Dependencies

- Phase 06 (supervisor manages multiple app processes)
- Phase 07 (networking for remote log shipping, optional)
- `libseccomp` or Rust `seccompiler` crate
- `ed25519-dalek` or similar for signing

## Tasks

### P08T01 — Structured Logging
[tasks/task-P08T01-structured-logging.md](tasks/task-P08T01-structured-logging.md)

### P08T02 — Seccomp Filter
[tasks/task-P08T02-seccomp-filter.md](tasks/task-P08T02-seccomp-filter.md)

### P08T03 — Linux Namespaces
[tasks/task-P08T03-linux-namespaces.md](tasks/task-P08T03-linux-namespaces.md)

### P08T04 — WASM Signing
[tasks/task-P08T04-wasm-signing.md](tasks/task-P08T04-wasm-signing.md)
