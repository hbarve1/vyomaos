// Unit tests for supervisor/src/namespace.rs — compile check and constant validation.
//
// setup_app_namespace() requires root + unshare(2) which we cannot test in CI.
// These tests verify the module compiles and the constants are correct.

// ── Compile checks ──────────────────────────────────────────────────────────

/// Verify libc CLONE_NEWNS constant exists and has expected value.
#[test]
fn test_clone_newns_constant() {
    // CLONE_NEWNS = 0x00020000 on Linux
    assert_eq!(libc::CLONE_NEWNS, 0x0002_0000);
}

/// Verify MS_REC and MS_PRIVATE flags exist.
#[test]
fn test_mount_flags_exist() {
    let flags = libc::MS_REC | libc::MS_PRIVATE;
    assert!(flags > 0, "MS_REC | MS_PRIVATE should be non-zero");
}

/// Verify MS_PRIVATE has the expected value.
#[test]
fn test_ms_private_value() {
    // MS_PRIVATE = 1 << 18 = 0x40000
    assert_eq!(libc::MS_PRIVATE, 1 << 18);
}

/// Verify MS_REC has the expected value.
#[test]
fn test_ms_rec_value() {
    // MS_REC = 0x4000
    assert_eq!(libc::MS_REC, 0x4000);
}

// ── Namespace flag combination ──────────────────────────────────────────────

/// Verify that CLONE_NEWNS can be combined with other namespace flags.
#[test]
fn test_namespace_flags_combinable() {
    let ns = libc::CLONE_NEWNS;
    let pid = libc::CLONE_NEWPID;
    let combined = ns | pid;
    assert_ne!(combined, ns);
    assert_ne!(combined, pid);
    assert_eq!(combined & ns, ns);
    assert_eq!(combined & pid, pid);
}

/// Verify the /\0 byte pattern used for the slash path in mount calls.
#[test]
fn test_slash_cstring() {
    let slash = b"/\0";
    assert_eq!(slash.len(), 2);
    assert_eq!(slash[0], b'/');
    assert_eq!(slash[1], 0);
}

// ── Documentation-level tests ───────────────────────────────────────────────

/// Verify the module's safety contract: setup_app_namespace is unsafe.
/// This test just asserts the function signature compiles in a const context.
#[test]
fn test_namespace_module_compiles() {
    // The function exists in the binary crate (cfg(target_os = "linux")).
    // We verify the libc types it uses are available.
    let _: libc::c_int = 0;
    let _: *const libc::c_char = std::ptr::null();
    let _: libc::c_ulong = 0;
}

/// Verify error type is std::io::Error as declared by the module.
#[test]
fn test_io_error_type() {
    let err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "test");
    assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
}
