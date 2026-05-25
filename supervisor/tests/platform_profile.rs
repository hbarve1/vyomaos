// T011: Failing unit tests for platform profile loading.
//
// Tests cover TOML parsing, validation rules, and module selection.
// All validation rules are from contracts/platform-profile-schema.md.

use std::io::Write;
use tempfile::NamedTempFile;
use supervisor::profile::{load_profile, ProfileError, Runtime, ObservabilityTier};

fn write_toml(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(content.as_bytes()).expect("write");
    f
}

/// Minimal valid desktop profile.
fn desktop_toml() -> &'static str {
    r#"
[platform]
name = "desktop-full"
arch = "x86-64"
runtime = "wasmtime"
min_ram_kb = 524288

[supervisor]
modules = ["lifecycle", "capability", "net"]

[observability]
tier = "baseline"
heartbeat_interval_s = 30

[ota]
enabled = true
health_check_secs = 60
"#
}

/// Minimal valid MCU profile.
fn mcu_toml() -> &'static str {
    r#"
[platform]
name = "mcu-minimal"
arch = "arm-cortex-m4"
runtime = "wasm3"
min_ram_kb = 128

[supervisor]
modules = ["lifecycle", "capability"]

[observability]
tier = "baseline"
heartbeat_interval_s = 60
"#
}

// ── T011-1: Valid desktop profile parses successfully ─────────────────────────

#[test]
fn test_valid_desktop_profile_parses_ok() {
    let f = write_toml(desktop_toml());
    let result = load_profile(f.path());
    assert!(result.is_ok(), "valid desktop profile should parse: {:?}", result);
    let profile = result.unwrap();
    assert_eq!(profile.platform.name, "desktop-full");
    assert_eq!(profile.platform.runtime, Runtime::Wasmtime);
    assert_eq!(profile.platform.min_ram_kb, 524288);
}

// ── T011-2: Valid MCU profile parses successfully ─────────────────────────────

#[test]
fn test_valid_mcu_profile_parses_ok() {
    let f = write_toml(mcu_toml());
    let result = load_profile(f.path());
    assert!(result.is_ok(), "valid MCU profile should parse: {:?}", result);
    let profile = result.unwrap();
    assert_eq!(profile.platform.runtime, Runtime::Wasm3);
    assert_eq!(profile.platform.min_ram_kb, 128);
}

// ── T011-3: Profile missing lifecycle module fails validation ─────────────────

#[test]
fn test_missing_lifecycle_module_returns_err() {
    let f = write_toml(r#"
[platform]
name = "bad"
arch = "x86-64"
runtime = "wasmtime"
min_ram_kb = 524288

[supervisor]
modules = ["capability", "net"]
"#);
    let result = load_profile(f.path());
    assert!(result.is_err(), "missing lifecycle should fail validation");
    match result.unwrap_err() {
        ProfileError::Validation(msg) => assert!(msg.contains("lifecycle"), "error should mention lifecycle: {msg}"),
        e => panic!("expected Validation error, got: {:?}", e),
    }
}

// ── T011-4: Profile missing capability module fails validation ────────────────

#[test]
fn test_missing_capability_module_returns_err() {
    let f = write_toml(r#"
[platform]
name = "bad"
arch = "x86-64"
runtime = "wasmtime"
min_ram_kb = 524288

[supervisor]
modules = ["lifecycle", "net"]
"#);
    let result = load_profile(f.path());
    assert!(result.is_err(), "missing capability should fail validation");
    match result.unwrap_err() {
        ProfileError::Validation(msg) => assert!(msg.contains("capability"), "error should mention capability: {msg}"),
        e => panic!("expected Validation error, got: {:?}", e),
    }
}

// ── T011-5: Wasmtime with min_ram_kb < 4096 fails (Rule 6) ───────────────────

#[test]
fn test_wasmtime_too_little_ram_returns_err() {
    let f = write_toml(r#"
[platform]
name = "tiny-wasmtime"
arch = "x86-64"
runtime = "wasmtime"
min_ram_kb = 512

[supervisor]
modules = ["lifecycle", "capability"]
"#);
    let result = load_profile(f.path());
    assert!(result.is_err(), "wasmtime with <4096 KB should fail");
    match result.unwrap_err() {
        ProfileError::Validation(msg) => assert!(msg.contains("4096"), "error should mention 4096: {msg}"),
        e => panic!("expected Validation error, got: {:?}", e),
    }
}

// ── T011-6: full observability requires wasmtime (Rule 7) ────────────────────

#[test]
fn test_full_observability_requires_wasmtime() {
    let f = write_toml(r#"
[platform]
name = "wamr-full"
arch = "arm64"
runtime = "wamr"
min_ram_kb = 4096

[supervisor]
modules = ["lifecycle", "capability"]

[observability]
tier = "full"
"#);
    let result = load_profile(f.path());
    assert!(result.is_err(), "full observability with wamr should fail");
    match result.unwrap_err() {
        ProfileError::Validation(msg) => assert!(
            msg.contains("wasmtime"),
            "error should mention wasmtime: {msg}"
        ),
        e => panic!("expected Validation error, got: {:?}", e),
    }
}

// ── T011-7: full observability with wasmtime is valid ────────────────────────

#[test]
fn test_full_observability_with_wasmtime_ok() {
    let f = write_toml(r#"
[platform]
name = "desktop-full"
arch = "x86-64"
runtime = "wasmtime"
min_ram_kb = 524288

[supervisor]
modules = ["lifecycle", "capability"]

[observability]
tier = "full"
"#);
    let result = load_profile(f.path());
    assert!(result.is_ok(), "full observability with wasmtime should be ok: {:?}", result);
    assert_eq!(result.unwrap().observability.tier, ObservabilityTier::Full);
}

// ── T011-8: Unknown runtime string fails TOML parse ──────────────────────────

#[test]
fn test_unknown_runtime_fails_parse() {
    let f = write_toml(r#"
[platform]
name = "bad"
arch = "x86-64"
runtime = "v8"
min_ram_kb = 524288

[supervisor]
modules = ["lifecycle", "capability"]
"#);
    let result = load_profile(f.path());
    assert!(result.is_err(), "unknown runtime should fail");
    match result.unwrap_err() {
        ProfileError::Parse(_) => {} // expected
        e => panic!("expected Parse error, got: {:?}", e),
    }
}

// ── T011-9: load_profile on non-existent file returns Io error ───────────────

#[test]
fn test_load_nonexistent_file_returns_io_error() {
    let result = load_profile(std::path::Path::new("/no/such/profile.toml"));
    assert!(result.is_err());
    match result.unwrap_err() {
        ProfileError::Io(_) => {} // expected
        e => panic!("expected Io error, got: {:?}", e),
    }
}

// ── T011-10: embedded mcu-minimal profile TOML file parses correctly ─────────

#[test]
fn test_embedded_mcu_minimal_profile_parses() {
    // Load directly from the embedded profiles/ directory.
    let path = std::path::Path::new(
        "src/profile/profiles/mcu-minimal.toml"
    );
    let result = load_profile(path);
    assert!(result.is_ok(), "embedded mcu-minimal.toml should parse: {:?}", result);
    let p = result.unwrap();
    assert_eq!(p.platform.name, "mcu-minimal");
    assert_eq!(p.platform.runtime, Runtime::Wasm3);
}

// ── T011-11: embedded desktop-full profile TOML file parses correctly ────────

#[test]
fn test_embedded_desktop_full_profile_parses() {
    let path = std::path::Path::new(
        "src/profile/profiles/desktop-full.toml"
    );
    let result = load_profile(path);
    assert!(result.is_ok(), "embedded desktop-full.toml should parse: {:?}", result);
    let p = result.unwrap();
    assert_eq!(p.platform.name, "desktop-full");
    assert_eq!(p.platform.runtime, Runtime::Wasmtime);
    assert_eq!(p.observability.tier, ObservabilityTier::Full);
}
