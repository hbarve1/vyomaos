// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P27: Per-app namespace isolation.
//!
//! Called inside `pre_exec` (before seccomp is applied) to place each wasmtime
//! child process into its own mount and PID namespace.  This prevents apps from
//! seeing each other's mounts or process trees.
//!
//! If the kernel lacks namespace support (or the supervisor is not running as
//! root), the calls fail gracefully with a logged warning -- the app still
//! launches without namespace isolation.

use std::io;

/// Set up mount and PID namespace isolation for the current process.
///
/// Must be called in a `pre_exec` context (after fork, before exec).
///
/// # Safety
/// This function calls libc `unshare` and `mount` -- it is intended to run
/// inside an `unsafe pre_exec` block where the child process has already
/// been forked.
pub unsafe fn setup_app_namespace() -> io::Result<()> {
    // CLONE_NEWNS  = new mount namespace
    // CLONE_NEWPID = new PID namespace (child will be PID 1 in its namespace)
    let flags = libc::CLONE_NEWNS | libc::CLONE_NEWPID;
    if libc::unshare(flags) < 0 {
        return Err(io::Error::last_os_error());
    }

    // Remount root as private so mount events don't propagate to the host.
    let none = std::ptr::null::<libc::c_char>();
    let slash = b"/\0".as_ptr() as *const libc::c_char;
    let ms_rec_private = libc::MS_REC | libc::MS_PRIVATE;
    if libc::mount(none, slash, none, ms_rec_private, std::ptr::null()) < 0 {
        // Non-fatal: unshare succeeded but remount failed; isolation is partial.
        return Err(io::Error::last_os_error());
    }

    Ok(())
}
