// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Filesystem mounting helpers (P03T02).

#[cfg(target_os = "linux")]
use std::ffi::CString;

use crate::{log_info, log_warn, log_error};
use supervisor::logging::Subsystem;

pub fn mount_filesystems() {
    mount_fs("proc",     "/proc", "proc",     0);
    mount_fs("sysfs",    "/sys",  "sysfs",    0);
    mount_fs("devtmpfs", "/dev",  "devtmpfs", 0);
    #[cfg(target_os = "linux")]
    {
        let opts = c"trans=virtio,version=9p2000.L";
        let ret = mount_fs_with_data("vyoma-data", "/data", "9p", 0, opts.as_ptr());
        if !ret {
            log_warn!(Subsystem::Lifecycle, None, "9P share not available — /data will be empty tmpfs");
            mount_fs("tmpfs", "/data", "tmpfs", 0);
        }
    }
}

pub fn mount_fs(_source: &str, target: &str, fstype: &str, _flags: libc::c_ulong) {
    #[cfg(target_os = "linux")]
    {
        let source = _source;
        let c_source = CString::new(source).expect("mount source contains NUL");
        let c_target = CString::new(target).expect("mount target contains NUL");
        let c_fstype = CString::new(fstype).expect("mount fstype contains NUL");

        let ret = unsafe {
            libc::mount(
                c_source.as_ptr(),
                c_target.as_ptr(),
                c_fstype.as_ptr(),
                _flags,
                std::ptr::null(),
            )
        };

        if ret != 0 {
            let err = std::io::Error::last_os_error();
            let errno = err.raw_os_error().unwrap_or(0);
            if errno == libc::EBUSY {
                log_info!(Subsystem::Lifecycle, None, "{target} already mounted, skipping");
                return;
            }
            panic!("mount({source} -> {target}, {fstype}) failed: {err}");
        }

        log_info!(Subsystem::Lifecycle, None, "mounted {target} ({fstype})");
    }

    #[cfg(not(target_os = "linux"))]
    log_info!(Subsystem::Lifecycle, None, "[dev build] skipping mount {target} ({fstype})");
}

#[cfg(target_os = "linux")]
pub fn mount_fs_with_data(
    source: &str,
    target: &str,
    fstype: &str,
    flags: libc::c_ulong,
    data: *const libc::c_char,
) -> bool {
    let c_source = CString::new(source).expect("source NUL");
    let c_target = CString::new(target).expect("target NUL");
    let c_fstype = CString::new(fstype).expect("fstype NUL");

    let ret = unsafe {
        libc::mount(
            c_source.as_ptr(),
            c_target.as_ptr(),
            c_fstype.as_ptr(),
            flags,
            data as *const libc::c_void,
        )
    };

    if ret == 0 {
        log_info!(Subsystem::Lifecycle, None, "mounted {target} ({fstype})");
        true
    } else {
        let err = std::io::Error::last_os_error();
        log_error!(Subsystem::Lifecycle, None, "mount({source} -> {target}, {fstype}) failed: {err}");
        false
    }
}
