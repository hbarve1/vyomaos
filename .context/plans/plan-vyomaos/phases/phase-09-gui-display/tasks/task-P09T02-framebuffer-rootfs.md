# P09T02 — Framebuffer device in rootfs

## What
Verify `/dev/fb0` is accessible at boot and add a `display: bool` capability to
the manifest schema + supervisor.

## Changes

### supervisor/src/main.rs — Capabilities struct
```rust
#[serde(deny_unknown_fields)]
struct Capabilities {
    #[serde(default)] stdio: bool,
    #[serde(default)] filesystem: bool,
    #[serde(default)] network: bool,
    #[serde(default)] display: bool,   // ← new
}
```

### supervisor startup check
```rust
fn check_display() -> bool {
    std::path::Path::new("/dev/fb0").exists()
}
```

Log `[display] /dev/fb0 available` or `[display] /dev/fb0 not found — GUI disabled`
at supervisor startup, before spawning apps.

### Seccomp denylist update
The `ioctl` syscall (nr=16) must NOT be in the denylist (it isn't currently).
Verify the mmap syscall (nr=9) is also not denied. Both are needed for framebuffer
access. No changes expected — current denylist only blocks 8 specific syscalls.

## Gate
Boot log contains `[display] /dev/fb0 available`.
`FBIOGET_VSCREENINFO` ioctl succeeds from a test call in supervisor startup.
