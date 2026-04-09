# P11T01 — QEMU Network Config

## Phase
Phase 11 — Networking

## Goal
Add `run-net` and `run-gui-net` Makefile targets that attach a QEMU user-mode network with host port forwarding. Verify `eth0` appears inside the VM at boot.

## Files to modify

```
Makefile
base/modules/scripts/boot.toml    — add http-server entry
```

## Implementation

### Makefile additions

```makefile
# ── run-net (headless + network) ─────────────────────────────────────────────
run-net: $(BZIMAGE) $(INITRAMFS) data
    qemu-system-x86_64 \
      -kernel $(BZIMAGE) \
      -initrd $(INITRAMFS) \
      -append "console=ttyS0 panic=1" \
      -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
      -device virtio-net-pci,netdev=net0 \
      -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
      -nographic -m 512M -no-reboot $(KVM)

# ── run-gui-net (GUI + network) ───────────────────────────────────────────────
run-gui-net: $(BZIMAGE) $(INITRAMFS) data
    qemu-system-x86_64 \
      -kernel $(BZIMAGE) \
      -initrd $(INITRAMFS) \
      -append "console=tty0 console=ttyS0 panic=1" \
      -vga virtio \
      -display $(DISPLAY_BACKEND) \
      -serial stdio \
      -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
      -device virtio-net-pci,netdev=net0 \
      -virtfs local,path=$(DATA_DIR),mount_tag=vyoma-data,security_model=mapped-xattr \
      -m 512M -no-reboot $(KVM)
```

### boot.toml addition

```toml
[[apps]]
manifest = "/apps/http-server/vyoma.toml"
restart  = "never"
```

## Notes

- QEMU user-mode networking (SLiRP) requires no root and no TAP device — works on macOS and Linux out of the box
- `hostfwd=tcp::8080-:8080` maps `localhost:8080` on the host to port `8080` inside the VM
- The VM gets IP `10.0.2.15`, gateway `10.0.2.2`; no DHCP client is needed since wasmtime handles the socket directly via `-S tcplisten`
- `CONFIG_VIRTIO_NET=y` is already in `base/kernel.config`; no kernel rebuild needed

## Verification

```sh
make run-net
# Serial output should include:
#   virtio_net virtio1: eth0: ...
#   http-server: listening on 0.0.0.0:8080
# Then from host:
curl -s http://localhost:8080/health
```
