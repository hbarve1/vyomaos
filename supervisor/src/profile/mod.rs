// Platform profile system — T003 + T012
//
// `PlatformProfile` is deserialized from a TOML file matching
// contracts/platform-profile-schema.md.  `load_profile` parses a file from
// disk (or the embedded profiles/ directory) and validates it against the
// schema rules before returning.

use serde::Deserialize;

pub mod loader;

// ── Enums ─────────────────────────────────────────────────────────────────────

/// Supported WASM runtimes.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Runtime {
    Wasm3,
    Wamr,
    Wasmtime,
}

/// Observability tier — determines which telemetry subsystems are active.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ObservabilityTier {
    /// Structured logs + periodic heartbeats only.
    Baseline,
    /// Full OpenTelemetry-compatible traces, metrics, and logs.
    Full,
}

impl Default for ObservabilityTier {
    fn default() -> Self {
        Self::Baseline
    }
}

// ── Sub-tables ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct SupervisorConfig {
    /// Supervisor subsystems to enable (must include at minimum "lifecycle" and "capability").
    pub modules: Vec<String>,
    /// Supervisor subsystems to exclude (optional).
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct HalConfig {
    /// HAL driver modules to include (e.g. ["gpio", "i2c"]).
    #[serde(default)]
    pub drivers: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BootConfig {
    /// WASM app names to launch at boot.
    #[serde(default)]
    pub apps: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ObservabilityConfig {
    #[serde(default)]
    pub tier: ObservabilityTier,
    /// Seconds between heartbeat emissions (default 30).
    #[serde(default = "default_heartbeat_interval")]
    pub heartbeat_interval_s: u32,
}

fn default_heartbeat_interval() -> u32 {
    30
}

impl Default for ObservabilityConfig {
    fn default() -> Self {
        Self {
            tier: ObservabilityTier::Baseline,
            heartbeat_interval_s: default_heartbeat_interval(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct OtaConfig {
    /// Whether OTA updates are supported on this platform.
    #[serde(default)]
    pub enabled: bool,
    /// Seconds the supervisor waits for a health check after an update.
    #[serde(default = "default_health_check_secs")]
    pub health_check_secs: u32,
    /// Enable A/B slot management (defaults to `true` when `enabled = true`).
    #[serde(default = "default_true")]
    pub ab_slots: bool,
}

fn default_health_check_secs() -> u32 {
    60
}

fn default_true() -> bool {
    true
}

impl Default for OtaConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            health_check_secs: default_health_check_secs(),
            ab_slots: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BuildConfig {
    /// Path to the kernel .config (relative to platform dir).
    #[serde(default)]
    pub kernel_config: Option<String>,
    /// Path to the rootfs build script (relative to platform dir).
    #[serde(default)]
    pub rootfs_script: Option<String>,
}

// ── DisplayConfig ─────────────────────────────────────────────────────────────

/// Display / form-factor layout configuration.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DisplayConfig {
    /// Named display profile: "desktop", "phone", "tablet", "watch", "tv", "vision".
    #[serde(default = "default_display_profile")]
    pub profile: String,
    /// Show dock strip at bottom.
    #[serde(default = "default_true")]
    pub show_dock: bool,
    /// Show menu bar at top.
    #[serde(default = "default_true")]
    pub show_menu_bar: bool,
    /// Apps run in windows (true) or full-screen single-app (false).
    #[serde(default = "default_true")]
    pub windowed_mode: bool,
    /// Show keyboard focus ring around focused element (TV/Vision).
    #[serde(default)]
    pub focus_ring: bool,
    /// Swipe-up gesture returns to home screen (Phone/Tablet).
    #[serde(default)]
    pub home_gesture: bool,
    /// Digital crown input maps to scroll (Watch).
    #[serde(default)]
    pub crown_scroll: bool,
    /// Spatial floating panel layout (Vision Pro).
    #[serde(default)]
    pub spatial_layout: bool,
}

fn default_display_profile() -> String { "desktop".to_string() }

// ── PlatformProfile ───────────────────────────────────────────────────────────

/// Top-level platform profile, loaded from a TOML file.
#[derive(Debug, Clone, Deserialize)]
pub struct PlatformProfile {
    pub platform: PlatformMeta,
    pub supervisor: SupervisorConfig,
    #[serde(default)]
    pub hal: HalConfig,
    #[serde(default)]
    pub boot: BootConfig,
    #[serde(default)]
    pub observability: ObservabilityConfig,
    #[serde(default)]
    pub ota: OtaConfig,
    #[serde(default)]
    pub build: BuildConfig,
    #[serde(default)]
    pub display: DisplayConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlatformMeta {
    pub name: String,
    pub arch: String,
    pub runtime: Runtime,
    /// Minimum RAM requirement in KB.
    pub min_ram_kb: u32,
}

// ── Public API — re-exported from loader ─────────────────────────────────────

pub use loader::{load_profile, validate_profile, ProfileError};
