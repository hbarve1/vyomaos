# Phase 04 — WASM Runtime (Real Wasmtime + WASI Preview 2)

## Goal

Replace the mock wasmtime wrapper script with a real, statically-linked Wasmtime binary that supports WASI Preview 2. Integrate it into the Rust supervisor so apps execute with proper WASI stdio, args, and exit codes. This completes the **minimal working prototype**.

## Gate

```sh
# Boot and see real WASM app output (not mock)
qemu-system-x86_64 -kernel out/bzImage -initrd out/initramfs.cpio.gz \
  -append "console=ttyS0 quiet" -nographic -m 256M 2>&1 | \
  grep "Hello from"

# Wasmtime binary is static musl
file out/rootfs/usr/bin/wasmtime | grep "statically linked"

# WASI Preview 2 component runs
echo '(component)' > /tmp/empty.wat
wasmtime compile --target wasm32-wasip2 /tmp/empty.wat -o /tmp/empty.wasm 2>&1 | grep -v "error"
```

## Dependencies

- Phase 03 (supervisor is PID 1 and can execute child processes)
- Wasmtime release binary for `x86_64-unknown-linux-musl`
- Rust `wasm32-wasip2` target (`rustup target add wasm32-wasip2`)

## Tasks

### P04T01 — Wasmtime Static Binary
[task-P04T01-wasmtime-static-binary.md](tasks/task-P04T01-wasmtime-static-binary.md)

### P04T02 — WASI Preview 2 Config
[task-P04T02-wasi-preview2-config.md](tasks/task-P04T02-wasi-preview2-config.md)

### P04T03 — Supervisor Runtime Integration
[task-P04T03-supervisor-runtime-integration.md](tasks/task-P04T03-supervisor-runtime-integration.md)
