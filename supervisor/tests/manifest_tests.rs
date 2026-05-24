// TDD RED: These tests fail until parse_manifest + validate_manifest are implemented.

use std::io::Write;
use tempfile::NamedTempFile;

fn write_toml(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(content.as_bytes()).expect("write");
    f
}

// (1) Valid TOML parses into AppManifest without error
#[test]
fn test_valid_manifest_parses_ok() {
    let f = write_toml(r#"
[app]
name    = "hello-world"
version = "0.1.0"
wasm    = "hello-world.wasm"

[capabilities]
stdio = true
"#);
    let result = supervisor::manifest::parse_manifest(f.path());
    assert!(result.is_ok(), "expected Ok, got: {:?}", result);
    let m = result.unwrap();
    assert_eq!(m.app.name, "hello-world");
}

// (2) TOML with unknown capability field returns Err
#[test]
fn test_unknown_capability_field_returns_err() {
    let f = write_toml(r#"
[app]
name    = "bad-app"
version = "0.1.0"
wasm    = "bad-app.wasm"

[capabilities]
stdio   = true
unknown = true
"#);
    let result = supervisor::manifest::parse_manifest(f.path());
    assert!(result.is_err(), "expected Err for unknown field, got Ok");
}

// (3) TOML missing app.name returns Err
#[test]
fn test_missing_app_name_returns_err() {
    let f = write_toml(r#"
[app]
version = "0.1.0"
wasm    = "app.wasm"

[capabilities]
"#);
    let result = supervisor::manifest::parse_manifest(f.path());
    assert!(result.is_err(), "expected Err for missing app.name, got Ok");
}

// (4) validate_manifest with duplicate name in registered_names returns Err
#[test]
fn test_duplicate_app_name_returns_err() {
    let f = write_toml(r#"
[app]
name    = "hello-world"
version = "0.1.0"
wasm    = "hello-world.wasm"
"#);
    let m = supervisor::manifest::parse_manifest(f.path())
        .expect("parse should succeed");
    let registered = &["hello-world"];
    let result = supervisor::manifest::validate_manifest(&m, registered);
    assert!(result.is_err(), "expected Err for duplicate name");
    let msg = result.unwrap_err();
    assert!(msg.contains("hello-world"), "error should mention the app name: {msg}");
}

// (5) watchdog_secs = 0 is valid (not an error)
#[test]
fn test_watchdog_zero_is_valid() {
    let f = write_toml(r#"
[app]
name    = "watchdog-app"
version = "0.1.0"
wasm    = "watchdog-app.wasm"

[capabilities]
watchdog_secs = 0
"#);
    let result = supervisor::manifest::parse_manifest(f.path());
    assert!(result.is_ok(), "watchdog_secs=0 should be valid: {:?}", result);
    let m = result.unwrap();
    assert_eq!(m.capabilities.watchdog_secs, 0);
}
