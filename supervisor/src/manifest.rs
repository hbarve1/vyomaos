// Manifest module — app manifest structs and pure parsing/validation functions.

use serde::Deserialize;

// ── Boot config structs ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct BootConfig {
    pub apps: Vec<BootEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BootEntry {
    pub manifest: String,
    #[serde(default = "default_restart")]
    pub restart: String,
}

pub fn default_restart() -> String {
    "never".to_string()
}

// ── App manifest structs ──────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize, Clone, Copy)]
#[serde(deny_unknown_fields)]
pub struct WindowRegion {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

#[derive(Debug, Deserialize)]
pub struct AppManifest {
    pub app: AppMeta,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub window: Option<WindowRegion>,
}

#[derive(Debug, Deserialize)]
pub struct AppMeta {
    pub name: String,
    pub version: String,
    pub wasm: String,
    /// Optional SHA-256 hex digest of the .wasm binary.
    #[serde(default)]
    pub wasm_sha256: Option<String>,
}

// deny_unknown_fields ensures manifests cannot declare undocumented capabilities.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub stdio: bool,
    #[serde(default)]
    pub filesystem: bool,
    #[serde(default)]
    pub network: bool,
    #[serde(default)]
    pub network_port: Option<u16>,
    #[serde(default)]
    pub display: bool,
    #[serde(default)]
    pub shell: bool,
    #[serde(default)]
    pub watchdog_secs: u32,
    #[serde(default)]
    pub mouse: bool,
}

// ── Parse + validate functions (added in T006/T007) ──────────────────────────
