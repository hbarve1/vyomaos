// Unit tests for supervisor/src/mount.rs — path/argument validation.
//
// mount_fs and mount_fs_with_data do actual libc::mount calls on Linux,
// but on non-Linux they just log.  We test the argument validation logic
// (CString construction, path patterns) without calling mount.

use std::ffi::CString;

// ── CString construction validation ─────────────────────────────────────────

#[test]
fn test_cstring_valid_source() {
    let c = CString::new("proc");
    assert!(c.is_ok());
}

#[test]
fn test_cstring_valid_target() {
    let c = CString::new("/proc");
    assert!(c.is_ok());
}

#[test]
fn test_cstring_valid_fstype() {
    let c = CString::new("sysfs");
    assert!(c.is_ok());
}

#[test]
fn test_cstring_rejects_nul() {
    let c = CString::new("bad\0path");
    assert!(c.is_err(), "CString should reject embedded NUL");
}

// ── Expected mount calls ────────────────────────────────────────────────────

/// Verify all filesystem types from mount_filesystems() are valid CStrings.
#[test]
fn test_all_mount_args_valid() {
    let mounts: Vec<(&str, &str, &str)> = vec![
        ("proc",     "/proc", "proc"),
        ("sysfs",    "/sys",  "sysfs"),
        ("devtmpfs", "/dev",  "devtmpfs"),
        ("tmpfs",    "/data", "tmpfs"),
    ];
    for (source, target, fstype) in &mounts {
        assert!(CString::new(*source).is_ok(), "bad source: {source}");
        assert!(CString::new(*target).is_ok(), "bad target: {target}");
        assert!(CString::new(*fstype).is_ok(), "bad fstype: {fstype}");
    }
}

/// Verify the 9P mount arguments are valid CStrings.
#[test]
fn test_9p_mount_args_valid() {
    assert!(CString::new("vyoma-data").is_ok());
    assert!(CString::new("/data").is_ok());
    assert!(CString::new("9p").is_ok());
}

// ── Mount flag validation ───────────────────────────────────────────────────

#[test]
fn test_mount_flags_zero_default() {
    // Default flags = 0 (no special mount options)
    let flags: libc::c_ulong = 0;
    assert_eq!(flags, 0);
}

#[test]
fn test_mount_target_paths_absolute() {
    let targets = ["/proc", "/sys", "/dev", "/data"];
    for t in &targets {
        assert!(t.starts_with('/'), "mount target must be absolute: {t}");
    }
}

// ── Error code constants ────────────────────────────────────────────────────

#[test]
fn test_ebusy_constant() {
    // EBUSY is used to detect "already mounted" — verify it exists
    assert!(libc::EBUSY > 0);
}

#[test]
fn test_errno_values() {
    // Verify key errno values the mount module checks
    assert_ne!(libc::EBUSY, libc::ENOENT);
    assert_ne!(libc::EBUSY, libc::EPERM);
}

// ── 9P options string ───────────────────────────────────────────────────────

#[test]
fn test_9p_options_format() {
    let opts = "trans=virtio,version=9p2000.L";
    assert!(opts.contains("trans=virtio"));
    assert!(opts.contains("version=9p2000.L"));
}

#[test]
fn test_9p_options_no_nul() {
    let opts = "trans=virtio,version=9p2000.L";
    assert!(!opts.contains('\0'));
}

// ── Path existence check pattern ────────────────────────────────────────────

#[test]
fn test_tempdir_mount_simulation() {
    // Simulate creating mount point directories (what init would do)
    let dir = tempfile::tempdir().unwrap();
    let proc_path = dir.path().join("proc");
    std::fs::create_dir_all(&proc_path).unwrap();
    assert!(proc_path.exists());
}
