// Z-order / z-layer tests (spec-049).
//
// Covers:
//   1. WindowRegion.z deserializes correctly — default=10 when absent.
//   2. Explicit z values (z=0 for desktop, z=100 for dock) round-trip via TOML.
//   3. Layer constants exported from chrome have the expected numeric values.

use std::io::Write;
use tempfile::NamedTempFile;

fn write_toml(content: &str) -> NamedTempFile {
    let mut f = NamedTempFile::new().expect("tempfile");
    f.write_all(content.as_bytes()).expect("write");
    f
}

// (1) [window] section without z= defaults to 10 (Z_APP layer).
#[test]
fn test_window_z_defaults_to_10() {
    let f = write_toml(r#"
[app]
name    = "my-app"
version = "0.1.0"
wasm    = "my-app.wasm"

[capabilities]
display = true

[window]
x = 0
y = 28
w = 1440
h = 832
"#);
    let m = supervisor::manifest::parse_manifest(f.path())
        .expect("parse should succeed");
    let win = m.window.expect("should have [window]");
    assert_eq!(win.z, 10, "z should default to 10 (Z_APP) when absent");
}

// (2) Manifest with no [window] section — win.z is unavailable (window is None).
#[test]
fn test_no_window_section_gives_none() {
    let f = write_toml(r#"
[app]
name    = "headless-app"
version = "0.1.0"
wasm    = "headless-app.wasm"

[capabilities]
stdio = true
"#);
    let m = supervisor::manifest::parse_manifest(f.path())
        .expect("parse should succeed");
    assert!(m.window.is_none(), "no [window] section → window should be None");
}

// (3) z=0 for desktop (permanent background layer).
#[test]
fn test_window_z_desktop_zero() {
    let f = write_toml(r#"
[app]
name    = "desktop"
version = "0.2.0"
wasm    = "desktop.wasm"

[capabilities]
display = true

[window]
x = 0
y = 28
w = 1440
h = 832
z = 0
"#);
    let m = supervisor::manifest::parse_manifest(f.path())
        .expect("parse should succeed");
    let win = m.window.expect("should have [window]");
    assert_eq!(win.z, 0, "desktop z should be 0");
}

// (4) z=100 for dock (above app windows).
#[test]
fn test_window_z_dock_100() {
    let f = write_toml(r#"
[app]
name    = "dock"
version = "0.2.0"
wasm    = "dock.wasm"

[capabilities]
display = true

[window]
x = 0
y = 868
w = 1440
h = 60
z = 100
"#);
    let m = supervisor::manifest::parse_manifest(f.path())
        .expect("parse should succeed");
    let win = m.window.expect("should have [window]");
    assert_eq!(win.z, 100, "dock z should be 100 (Z_DOCK)");
}

// (5) Arbitrary explicit z value round-trips correctly.
#[test]
fn test_window_z_explicit_value() {
    let f = write_toml(r#"
[app]
name    = "overlay-app"
version = "0.1.0"
wasm    = "overlay-app.wasm"

[capabilities]
display = true

[window]
x = 0
y = 0
w = 400
h = 200
z = 255
"#);
    let m = supervisor::manifest::parse_manifest(f.path())
        .expect("parse should succeed");
    let win = m.window.expect("should have [window]");
    assert_eq!(win.z, 255, "explicit z=255 should round-trip");
}

// (6) Z-layer constants from chrome module have the expected values.
#[test]
fn test_z_layer_constants() {
    assert_eq!(supervisor::chrome::Z_DESKTOP, 0,   "Z_DESKTOP must be 0");
    assert_eq!(supervisor::chrome::Z_APP,     10,  "Z_APP must be 10");
    assert_eq!(supervisor::chrome::Z_DOCK,    100, "Z_DOCK must be 100");
    assert_eq!(supervisor::chrome::Z_OVERLAY, 255, "Z_OVERLAY must be 255");
}

// (7) Z_DESKTOP < Z_APP < Z_DOCK < Z_OVERLAY (ordering invariant).
#[test]
fn test_z_layer_ordering() {
    use supervisor::chrome::{Z_DESKTOP, Z_APP, Z_DOCK, Z_OVERLAY};
    assert!(Z_DESKTOP < Z_APP,   "desktop must be behind apps");
    assert!(Z_APP     < Z_DOCK,  "apps must be behind dock");
    assert!(Z_DOCK    < Z_OVERLAY, "dock must be behind overlays");
}
