# Phase 11 — Networking

## Goal

Add a live network interface to the VM and build an `http-server` WASM app that serves a status page over HTTP. The page lists every running app, the boot count, and recent IPC messages — all generated dynamically by the WASM app from data written to the 9P `/data` share. The page is reachable from the host machine at `http://localhost:8080`.

## Gate

```sh
# In one terminal — boot with network
make run-net

# In another terminal on the host — verify the status page
curl -s http://localhost:8080 | grep "VyomaOS"
curl -s http://localhost:8080 | grep "hello-world"
curl -s http://localhost:8080/health | grep "ok"
```

## Architecture

```
QEMU -netdev user,id=net0,hostfwd=tcp::8080-:8080
     -device virtio-net-pci,netdev=net0
  └─ kernel virtio-net driver → eth0 inside VM
       └─ wasmtime -S tcplisten=0.0.0.0:8080 http-server.wasm
            └─ WASM app: accept TCP → parse HTTP GET → write response
```

No glibc networking libs needed — wasmtime provides the WASI socket via the `-S tcplisten` flag, and the WASM app reads/writes it as a file descriptor.

## Dependencies

- Phase 05 (capability manifests — `network: true` already wired in supervisor)
- Phase 07 (9P `/data` share — status page reads boot_count.txt)
- Phase 09 (GUI — optional: http-server can also send VYOMA_DRAW lines)
- Kernel: `CONFIG_VIRTIO_NET=y` already present in `base/kernel.config`
- No new Rust crates needed (wasmtime handles socket setup via `-S tcplisten`)

## Makefile additions

```makefile
run-net: $(BZIMAGE) $(INITRAMFS) data
    qemu-system-x86_64 \
      -kernel $(BZIMAGE) -initrd $(INITRAMFS) \
      -append "console=ttyS0 panic=1" \
      -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
      -device virtio-net-pci,netdev=net0 \
      -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
      -nographic -m 512M -no-reboot $(KVM)

run-gui-net: $(BZIMAGE) $(INITRAMFS) data
    # GUI + network combined
```

## Tasks

### P11T01 — QEMU Network Config
[tasks/task-P11T01-qemu-network-config.md](tasks/task-P11T01-qemu-network-config.md)

Add `run-net` and `run-gui-net` Makefile targets. Add `http-server` to `boot.toml` and `apps` build list. Verify `eth0` appears in kernel boot messages.

### P11T02 — http-server WASM App
[tasks/task-P11T02-http-server-app.md](tasks/task-P11T02-http-server-app.md)

New `apps/http-server/` crate. Uses only `std::io` and `std::net` (wasmtime maps WASI sockets to these). Serves:
- `GET /` → HTML status page
- `GET /health` → `{"status":"ok"}`
- `GET /apps` → JSON array of app names from `/data/`

### P11T03 — Status Page Content
[tasks/task-P11T03-status-page.md](tasks/task-P11T03-status-page.md)

Status page reads `/data/boot_count.txt` and `/data/boot_log.txt` from the 9P share to show live boot history. HTML is hand-generated (no templating crate needed). Response includes `Content-Type: text/html` and `Content-Length` headers.

### P11T04 — Supervisor Network Capability Wiring
[tasks/task-P11T04-network-capability.md](tasks/task-P11T04-network-capability.md)

Verify `network: true` in `vyoma.toml` correctly passes `-S tcplisten=0.0.0.0:8080` to wasmtime. Audit log should show `net:yes` for http-server. Confirm apps without `network: true` cannot listen on ports (wasmtime enforces this).
