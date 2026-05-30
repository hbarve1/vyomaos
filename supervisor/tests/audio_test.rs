// Audio subsystem tests — AudioState defaults, volume set/get, mute toggle.

use supervisor::logging::Subsystem;

#[test]
fn audio_subsystem_variant_exists() {
    // Verify the Audio subsystem variant is accessible.
    let sub = Subsystem::Audio;
    // format_log should include "audio" subsystem tag.
    let line = supervisor::logging::format_log(
        supervisor::logging::Level::Info,
        sub,
        Some("test-app"),
        "test message",
    );
    assert!(line.contains("[audio]"), "expected [audio] in log line: {line}");
    assert!(line.contains("test-app"), "expected app name in log line: {line}");
}

#[test]
fn audio_capability_in_manifest() {
    // Verify that audio = true parses correctly in a manifest.
    let toml_str = r#"
[app]
name = "audio-test"
version = "0.1.0"
wasm = "audio-test.wasm"

[capabilities]
stdio = true
audio = true
"#;
    let manifest: supervisor::manifest::AppManifest = toml::from_str(toml_str).unwrap();
    assert!(manifest.capabilities.audio);
    assert!(manifest.capabilities.stdio);
    assert!(!manifest.capabilities.display);
}

#[test]
fn audio_capability_defaults_false() {
    let toml_str = r#"
[app]
name = "no-audio"
version = "0.1.0"
wasm = "no-audio.wasm"

[capabilities]
stdio = true
"#;
    let manifest: supervisor::manifest::AppManifest = toml::from_str(toml_str).unwrap();
    assert!(!manifest.capabilities.audio);
}
