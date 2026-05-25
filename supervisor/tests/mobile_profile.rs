// TM02: Failing tests for mobile platform profile loading.
//
// Verifies mobile.toml:
//   - display = true, touch = true, network = true, mouse = false
//   - screen_w = 1080, screen_h = 2340, orientation = "portrait"
//   - runtime = "wasmtime", arch = "arm64"

use supervisor::profile::load_profile;
use supervisor::profile::mobile::{MobileCapabilities, MobileDisplayConfig};

#[test]
fn mobile_profile_loads_successfully() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let result = load_profile(path);
    assert!(result.is_ok(), "mobile profile should load: {:?}", result);
}

#[test]
fn mobile_profile_runtime_is_wasmtime() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    assert_eq!(
        profile.platform.runtime,
        supervisor::profile::Runtime::Wasmtime,
        "mobile platform must use wasmtime"
    );
}

#[test]
fn mobile_profile_arch_is_arm64() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    assert_eq!(profile.platform.arch, "arm64");
}

#[test]
fn mobile_profile_has_touch_capability() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    let caps = profile.mobile_caps.expect("mobile_caps should be present");
    assert!(caps.touch, "touch must be true in mobile profile");
}

#[test]
fn mobile_profile_mouse_is_false() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    let caps = profile.mobile_caps.expect("mobile_caps should be present");
    assert!(!caps.mouse, "mouse must be false in mobile profile");
}

#[test]
fn mobile_profile_display_is_true() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    let caps = profile.mobile_caps.expect("mobile_caps should be present");
    assert!(caps.display, "display must be true in mobile profile");
}

#[test]
fn mobile_profile_network_is_true() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    let caps = profile.mobile_caps.expect("mobile_caps should be present");
    assert!(caps.network, "network must be true in mobile profile");
}

#[test]
fn mobile_profile_screen_width_is_1080() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    let display = profile.mobile_display.expect("mobile_display should be present");
    assert_eq!(display.screen_w, 1080, "screen width must be 1080");
}

#[test]
fn mobile_profile_screen_height_is_2340() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    let display = profile.mobile_display.expect("mobile_display should be present");
    assert_eq!(display.screen_h, 2340, "screen height must be 2340");
}

#[test]
fn mobile_profile_orientation_is_portrait() {
    let path = std::path::Path::new("src/profile/profiles/mobile.toml");
    let profile = load_profile(path).expect("load mobile profile");
    let display = profile.mobile_display.expect("mobile_display should be present");
    assert_eq!(display.orientation, "portrait");
}

// ── struct type checks ────────────────────────────────────────────────────────

#[test]
fn mobile_capabilities_struct_fields() {
    let caps = MobileCapabilities {
        touch: true,
        mouse: false,
        display: true,
        network: true,
    };
    assert!(caps.touch);
    assert!(!caps.mouse);
    assert!(caps.display);
    assert!(caps.network);
}

#[test]
fn mobile_display_config_struct_fields() {
    let d = MobileDisplayConfig {
        screen_w: 1080,
        screen_h: 2340,
        orientation: "portrait".to_string(),
    };
    assert_eq!(d.screen_w, 1080);
    assert_eq!(d.screen_h, 2340);
    assert_eq!(d.orientation, "portrait");
}
