# Phase 09 — GUI Display

## Goal

Give VyomaOS a graphical display. The kernel initialises a DRM/KMS framebuffer via
virtio-gpu. The supervisor owns `/dev/fb0` and implements a `vyoma:display` host
interface that WASM apps call to draw pixels, text, and rectangles. No full windowing
system is required — the supervisor acts as the display server, multiplexing a single
framebuffer between apps by assigning each a region (tiled layout) or by running one
fullscreen app at a time.

## Architecture

```
QEMU -device virtio-gpu-pci
  └─ Kernel DRM + virtio-gpu driver → /dev/fb0 (linear RGBA framebuffer)
       └─ Supervisor opens /dev/fb0 via mmap
            └─ vyoma:display WIT interface (host functions in Wasmtime)
                 ├─ fill_rect(x, y, w, h, color)
                 ├─ draw_text(x, y, text, color)
                 ├─ blit(x, y, pixels: &[u8])
                 └─ flush()
                      └─ WASM app calls display API → supervisor writes pixels → screen
```

## Why this approach (not a full Wayland compositor)

- WASM apps use WASI for all I/O; they cannot link against GTK/SDL/Wayland libraries
- A Wayland compositor inside VyomaOS would require a full display stack (libdrm,
  libwayland, etc.) compiled to musl, adding ~30 MB to the initramfs
- The `vyoma:display` WIT interface is ~5 host functions, fits the existing capability
  manifest model, and keeps the initramfs small
- Wayland can be revisited once the Component Model layer is mature

## Gate

```sh
# QEMU boots with a graphical window
qemu-system-x86_64 \
  -kernel out/bzImage -initrd out/initramfs.cpio.gz \
  -device virtio-gpu-pci \
  -display sdl \
  -append "console=ttyS0"

# gui-demo WASM app draws a coloured rectangle and "Hello VyomaOS" text
# visible in the QEMU window — no kernel panic, no SIGSYS from seccomp
```

## Dependencies

- Phase 03 (supervisor owns process lifecycle)
- Phase 05 (capability manifests — `display: true` gates framebuffer access)
- Phase 08 (seccomp denylist must allow ioctl/mmap for /dev/fb0)

## Kernel config additions

```
CONFIG_DRM=y
CONFIG_DRM_VIRTIO_GPU=y
CONFIG_FB=y
CONFIG_FB_VESA=y
CONFIG_FRAMEBUFFER_CONSOLE=y
CONFIG_FRAMEBUFFER_CONSOLE_DETECT_PRIMARY=y
```

## Tasks

### P09T01 — Kernel DRM + virtio-gpu config
[tasks/task-P09T01-kernel-drm-virtio-gpu.md](tasks/task-P09T01-kernel-drm-virtio-gpu.md)

Enable `CONFIG_DRM`, `CONFIG_DRM_VIRTIO_GPU`, `CONFIG_FB`, `CONFIG_FRAMEBUFFER_CONSOLE`
in `base/kernel.config`. Add `-device virtio-gpu-pci -display sdl` to the `make run`
QEMU command. Gate: QEMU opens a graphical window on `make run`.

### P09T02 — Framebuffer device in rootfs
[tasks/task-P09T02-framebuffer-rootfs.md](tasks/task-P09T02-framebuffer-rootfs.md)

Ensure `/dev/fb0` is accessible at boot. `CONFIG_DEVTMPFS_MOUNT=y` already populates
`/dev`, so this is mainly a verification task. Add `/dev/fb0` mmap test in supervisor
startup to confirm the device is present before registering the display host functions.

### P09T03 — `vyoma:display` WIT interface definition
[tasks/task-P09T03-wit-display-interface.md](tasks/task-P09T03-wit-display-interface.md)

Define `vyoma:display/canvas` WIT interface:

```wit
interface canvas {
  fill-rect: func(x: u32, y: u32, w: u32, h: u32, rgba: u32);
  draw-text: func(x: u32, y: u32, text: string, rgba: u32);
  blit:      func(x: u32, y: u32, w: u32, h: u32, pixels: list<u8>);
  flush:     func();
  width:     func() -> u32;
  height:    func() -> u32;
}

world display-app {
  import vyoma:display/canvas;
  export run: func();
}
```

Generate Rust bindings via `wit-bindgen`. Add `display: bool` to the `Capabilities`
struct in the supervisor and the manifest schema.

### P09T04 — Supervisor framebuffer host implementation
[tasks/task-P09T04-supervisor-fb-host.md](tasks/task-P09T04-supervisor-fb-host.md)

In `supervisor/src/main.rs` (or a new `display.rs` module):
- Open `/dev/fb0`, read screen dimensions from `FBIOGET_VSCREENINFO` ioctl
- `mmap` the framebuffer for direct pixel writes
- Register the `vyoma:display/canvas` host functions with the Wasmtime `Linker`
- Implement a PSF2 font renderer for `draw_text` (embed a minimal 8×16 bitmap font
  as a `&[u8]` constant — no file I/O at runtime)
- `flush()` triggers an `FBIOPAN_DISPLAY` ioctl to push the buffer to screen
- Seccomp denylist update: allow `ioctl` for FB device fds

### P09T05 — `gui-demo` WASM app
[tasks/task-P09T05-gui-demo-app.md](tasks/task-P09T05-gui-demo-app.md)

New `apps/gui-demo/` crate targeting `wasm32-wasip2` + the `vyoma:display` component
world. The app:
1. Clears the screen to a dark background
2. Draws a solid rectangle in VyomaOS brand colour
3. Renders "Hello VyomaOS" using the embedded font
4. Shows per-app output lines from the IPC broker as a simple log view
5. Calls `flush()` and exits (static frame — animation comes in a later phase)
