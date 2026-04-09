# P02T02 — virtio-9p-config

## Phase

Phase 02 — Kernel Hardening

## Goal

Extend `base/kernel.config` with the virtio transport, virtio-blk, virtio-net, and 9P filesystem options so that QEMU virtio devices work and host directories can be shared into the VM via 9P passthrough.

## File to create / modify

```
base/kernel.config
```

## Implementation

Append the following block to the existing `base/kernel.config` created in P02T01. The options are grouped by subsystem for readability.

```
# ────────────────────────────────────────────────────────────────────────────
# PHASE 02 TASK 02 — VirtIO + 9P additions
# Append to base/kernel.config after the P02T01 block.
# ────────────────────────────────────────────────────────────────────────────

# ── VirtIO core ───────────────────────────────────────────────────────────────
CONFIG_VIRTIO=y
CONFIG_VIRTIO_MENU=y

# ── VirtIO transport: PCI (standard for x86_64 QEMU) ─────────────────────────
CONFIG_VIRTIO_PCI=y
CONFIG_VIRTIO_PCI_LEGACY=y          # keeps compatibility with older QEMU

# ── VirtIO block device (virtio-blk) ─────────────────────────────────────────
# Enables /dev/vda, /dev/vdb ... block devices passed via -drive if=virtio
CONFIG_VIRTIO_BLK=y

# ── VirtIO network device (virtio-net) ────────────────────────────────────────
# Enables the NIC presented by QEMU -netdev type=user,... -device virtio-net
CONFIG_VIRTIO_NET=y

# ── VirtIO random number generator ───────────────────────────────────────────
# Provides /dev/hwrng; some WASM runtimes call getrandom() at startup
CONFIG_HW_RANDOM=y
CONFIG_HW_RANDOM_VIRTIO=y

# ── 9P client (Plan 9 filesystem protocol) ───────────────────────────────────
# Required for QEMU -fsdev / -device virtio-9p-pci host-directory sharing
CONFIG_NET_9P=y
CONFIG_NET_9P_VIRTIO=y
CONFIG_9P_FS=y
# Enable POSIX ACL support on 9P shares (needed for some app permissions)
CONFIG_9P_FS_POSIX_ACL=y

# ── VirtIO SCSI (optional but common in QEMU) ────────────────────────────────
# CONFIG_VIRTIO_SCSI is not set     # omit to keep image small; add if needed
```

### QEMU invocation to use these features

After applying this config, the `run` target in the Makefile (P01T01) can be extended to pass virtio devices:

```sh
qemu-system-x86_64 \
  -kernel out/bzImage \
  -initrd out/initramfs.cpio.gz \
  -append "console=ttyS0 panic=1" \
  -nographic \
  -m 512M \
  -no-reboot \
  # virtio-net (user-mode networking, no root required on host)
  -netdev user,id=net0 \
  -device virtio-net-pci,netdev=net0 \
  # virtio-blk (pass a raw disk image)
  # -drive file=disk.img,if=virtio,format=raw \
  # 9p host share: mount with "mount -t 9p -o trans=virtio hostshare /mnt"
  -fsdev local,id=hostfs,path="$(pwd)",security_model=none \
  -device virtio-9p-pci,fsdev=hostfs,mount_tag=hostshare
```

## Notes

- `CONFIG_VIRTIO_PCI_LEGACY=y` preserves compatibility with QEMU versions that default to `virtio-pci-non-transitional`. Remove it when you control the exact QEMU version.
- `CONFIG_NET_9P_VIRTIO=y` depends on both `CONFIG_NET_9P=y` and `CONFIG_VIRTIO_PCI=y` being set. The kernel Kconfig system enforces this, but list it explicitly to make the dependency visible to humans reading the file.
- `CONFIG_9P_FS_POSIX_ACL=y` is optional; omit it if the WASM apps never rely on ACL-based permissions. It adds a small amount to the kernel image.
- `CONFIG_HW_RANDOM_VIRTIO=y` is worth including because Wasmtime calls `getrandom()` during initialization and an RNG device prevents blocking.
- All options here are built-in (`=y`), not modules, consistent with the `CONFIG_MODULES is not set` decision from P02T01.

## Verification

```sh
# 1. All new options are present and set to =y in the config file
for opt in CONFIG_VIRTIO CONFIG_VIRTIO_PCI CONFIG_VIRTIO_BLK \
           CONFIG_VIRTIO_NET CONFIG_NET_9P CONFIG_NET_9P_VIRTIO \
           CONFIG_9P_FS; do
  grep -q "^${opt}=y" base/kernel.config || \
    { echo "MISSING: $opt"; exit 1; }
done

# 2. No duplicate CONFIG_ keys exist
sort base/kernel.config | grep "^CONFIG_" | cut -d= -f1 | uniq -d | \
  tee /tmp/dups.txt
test ! -s /tmp/dups.txt

# 3. Rebuild kernel with updated config and verify bzImage produced
# make kernel
# test -f out/bzImage

# 4. Boot and confirm virtio-net device appears (integration test)
# make run &
# QEMU_PID=$!
# sleep 20
# # In the QEMU serial console, expect to see "virtio_net" in dmesg
# kill $QEMU_PID

# 5. 9P share is accessible inside the VM (integration test)
# (Requires full rootfs with mount utility — available after P02T03)
# Boot VM, then inside:
#   mount -t 9p -o trans=virtio hostshare /mnt
#   ls /mnt   # should show project root files
```
