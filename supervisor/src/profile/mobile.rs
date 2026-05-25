// Mobile-specific profile extensions (TM05).
//
// These structs are deserialized from the `[mobile_caps]` and `[mobile_display]`
// tables in a platform profile TOML file.  They are optional; non-mobile profiles
// simply omit those tables.

use serde::Deserialize;

// ── MobileCapabilities ────────────────────────────────────────────────────────

/// Capability flags specific to mobile/tablet platforms.
///
/// Stored in `PlatformProfile::mobile_caps`.  If absent the profile is not a
/// mobile target and touch routing is disabled.
#[derive(Debug, Clone, Deserialize)]
pub struct MobileCapabilities {
    /// Platform has a touchscreen; `VYOMA_INPUT:touch:` events are generated.
    #[serde(default)]
    pub touch: bool,
    /// Platform has a pointing device (mouse / trackpad).
    /// Typically `false` on phones / tablets.
    #[serde(default)]
    pub mouse: bool,
    /// Platform has a display framebuffer.
    #[serde(default)]
    pub display: bool,
    /// Platform has network connectivity.
    #[serde(default)]
    pub network: bool,
}

// ── MobileDisplayConfig ───────────────────────────────────────────────────────

/// Display geometry for a mobile platform.
///
/// Stored in `PlatformProfile::mobile_display`.
#[derive(Debug, Clone, Deserialize)]
pub struct MobileDisplayConfig {
    /// Display width in pixels (portrait: narrow axis).
    #[serde(default)]
    pub screen_w: u32,
    /// Display height in pixels (portrait: tall axis).
    #[serde(default)]
    pub screen_h: u32,
    /// Default orientation: `"portrait"` or `"landscape"`.
    #[serde(default = "default_portrait")]
    pub orientation: String,
}

fn default_portrait() -> String {
    "portrait".to_string()
}
