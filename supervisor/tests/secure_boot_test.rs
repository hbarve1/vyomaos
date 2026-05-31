// P91: Secure boot chain verification tests.

use sha2::{Digest, Sha256};
use std::fs;

// ── Boot manifest TOML parsing ───────────────────────────────────────────────

#[derive(Debug, serde::Deserialize)]
struct BootManifestToml {
    boot: BootManifestEntries,
}

#[derive(Debug, serde::Deserialize)]
struct BootManifestEntries {
    #[serde(default)]
    supervisor_sha256: String,
    #[serde(default)]
    wasmtime_sha256: String,
}

fn parse_boot_manifest(raw: &str) -> Option<BootManifestToml> {
    toml::from_str(raw).ok()
}

#[test]
fn parse_valid_manifest() {
    let toml = r#"
[boot]
supervisor_sha256 = "aabbccdd"
wasmtime_sha256   = "11223344"
"#;
    let m = parse_boot_manifest(toml).unwrap();
    assert_eq!(m.boot.supervisor_sha256, "aabbccdd");
    assert_eq!(m.boot.wasmtime_sha256, "11223344");
}

#[test]
fn parse_manifest_missing_fields_defaults_empty() {
    let m = parse_boot_manifest("[boot]\n").unwrap();
    assert!(m.boot.supervisor_sha256.is_empty());
    assert!(m.boot.wasmtime_sha256.is_empty());
}

#[test]
fn parse_invalid_toml_returns_none() {
    assert!(parse_boot_manifest("{{invalid}}").is_none());
}

// ── SHA-256 helpers ──────────────────────────────────────────────────────────

fn sha256_of(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

fn sha256_matches(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

#[test]
fn sha256_matches_identical() {
    let h = sha256_of(b"hello");
    assert!(sha256_matches(&h, &h));
}

#[test]
fn sha256_matches_case_insensitive() {
    let h = sha256_of(b"hello");
    assert!(sha256_matches(&h, &h.to_uppercase()));
}

#[test]
fn sha256_does_not_match_different() {
    assert!(!sha256_matches(&sha256_of(b"a"), &sha256_of(b"b")));
}

// ── File hashing integration ─────────────────────────────────────────────────

#[test]
fn hash_file_matches_expected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("test.bin");
    let data = b"boot chain test data";
    fs::write(&path, data).unwrap();

    let file_bytes = fs::read(&path).unwrap();
    let actual = format!("{:x}", Sha256::digest(&file_bytes));
    let expected = sha256_of(data);
    assert_eq!(actual, expected);
}

#[test]
fn hash_file_not_found() {
    let result = fs::read("/nonexistent/secure_boot_test.bin");
    assert!(result.is_err());
}

// ── Manifest generation format ───────────────────────────────────────────────

#[test]
fn generated_manifest_is_valid_toml() {
    let manifest = format!(
        "[boot]\nsupervisor_sha256 = \"{}\"\nwasmtime_sha256 = \"{}\"\n",
        sha256_of(b"supervisor-binary"),
        sha256_of(b"wasmtime-binary"),
    );
    let parsed: Result<BootManifestToml, _> = toml::from_str(&manifest);
    assert!(parsed.is_ok(), "generated manifest should parse as valid TOML");
    let m = parsed.unwrap();
    assert!(!m.boot.supervisor_sha256.is_empty());
    assert!(!m.boot.wasmtime_sha256.is_empty());
}

// ── Verify result logic ─────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum VerifyResult {
    Pass,
    Fail,
    Missing,
    Skipped,
}

fn verify_component(actual: &str, expected: &str) -> VerifyResult {
    if expected.is_empty() {
        return VerifyResult::Skipped;
    }
    if actual.is_empty() {
        return VerifyResult::Missing;
    }
    if sha256_matches(actual, expected) {
        VerifyResult::Pass
    } else {
        VerifyResult::Fail
    }
}

#[test]
fn verify_component_pass() {
    assert_eq!(verify_component("abc", "ABC"), VerifyResult::Pass);
}

#[test]
fn verify_component_fail() {
    assert_eq!(verify_component("abc", "def"), VerifyResult::Fail);
}

#[test]
fn verify_component_missing_actual() {
    assert_eq!(verify_component("", "abc"), VerifyResult::Missing);
}

#[test]
fn verify_component_skipped_no_expected() {
    assert_eq!(verify_component("abc", ""), VerifyResult::Skipped);
}

#[test]
fn boot_verified_when_all_pass() {
    let s = verify_component("aaa", "AAA");
    let w = verify_component("bbb", "BBB");
    let verified = matches!((&s, &w), (VerifyResult::Pass, VerifyResult::Pass));
    assert!(verified);
}

#[test]
fn boot_verified_when_all_skipped() {
    let s = verify_component("x", "");
    let w = verify_component("y", "");
    let verified = matches!((&s, &w), (VerifyResult::Skipped, VerifyResult::Skipped));
    assert!(verified);
}

#[test]
fn boot_not_verified_on_any_fail() {
    let s = verify_component("abc", "ABC");
    let w = verify_component("wrong", "expected");
    let verified = matches!((&s, &w), (VerifyResult::Pass, VerifyResult::Pass)
        | (VerifyResult::Skipped, VerifyResult::Skipped));
    assert!(!verified);
}
