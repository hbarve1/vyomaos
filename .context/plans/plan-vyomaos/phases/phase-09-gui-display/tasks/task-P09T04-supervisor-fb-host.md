# P09T04 — Supervisor framebuffer host implementation

## What
Implement the `vyoma:display/canvas` host functions in the supervisor.  The
supervisor mmaps `/dev/fb0` and exposes drawing primitives as Wasmtime host
functions.  A minimal PSF2 8×16 bitmap font is embedded at compile time for
`draw_text`.

## Key implementation details

### Opening the framebuffer
```rust
use std::os::unix::io::AsRawFd;

struct Framebuffer {
    ptr:    *mut u8,   // mmap base
    size:   usize,     // mmap length
    width:  u32,
    height: u32,
    pitch:  u32,       // bytes per row
    bpp:    u32,       // bits per pixel (expect 32 for virtio-gpu)
}

impl Framebuffer {
    fn open() -> io::Result<Self> {
        let file = File::open("/dev/fb0")?;
        let fd   = file.as_raw_fd();

        // FBIOGET_VSCREENINFO = 0x4600
        // FBIOGET_FSCREENINFO = 0x4602
        // Use libc::ioctl to read fb_var_screeninfo
        ...
    }
}
```

### Pixel write helper
```rust
fn put_pixel(fb: &Framebuffer, x: u32, y: u32, rgba: u32) {
    if x >= fb.width || y >= fb.height { return; }
    let offset = (y * fb.pitch + x * (fb.bpp / 8)) as usize;
    // virtio-gpu uses XRGB8888: map rgba -> 0x00RRGGBB
    let [r, g, b, _a] = rgba.to_be_bytes();
    unsafe {
        let p = fb.ptr.add(offset);
        *p       = b;
        *p.add(1) = g;
        *p.add(2) = r;
        *p.add(3) = 0;
    }
}
```

### Embedded bitmap font
Embed a public-domain 8×16 PSF2 font (e.g. from the Linux kernel's `lib/fonts/`):
```rust
static FONT_8X16: &[u8] = include_bytes!("../assets/font_8x16.psf2");
```
Parse the PSF2 header at startup to locate glyph data.  `draw_text` iterates each
character, looks up its 16-byte glyph bitmap, and plots set bits as foreground pixels.

### Registering host functions with Wasmtime
Switch supervisor from `wasmtime::Engine + Process::Command` to
`wasmtime::Engine + wasmtime::component::Component + wasmtime::Linker` for
display-capable apps. Non-display apps continue to use the existing `Command`-based
launch path.

## Gate
`gui-demo.wasm` fills a rectangle and renders text visible in the QEMU SDL window.
No kernel panic, no SIGSYS from seccomp (ioctl + mmap are not in the denylist).

## Notes
- The `mmap`/`munmap` calls happen in the supervisor process, not inside the WASM
  sandbox, so they are not subject to WASI capability restrictions.
- Thread safety: wrap `Framebuffer` in `Arc<Mutex<Framebuffer>>` if multiple display
  apps run concurrently (tiled layout — future work).
