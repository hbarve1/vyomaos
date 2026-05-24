// Unit tests for ipc::format_version

#[test]
fn format_version_basic() {
    assert_eq!(supervisor::ipc::format_version(0, 19, 0), "VyomaOS 0.19.0");
}

#[test]
fn format_version_one_zero() {
    assert_eq!(supervisor::ipc::format_version(1, 0, 0), "VyomaOS 1.0.0");
}

#[test]
fn format_version_patch() {
    assert_eq!(supervisor::ipc::format_version(0, 1, 42), "VyomaOS 0.1.42");
}

#[test]
fn format_version_prefix() {
    assert!(supervisor::ipc::format_version(0, 0, 1).starts_with("VyomaOS "));
}
