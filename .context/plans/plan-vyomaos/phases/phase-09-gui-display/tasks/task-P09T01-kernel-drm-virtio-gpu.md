# P09T01 — Kernel DRM + virtio-gpu config

## What
Add DRM, virtio-gpu, and framebuffer console to `base/kernel.config`. Update the
`make run` QEMU command to attach a virtio-gpu device and open a display window.

## Changes

### base/kernel.config
```
CONFIG_DRM=y
CONFIG_DRM_VIRTIO_GPU=y
CONFIG_FB=y
CONFIG_FB_VESA=y
CONFIG_FRAMEBUFFER_CONSOLE=y
CONFIG_FRAMEBUFFER_CONSOLE_DETECT_PRIMARY=y
CONFIG_VGA_CONSOLE=y
CONFIG_DUMMY_CONSOLE=y
```

### Makefile — run target
```makefile
run: $(BZIMAGE) $(INITRAMFS)
    qemu-system-x86_64 \
      -kernel $(BZIMAGE) \
      -initrd $(INITRAMFS) \
      -append "console=ttyS0 panic=1" \
      -serial mon:stdio \
      -device virtio-gpu-pci \
      -display sdl \
      -m 512M \
      -no-reboot \
      $(KVM)
```

Keep `-serial mon:stdio` so the ttyS0 console still works for supervisor output.
`-display sdl` opens the graphical window on the host; requires SDL2 on the host
(`brew install sdl2` on macOS).

## Gate
`make run` opens a QEMU window alongside the serial console output.  No kernel
panic, framebuffer visible (even if blank).

## Notes
- Kernel rebuild required after config change (delete `out/bzImage`)
- `CONFIG_DRM_VIRTIO_GPU` depends on `CONFIG_DRM` and `CONFIG_VIRTIO_PCI` (already set)
- `CONFIG_FB` enables the legacy fbdev API which is simpler than full DRM for our use case
