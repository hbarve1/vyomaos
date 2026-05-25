// T041: Failing unit tests for profile-dependent capability behavior.
//
// Tests verify that selecting desktop-full vs server-headless profile
// results in different capability sets being active for the same app manifest.
//
// The key invariant (User Story 3):
//   - desktop-full  → includes "display" in supervisor.modules
//   - server-headless → does NOT include "display" in supervisor.modules
//   - Both profiles include "net" / "lifecycle" / "capability"
//
// A doc-viewer app with both `display = true` and `network = true` in its
// vyoma.toml will render via VYOMA_DRAW on desktop (display wired) and serve
// via HTTP on server (only network wired).

use supervisor::profile::load_profile;
use supervisor::manifest::parse_manifest;
use std::io::Write;
use tempfile::NamedTempFile;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn write_file(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(content.as_bytes()).expect("write");
    f
}

/// True if the profile's supervisor modules include the named module.
fn profile_has_module(profile: &supervisor::profile::PlatformProfile, module: &str) -> bool {
    profile.supervisor.modules.iter().any(|m| m == module)
}

// ── T041-1: desktop-full profile has "display" module ────────────────────────

#[test]
fn test_desktop_full_has_display_module() {
    let path = std::path::Path::new("src/profile/profiles/desktop-full.toml");
    let profile = load_profile(path).expect("desktop-full profile must parse");
    assert!(
        profile_has_module(&profile, "display"),
        "desktop-full should include 'display' in supervisor.modules"
    );
}

// ── T041-2: server-headless profile does NOT have "display" module ────────────

#[test]
fn test_server_headless_has_no_display_module() {
    let path = std::path::Path::new("src/profile/profiles/server-headless.toml");
    let profile = load_profile(path).expect("server-headless profile must parse");
    assert!(
        !profile_has_module(&profile, "display"),
        "server-headless should NOT include 'display' in supervisor.modules"
    );
}

// ── T041-3: server-headless profile has "net" module ─────────────────────────

#[test]
fn test_server_headless_has_net_module() {
    let path = std::path::Path::new("src/profile/profiles/server-headless.toml");
    let profile = load_profile(path).expect("server-headless profile must parse");
    assert!(
        profile_has_module(&profile, "net"),
        "server-headless should include 'net' in supervisor.modules"
    );
}

// ── T041-4: desktop-full profile also has "net" module ───────────────────────

#[test]
fn test_desktop_full_has_net_module() {
    let path = std::path::Path::new("src/profile/profiles/desktop-full.toml");
    let profile = load_profile(path).expect("desktop-full profile must parse");
    assert!(
        profile_has_module(&profile, "net"),
        "desktop-full should also include 'net' in supervisor.modules"
    );
}

// ── T041-5: doc-viewer manifest with display+network parses without error ─────

#[test]
fn test_doc_viewer_manifest_parses_ok() {
    let f = write_file(r#"
[app]
name    = "doc-viewer"
version = "0.1.0"
wasm    = "doc-viewer.wasm"

[capabilities]
stdio   = true
display = true
network = true
network_port = 8081
"#);
    let result = parse_manifest(f.path());
    assert!(result.is_ok(), "doc-viewer manifest should parse: {:?}", result);
    let manifest = result.unwrap();
    assert!(manifest.capabilities.display, "display should be true");
    assert!(manifest.capabilities.network, "network should be true");
}

// ── T041-6: Profile-dependent behavior — on desktop, display is wired ─────────
//
// Simulates: supervisor reads desktop-full profile → sees "display" module →
// doc-viewer (which has display=true) gets VYOMA_DRAW capabilities wired up.

#[test]
fn test_desktop_profile_wires_display_for_doc_viewer() {
    let profile_path = std::path::Path::new("src/profile/profiles/desktop-full.toml");
    let profile = load_profile(profile_path).expect("desktop-full must parse");

    let manifest_file = write_file(r#"
[app]
name    = "doc-viewer"
version = "0.1.0"
wasm    = "doc-viewer.wasm"

[capabilities]
stdio   = true
display = true
network = true
network_port = 8081
"#);
    let manifest = parse_manifest(manifest_file.path()).expect("manifest must parse");

    // The "wiring" rule: display is active if:
    //   1. app manifest has display = true, AND
    //   2. the platform profile includes the "display" supervisor module.
    let display_wired = manifest.capabilities.display && profile_has_module(&profile, "display");
    assert!(
        display_wired,
        "display should be wired on desktop-full for doc-viewer"
    );
}

// ── T041-7: Profile-dependent behavior — on server, display is NOT wired ──────
//
// Same app manifest, server-headless profile → display module absent →
// display not wired even though manifest declares display=true.

#[test]
fn test_server_profile_does_not_wire_display_for_doc_viewer() {
    let profile_path = std::path::Path::new("src/profile/profiles/server-headless.toml");
    let profile = load_profile(profile_path).expect("server-headless must parse");

    let manifest_file = write_file(r#"
[app]
name    = "doc-viewer"
version = "0.1.0"
wasm    = "doc-viewer.wasm"

[capabilities]
stdio   = true
display = true
network = true
network_port = 8081
"#);
    let manifest = parse_manifest(manifest_file.path()).expect("manifest must parse");

    // display is NOT wired: profile does not include "display" module
    let display_wired = manifest.capabilities.display && profile_has_module(&profile, "display");
    assert!(
        !display_wired,
        "display should NOT be wired on server-headless for doc-viewer"
    );
}

// ── T041-8: Profile-dependent behavior — on server, network IS wired ──────────

#[test]
fn test_server_profile_wires_network_for_doc_viewer() {
    let profile_path = std::path::Path::new("src/profile/profiles/server-headless.toml");
    let profile = load_profile(profile_path).expect("server-headless must parse");

    let manifest_file = write_file(r#"
[app]
name    = "doc-viewer"
version = "0.1.0"
wasm    = "doc-viewer.wasm"

[capabilities]
stdio   = true
display = true
network = true
network_port = 8081
"#);
    let manifest = parse_manifest(manifest_file.path()).expect("manifest must parse");

    // network is wired: profile includes "net" module AND manifest has network=true
    let network_wired = manifest.capabilities.network && profile_has_module(&profile, "net");
    assert!(
        network_wired,
        "network should be wired on server-headless for doc-viewer"
    );
}

// ── T041-9: Capability enforcement — network=false app cannot use network ──────
//
// Simulates T046: an app with network=false on server profile cannot open sockets.
// The supervisor wires network only when BOTH manifest.network=true AND profile
// has "net" module.  If manifest.network=false, network is never wired regardless
// of profile.

#[test]
fn test_network_false_manifest_blocks_network_regardless_of_profile() {
    let profile_path = std::path::Path::new("src/profile/profiles/server-headless.toml");
    let profile = load_profile(profile_path).expect("server-headless must parse");

    let manifest_file = write_file(r#"
[app]
name    = "secure-doc"
version = "0.1.0"
wasm    = "secure-doc.wasm"

[capabilities]
stdio   = true
display = false
network = false
"#);
    let manifest = parse_manifest(manifest_file.path()).expect("manifest must parse");

    // Even though profile has "net", manifest.network=false means no wiring.
    let network_wired = manifest.capabilities.network && profile_has_module(&profile, "net");
    assert!(
        !network_wired,
        "network=false in manifest must block network even on server profile"
    );
}

// ── T041-10: Both profiles use wasmtime runtime ───────────────────────────────

#[test]
fn test_both_profiles_use_wasmtime() {
    use supervisor::profile::Runtime;

    let desktop_path = std::path::Path::new("src/profile/profiles/desktop-full.toml");
    let server_path = std::path::Path::new("src/profile/profiles/server-headless.toml");

    let desktop = load_profile(desktop_path).expect("desktop-full must parse");
    let server = load_profile(server_path).expect("server-headless must parse");

    assert_eq!(
        desktop.platform.runtime,
        Runtime::Wasmtime,
        "desktop-full must use wasmtime"
    );
    assert_eq!(
        server.platform.runtime,
        Runtime::Wasmtime,
        "server-headless must use wasmtime"
    );
}
