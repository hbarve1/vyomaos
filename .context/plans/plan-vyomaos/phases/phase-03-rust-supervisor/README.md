# Phase 03 — Rust Supervisor (PID 1)

## Goal

Replace the BusyBox shell `init` script with a Rust binary compiled to a static musl binary. The supervisor becomes PID 1: it mounts essential filesystems, discovers WASM apps, and spawns/monitors them. This is the **minimal working prototype gate** — after this phase, the OS boots without any shell.

## Gate

```sh
# OS boots with Rust binary as PID 1
qemu-system-x86_64 -kernel out/bzImage -initrd out/initramfs.cpio.gz \
  -append "console=ttyS0 quiet" -nographic -m 256M 2>&1 | \
  grep "vyoma-supervisor"

# No BusyBox sh invocation in boot path
grep -r "busybox" base/modules/scripts/init | wc -l | grep "^0$"

# Supervisor binary is static musl (no dynamic deps)
file out/rootfs/sbin/init | grep "statically linked"
```

## Dependencies

- Phase 01 (Makefile to integrate Rust build into the pipeline)
- Phase 02 (kernel boots reliably in QEMU)
- Rust toolchain with `x86_64-unknown-linux-musl` target installed

## Tasks

### P03T01 — Supervisor Crate Scaffold
[task-P03T01-supervisor-crate-scaffold.md](tasks/task-P03T01-supervisor-crate-scaffold.md)

### P03T02 — Mount proc/sys/devtmpfs
[task-P03T02-mount-proc-sys.md](tasks/task-P03T02-mount-proc-sys.md)

### P03T03 — App Discovery
[task-P03T03-app-discovery.md](tasks/task-P03T03-app-discovery.md)

### P03T04 — Process Lifecycle
[task-P03T04-process-lifecycle.md](tasks/task-P03T04-process-lifecycle.md)
