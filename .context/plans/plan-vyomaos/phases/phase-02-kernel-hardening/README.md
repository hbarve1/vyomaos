# Phase 02 — Kernel Hardening

## Goal

Shrink and harden the kernel config to the absolute minimum required to boot in QEMU with virtio drivers and 9p filesystem sharing. Eliminate all unnecessary drivers, modules, and debug features to reduce attack surface and binary size.

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
