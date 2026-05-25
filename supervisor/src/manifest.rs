// Manifest module — app manifest structs and pure parsing/validation functions.

use serde::Deserialize;

use crate::capability::peripheral::PeripheralEnforcer;

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
    /// Peripheral capability enforcer derived from `[capabilities.gpio]` etc.
    /// Populated by `parse_manifest`; not from serde directly.
    #[serde(skip)]
    pub peripherals: Option<PeripheralEnforcer>,
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
    /// App receives `VYOMA_INPUT:touch:` events when touch = true in manifest.
    #[serde(default)]
    pub touch: bool,
}

// ── Parse + validate functions ────────────────────────────────────────────────

/// Read and deserialize a `vyoma.toml` manifest file.
///
/// Returns `Err` for I/O failures, TOML parse errors, or unknown capability fields.
///
/// T018: Also extracts `[capabilities.gpio]`, `[capabilities.i2c]` etc. into
/// `AppManifest::peripherals` as a `PeripheralEnforcer`.  The peripheral
/// sub-tables are stripped before the normal serde deserialization so the
/// `deny_unknown_fields` on `Capabilities` is not triggered.
pub fn parse_manifest(path: &std::path::Path) -> Result<AppManifest, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;

    // Parse as a generic TOML value first to extract peripheral sub-tables.
    let mut doc: toml::Value = toml::from_str(&raw)
        .map_err(|e| format!("invalid manifest {}: {e}", path.display()))?;

    // Extract [capabilities.gpio/i2c/spi/uart/adc] before strict struct parse.
    let peripheral_toml = extract_peripheral_toml(&mut doc);

    // Now re-serialise the sanitised doc and parse into AppManifest.
    let sanitised = toml::to_string(&doc)
        .map_err(|e| format!("manifest re-serialise error: {e}"))?;
    let mut manifest: AppManifest = toml::from_str(&sanitised)
        .map_err(|e| format!("invalid manifest {}: {e}", path.display()))?;

    // Parse peripheral capabilities (may be empty if no sub-tables declared).
    manifest.peripherals = match PeripheralEnforcer::from_toml(&peripheral_toml) {
        Ok(enforcer) => Some(enforcer),
        Err(e) => return Err(format!("peripheral capability error in {}: {e}", path.display())),
    };

    Ok(manifest)
}

/// Extract peripheral sub-tables from the `[capabilities]` table in a TOML value.
///
/// Removes `gpio`, `i2c`, `spi`, `uart`, `adc` keys from `[capabilities]` and
/// returns them as a TOML string suitable for `PeripheralEnforcer::from_toml`.
fn extract_peripheral_toml(doc: &mut toml::Value) -> String {
    const PERIPHERAL_KEYS: &[&str] = &["gpio", "i2c", "spi", "uart", "adc"];

    let caps_table = match doc
        .as_table_mut()
        .and_then(|t| t.get_mut("capabilities"))
        .and_then(|v| v.as_table_mut())
    {
        Some(t) => t,
        None => return String::new(),
    };

    let mut peripheral_entries = Vec::new();
    for key in PERIPHERAL_KEYS {
        if let Some(val) = caps_table.remove(*key) {
            // Serialise each peripheral sub-table back to TOML.
            if let Ok(s) = toml::to_string(&toml::Value::Table({
                let mut t = toml::value::Table::new();
                t.insert((*key).to_string(), val);
                t
            })) {
                peripheral_entries.push(s);
            }
        }
    }

    peripheral_entries.join("\n")
}

/// Validate a parsed manifest against already-registered app names.
/// Returns `Err` if the app name is already in `registered_names`.
pub fn validate_manifest(m: &AppManifest, registered_names: &[&str]) -> Result<(), String> {
    if registered_names.contains(&m.app.name.as_str()) {
        return Err(format!("duplicate app name: {}", m.app.name));
    }
    Ok(())
}
