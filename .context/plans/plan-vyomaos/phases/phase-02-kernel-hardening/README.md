# Phase 02 — Kernel Hardening

**Status: complete**

## Goal

Shrink and harden the kernel config to the absolute minimum required to boot in QEMU with virtio drivers, 9p filesystem sharing, and DRM/virtio-gpu display. Eliminate all unnecessary drivers, modules, and debug features to reduce attack surface and binary size.

## What was actually built

- Switched from `tinyconfig` to `allnoconfig` in `kernel.sh` — tinyconfig silently dropped forced config options when transitive deps were unresolved; allnoconfig gives full deterministic control
- Added explicit transitive deps: `CONFIG_VIRTIO_RING=y`, `CONFIG_NETWORK_FILESYSTEMS=y`
- Added DRM/display stack: `CONFIG_DRM=y`, `CONFIG_DRM_VIRTIO_GPU=y`, `CONFIG_DRM_FBDEV_EMULATION=y`, `CONFIG_FRAMEBUFFER_CONSOLE=y`
- Final kernel size: 2.3 MB (well under 4 MB gate)

## Gate

```sh
# Kernel image under 4 MB
wc -c out/bzImage | awk '{if ($1 < 4000000) print "OK"; else print "FAIL"}'

# 9p filesystem is available at boot
qemu-system-x86_64 -kernel out/bzImage -initrd out/initramfs.cpio.gz \
  -virtfs local,path=$(pwd)/apps/build,mount_tag=apps,security_model=none \
  -append "console=ttyS0 quiet" -nographic -m 256M | grep "9p"

# No loadable modules
grep -c "=m" out/linux-*/arch/x86/configs/vyomaos_defconfig | grep "^0$"
```

## Dependencies

- Phase 01 (Makefile tracks kernel config as a dependency)
- QEMU installed on host

## Tasks

### P02T01 — Minimal Kernel Config
[task-P02T01-minimal-kernel-config.md](tasks/task-P02T01-minimal-kernel-config.md)

### P02T02 — Virtio + 9p Config
[task-P02T02-virtio-9p-config.md](tasks/task-P02T02-virtio-9p-config.md)

### P02T03 — Static Musl BusyBox
[task-P02T03-static-musl-busybox.md](tasks/task-P02T03-static-musl-busybox.md)
