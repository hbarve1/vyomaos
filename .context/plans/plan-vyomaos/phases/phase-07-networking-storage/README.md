# Phase 07 — Networking & Storage

## Goal

Add virtio-net to the kernel config so the VM has a network interface, and expose it to WASM apps via `wasi:sockets`. Add virtio-blk for a writable block device and expose it via `wasi:filesystem`. Apps that declare these capabilities in their manifest gain access; apps that don't are isolated.

## Gate

```sh
# VM gets a network interface
qemu-system-x86_64 -kernel out/bzImage -initrd out/initramfs.cpio.gz \
  -netdev user,id=net0 -device virtio-net-pci,netdev=net0 \
  -append "console=ttyS0 quiet" -nographic -m 256M 2>&1 | grep "eth0"

# WASM app with sockets capability can connect to host
# (server.wasm connects to host port 8080 and reads response)

# WASM app with filesystem capability can write a file
# (store.wasm writes /data/test.txt and reads it back)
```

## Dependencies

- Phase 05 (capability manifests — gates which apps get network/storage)
- Phase 06 (multi-app supervisor running concurrently)
- Kernel config from Phase 02 (extend, don't replace)

## Tasks

### P07T01 — Virtio-net Kernel Config
[task-P07T01-virtio-net-kernel-config.md](tasks/task-P07T01-virtio-net-kernel-config.md)

### P07T02 — WASI Sockets Integration
[task-P07T02-wasi-sockets-integration.md](tasks/task-P07T02-wasi-sockets-integration.md)

### P07T03 — Virtio-blk Storage
[task-P07T03-virtio-blk-storage.md](tasks/task-P07T03-virtio-blk-storage.md)

### P07T04 — WASI Filesystem Integration
[task-P07T04-wasi-filesystem-integration.md](tasks/task-P07T04-wasi-filesystem-integration.md)
