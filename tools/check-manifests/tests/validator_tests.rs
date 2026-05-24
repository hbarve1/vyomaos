// Validator unit tests — TDD RED until check-manifests lib functions are implemented.

use std::io::Write;
use tempfile::NamedTempFile;

fn write_toml(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(content.as_bytes()).expect("write");
    f
}

// (1) Valid TOML parses and reports OK (no error)
#[test]
fn test_valid_manifest_reports_ok() {
    let f = write_toml(r#"
[app]
name    = "valid-app"
version = "0.1.0"
wasm    = "valid-app.wasm"

[capabilities]
stdio = true
"#);
    let result = supervisor::manifest::parse_manifest(f.path());
    assert!(result.is_ok(), "valid manifest should parse OK: {:?}", result);
}

// (2) Unknown field returns ManifestValidationError with UnknownField error
#[test]
fn test_unknown_field_returns_err() {
    let f = write_toml(r#"
[app]
name    = "bad-app"
version = "0.1.0"
wasm    = "bad-app.wasm"

[capabilities]
stdio      = true
network_v2 = true
"#);
    let result = supervisor::manifest::parse_manifest(f.path());
    assert!(result.is_err(), "unknown field should return Err");
    let msg = result.unwrap_err();
    assert!(msg.contains("network_v2") || msg.contains("unknown"),
        "error should mention unknown field: {msg}");
}

// (3) Duplicate name across two manifests returns error for the second
#[test]
fn test_duplicate_name_returns_err_for_second() {
    let f1 = write_toml(r#"
[app]
name    = "shared-name"
version = "0.1.0"
wasm    = "app.wasm"
"#);
    let f2 = write_toml(r#"
[app]
name    = "shared-name"
version = "0.1.0"
wasm    = "app2.wasm"
"#);
    let m1 = supervisor::manifest::parse_manifest(f1.path()).expect("first parse ok");
    let m2 = supervisor::manifest::parse_manifest(f2.path()).expect("second parse ok");

    let registered: &[&str] = &[];
    assert!(supervisor::manifest::validate_manifest(&m1, registered).is_ok());

    let registered_after_first = &[m1.app.name.as_str()];
    let result = supervisor::manifest::validate_manifest(&m2, registered_after_first);
    assert!(result.is_err(), "second registration of same name should be Err");
    assert!(result.unwrap_err().contains("shared-name"));
}
