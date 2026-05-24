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

#[derive(Debug, Default, Deserialize, Clone)]
pub struct WindowRegion {
    // Screen coordinates used by supervisor for hit-testing and z-ordering.
    #[serde(default)]
    pub x: u32,
    #[serde(default)]
    pub y: u32,
    #[serde(default)]
    pub w: u32,
    #[serde(default)]
    pub h: u32,
    // App-declared display metadata (optional; ignored by supervisor layout engine).
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
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

// ── Parse + validate functions ────────────────────────────────────────────────

/// Read and deserialize a `vyoma.toml` manifest file.
/// Returns `Err` for I/O failures, TOML parse errors, or unknown capability fields.
pub fn parse_manifest(path: &std::path::Path) -> Result<AppManifest, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    toml::from_str::<AppManifest>(&raw)
        .map_err(|e| format!("invalid manifest {}: {e}", path.display()))
}

/// Validate a parsed manifest against already-registered app names.
/// Returns `Err` if the app name is already in `registered_names`.
pub fn validate_manifest(m: &AppManifest, registered_names: &[&str]) -> Result<(), String> {
    if registered_names.contains(&m.app.name.as_str()) {
        return Err(format!("duplicate app name: {}", m.app.name));
    }
    Ok(())
}
