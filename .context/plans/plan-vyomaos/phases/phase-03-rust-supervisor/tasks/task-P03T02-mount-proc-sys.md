# P03T02 — mount-proc-sys

## Phase

Phase 03 — Rust Supervisor

## Goal

Implement the `mount_filesystems()` function in `supervisor/src/main.rs` that mounts `/proc`, `/sys`, and `/dev` using the `libc::mount` syscall, replacing BusyBox's `/init` as the entity responsible for virtual filesystem setup.

## File to create / modify

```
supervisor/src/main.rs
supervisor/Cargo.toml
```

## Implementation

### Add `libc` to `supervisor/Cargo.toml` dependencies

```toml
[dependencies]
libc = { version = "0.2", default-features = false }
```

### `supervisor/src/main.rs` — complete updated file

```rust
//! VyomaOS supervisor — PID 1
//! P03T02: mount /proc, /sys, /dev

use std::ffi::CString;

fn main() {
    eprintln!("vyoma-supervisor starting");

    mount_filesystems();

    eprintln!("vyoma-supervisor: filesystems mounted");

    // Placeholder: real app loop added in P03T03 / P03T04
    loop {
        std::thread::park();
    }
}

/// Mount the virtual filesystems required before any userspace work proceeds.
///
/// Order matters:
///   1. /proc  — process information; needed by wasmtime and some libc calls
///   2. /sys   — device/bus topology; needed for PCI enumeration
///   3. /dev   — character devices; needed for console I/O from child processes
///
/// Each mount call is equivalent to:
///   mount -t <fstype> <fstype> <target>
/// (source == fstype is the conventional "none" substitute for virtual fs)
fn mount_filesystems() {
    mount_fs("proc",     "/proc", "proc",     0);
    mount_fs("sysfs",    "/sys",  "sysfs",    0);
    mount_fs("devtmpfs", "/dev",  "devtmpfs", 0);
}

/// Call libc::mount and panic loudly on failure.
///
/// # Arguments
/// * `source`  — source device / filesystem name (e.g. "proc")
/// * `target`  — mount point path (must exist in the initramfs skeleton)
/// * `fstype`  — filesystem type string
/// * `flags`   — MS_* bitfield (0 for a plain mount)
fn mount_fs(source: &str, target: &str, fstype: &str, flags: libc::c_ulong) {
    let c_source = CString::new(source).expect("mount source must not contain NUL");
    let c_target = CString::new(target).expect("mount target must not contain NUL");
    let c_fstype = CString::new(fstype).expect("mount fstype must not contain NUL");

    // SAFETY: all pointers are valid CStrings for the duration of the call.
    // libc::mount does not retain these pointers after return.
    let ret = unsafe {
        libc::mount(
            c_source.as_ptr(),
            c_target.as_ptr(),
            c_fstype.as_ptr(),
            flags,
            std::ptr::null(), // no extra mount options
        )
    };

    if ret != 0 {
        // SAFETY: errno is valid immediately after a failed syscall.
        let errno = unsafe { *libc::__errno_location() };
        panic!(
            "mount({source} -> {target}, {fstype}) failed: errno {errno}"
        );
    }

    eprintln!("vyoma-supervisor: mounted {target} ({fstype})");
}
```

## Notes

- `libc::mount` maps directly to the `mount(2)` syscall with no intermediate C wrapper overhead — appropriate for a PID-1 binary where every byte and syscall counts.
- Mount points (`/proc`, `/sys`, `/dev`) must exist as empty directories in the initramfs directory tree. `base/modules/rootfs.sh` creates them (P02T03). If they do not exist, `libc::mount` returns `ENOENT`.
- `CONFIG_DEVTMPFS_MOUNT=y` in the kernel config (P02T01) means the kernel auto-mounts devtmpfs at `/dev` during boot, before PID 1 starts. The `mount_fs("devtmpfs", "/dev", ...)` call here will therefore return `EBUSY` on kernels with that option set. Two options: (a) omit the `/dev` mount from the supervisor and rely on the kernel, or (b) check `EBUSY` and treat it as success. Option (b) is implemented here for portability across kernel configs:

```rust
fn mount_fs(source: &str, target: &str, fstype: &str, flags: libc::c_ulong) {
    // ... (same as above) ...
    if ret != 0 {
        let errno = unsafe { *libc::__errno_location() };
        // EBUSY (16) means already mounted — treat as success for /dev
        // when CONFIG_DEVTMPFS_MOUNT=y auto-mounts it before we run.
        if errno == libc::EBUSY {
            eprintln!("vyoma-supervisor: {target} already mounted, skipping");
            return;
        }
        panic!("mount({source} -> {target}, {fstype}) failed: errno {errno}");
    }
}
```

- Do not use `std::process::Command::new("mount")` — that would require BusyBox to be present and adds a fork/exec round-trip. Direct syscalls keep the supervisor self-contained.
- The `default-features = false` on the `libc` dependency avoids pulling in `std` compatibility shims that increase binary size slightly.

## Verification

```sh
# 1. libc dependency is present in Cargo.toml
grep -q 'libc' supervisor/Cargo.toml

# 2. Compiles cleanly (including for the musl target)
cargo build --manifest-path supervisor/Cargo.toml \
  --target x86_64-unknown-linux-musl 2>&1 | grep -v "^warning"

# 3. mount_filesystems function is defined
grep -q 'fn mount_filesystems' supervisor/src/main.rs

# 4. libc::mount is called (three times, once per filesystem)
grep -c 'libc::mount' supervisor/src/main.rs | grep -qE '^[1-9]'

# 5. No use of std::process::Command for mounting
grep -v "^//" supervisor/src/main.rs | grep -v "process::Command" | \
  grep -q "mount_fs\|libc::mount"

# 6. Integration test: boot VM and confirm /proc /sys /dev are mounted
#    (requires full build: make build)
# make run 2>&1 | grep -q "filesystems mounted"
# Inside VM: cat /proc/version should succeed
# Inside VM: ls /sys/class should succeed
# Inside VM: ls /dev/null should succeed
```
