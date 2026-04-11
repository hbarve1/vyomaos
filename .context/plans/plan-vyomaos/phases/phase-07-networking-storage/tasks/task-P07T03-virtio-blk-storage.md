# P07T03 — virtio-blk-storage

## Phase

Phase 07 — Networking & Storage

## Goal

Attach a virtio-blk persistent storage device to the QEMU VM by updating the launch script in `base/modules/qemu.sh`, and add a `make data-img` Makefile target that creates a 64 MB ext4-formatted disk image (`data.img`) that backs the virtio-blk device.

## File to create / modify

```
base/modules/qemu.sh   (modify — add -drive virtio-blk flags)
Makefile               (modify — add data-img target)
base/kernel.config     (modify — add CONFIG_VIRTIO_BLK and ext4 options)
```

## Implementation

### `base/kernel.config` additions

The block driver and filesystem support must be compiled into the kernel:

```
# ── virtio block device ──────────────────────────────────────────────────────
CONFIG_VIRTIO_BLK=y
CONFIG_BLK_DEV=y

# ── ext4 filesystem ──────────────────────────────────────────────────────────
CONFIG_EXT4_FS=y
CONFIG_EXT4_USE_FOR_EXT2=y
CONFIG_JBD2=y
CONFIG_FS_MBCACHE=y

# ── Block layer essentials ────────────────────────────────────────────────────
CONFIG_BLOCK=y
CONFIG_IOSCHED_DEADLINE=y
CONFIG_DEFAULT_IOSCHED="deadline"
```

`CONFIG_VIRTIO_BLK=y` is the driver for the `/dev/vda` device that QEMU exposes. `CONFIG_EXT4_FS=y` allows mounting the data image once the device appears. Both `CONFIG_VIRTIO_PCI=y` and `CONFIG_PCI=y` (added in P07T01) are prerequisites for `CONFIG_VIRTIO_BLK`.

---

### `base/modules/qemu.sh` — updated boot function

Add a `-drive` flag that attaches the `data.img` file as a virtio-blk device. The host file is treated as a raw block device from the guest's perspective.

```sh
#!/usr/bin/env bash
# VyomaOS QEMU Module

source "$(dirname "${BASH_SOURCE[0]}")/../config.sh"

DATA_IMG="${DATA_IMG:-$(dirname "${BASH_SOURCE[0]}")/../../data.img}"

boot_system() {
    [[ -f "$KERNEL_FILE" && -f "$INITRAMFS_FILE" ]] || {
        log_error "Missing build files. Run: ./vyomaos.sh build"
        return 1
    }

    # Auto-create data.img if it doesn't exist (non-destructive)
    if [[ ! -f "$DATA_IMG" ]]; then
        log_info "data.img not found — creating a blank 64 MB image at $DATA_IMG"
        dd if=/dev/zero of="$DATA_IMG" bs=1M count=64 status=none
        mkfs.ext4 -q -L vyoma-data "$DATA_IMG"
        log_success "data.img created."
    fi

    # Determine memory size from initramfs
    initramfs_size=$(du -m "$INITRAMFS_FILE" | cut -f1)
    if [ "$initramfs_size" -gt 10 ]; then
        memory="1G"
        log_info "Booting VyomaOS with large initramfs (${initramfs_size}MB)..."
    else
        memory="512M"
        log_info "Booting VyomaOS..."
    fi

    qemu-system-x86_64 \
        -kernel "$KERNEL_FILE" \
        -initrd "$INITRAMFS_FILE" \
        -cpu qemu64 \
        -m "$memory" \
        -accel tcg \
        -netdev user,id=net0,hostfwd=tcp::8080-:8080 \
        -device virtio-net-pci,netdev=net0 \
        -drive file="$DATA_IMG",if=none,id=blk0,format=raw \
        -device virtio-blk-pci,drive=blk0 \
        -append "console=ttyS0 loglevel=3 ip=dhcp" \
        -nographic

    log_success "Session completed."
}
```

Key flags:
- `-drive file=data.img,if=none,id=blk0,format=raw` — registers `data.img` as a drive named `blk0` without attaching a bus.
- `-device virtio-blk-pci,drive=blk0` — attaches `blk0` to the PCI bus as a virtio block device; appears as `/dev/vda` inside the guest.

---

### `Makefile` — `data-img` target

```makefile
DATA_IMG ?= data.img
DATA_IMG_SIZE_MB ?= 64

.PHONY: data-img clean-data-img

## Create a 64 MB ext4 disk image for virtio-blk storage
data-img: $(DATA_IMG)

$(DATA_IMG):
	@echo "[make] Creating $(DATA_IMG_SIZE_MB) MB ext4 image: $(DATA_IMG)"
	dd if=/dev/zero of=$(DATA_IMG) bs=1M count=$(DATA_IMG_SIZE_MB) status=none
	mkfs.ext4 -q -L vyoma-data $(DATA_IMG)
	@echo "[make] $(DATA_IMG) created ($(DATA_IMG_SIZE_MB) MB, ext4, label=vyoma-data)"

## Remove the data image (WARNING: destroys all stored data)
clean-data-img:
	@echo "[make] Removing $(DATA_IMG)"
	rm -f $(DATA_IMG)
```

The `DATA_IMG` and `DATA_IMG_SIZE_MB` variables can be overridden on the command line:

```sh
make data-img DATA_IMG=/tmp/test-data.img DATA_IMG_SIZE_MB=128
```

---

### Guest-side mount (handled by supervisor, documented in P07T04)

Once the guest boots and `/dev/vda` appears, the supervisor (or an init script) must mount it before apps with `filesystem = true` are launched:

```sh
mkdir -p /data
mount -t ext4 /dev/vda /data
```

This is implemented in P07T04 as part of the supervisor's pre-launch setup.

---

### `.gitignore` entry

Add `data.img` to the root `.gitignore` (or create one if absent) to prevent a 64 MB binary from being accidentally committed:

```gitignore
data.img
*.img
```

## Notes

- `format=raw` in the QEMU `-drive` flag means the image file is treated as a raw binary block device with no QEMU metadata wrapper. `mkfs.ext4` writes directly to byte offset 0, which is exactly what the virtio-blk driver expects.
- Using `if=none` + a separate `-device virtio-blk-pci` is the modern QEMU idiom (vs. the legacy `if=virtio` shorthand). It gives explicit control over which bus the device appears on.
- The auto-creation logic in `qemu.sh` is a convenience for developers who clone the repo and run `./vyomaos.sh run` without first running `make data-img`. The `make data-img` target is the canonical way to create it in CI.
- `mkfs.ext4` must be installed on the host build machine. On macOS it is available via `brew install e2fsprogs`. In Docker the build environment should include `e2fsprogs`.
- The `-L vyoma-data` label allows the guest to mount by label (`mount LABEL=vyoma-data /data`) rather than by device path, which is more robust if additional drives are added later.
- `CONFIG_EXT4_USE_FOR_EXT2=y` allows the ext4 driver to also mount ext2 and ext3 images, which is useful during testing.
- Cross-reference: P07T04 mounts `/dev/vda` at `/data` and deploys the `store` WASM app that uses the filesystem.

## Verification

```sh
# 1. Confirm kernel.config has virtio-blk and ext4 options
grep -q "^CONFIG_VIRTIO_BLK=y" base/kernel.config && echo "CONFIG_VIRTIO_BLK: set"
grep -q "^CONFIG_EXT4_FS=y"    base/kernel.config && echo "CONFIG_EXT4_FS: set"
grep -q "^CONFIG_BLK_DEV=y"    base/kernel.config && echo "CONFIG_BLK_DEV: set"

# 2. Confirm qemu.sh has virtio-blk-pci device
grep -q "virtio-blk-pci" base/modules/qemu.sh && echo "qemu.sh virtio-blk-pci: present"
grep -q "data.img"        base/modules/qemu.sh && echo "qemu.sh data.img ref: present"

# 3. Run make data-img and verify the image
make data-img
test -f data.img && echo "data.img: created"

# 4. Verify image size is 64 MB
SIZE=$(stat -f%z data.img 2>/dev/null || stat -c%s data.img)
test "$SIZE" -eq $((64 * 1024 * 1024)) && echo "data.img: correct size (64 MB)"

# 5. Verify the image has a valid ext4 superblock
file data.img | grep -q "ext2 filesystem" && echo "data.img: valid ext4/ext2 filesystem"
# ext4 images are reported as "ext2 filesystem data" by file(1) on many systems

# 6. Mount the image on the host and verify it is writable
sudo mkdir -p /mnt/vyoma-data-test
sudo mount -o loop data.img /mnt/vyoma-data-test
sudo touch /mnt/vyoma-data-test/test-write
test -f /mnt/vyoma-data-test/test-write && echo "data.img: writable"
sudo rm /mnt/vyoma-data-test/test-write
sudo umount /mnt/vyoma-data-test
sudo rmdir /mnt/vyoma-data-test

# 7. Confirm Makefile clean-data-img target works
make clean-data-img
! test -f data.img && echo "clean-data-img: data.img removed"

# 8. Rebuild kernel with new config (confirm no build errors)
./vyomaos.sh build 2>&1 | grep -i "error" | grep -v "^#" | head -10
test -f base/output/bzImage && echo "kernel: rebuilt successfully"
```
