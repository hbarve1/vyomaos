// Profile loader — T012
//
// Parses a TOML profile file from disk, then validates it against the rules
// defined in contracts/platform-profile-schema.md.

use super::{ObservabilityTier, PlatformProfile, Runtime};

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors produced during profile loading or validation.
#[derive(Debug, PartialEq, Eq)]
pub enum ProfileError {
    /// File I/O failure.
    Io(String),
    /// TOML parse error.
    Parse(String),
    /// A validation rule from the schema was violated.
    Validation(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::Io(e) => write!(f, "I/O error: {e}"),
            ProfileError::Parse(e) => write!(f, "parse error: {e}"),
            ProfileError::Validation(e) => write!(f, "validation error: {e}"),
        }
    }
}

// ── load_profile ──────────────────────────────────────────────────────────────

/// Read `path`, deserialize it as a `PlatformProfile`, and validate it.
///
/// Returns `Ok(PlatformProfile)` on success, `Err(ProfileError)` otherwise.
pub fn load_profile(path: &std::path::Path) -> Result<PlatformProfile, ProfileError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| ProfileError::Io(format!("cannot read {}: {e}", path.display())))?;

    let profile: PlatformProfile = toml::from_str(&raw)
        .map_err(|e| ProfileError::Parse(format!("invalid profile {}: {e}", path.display())))?;

    validate_profile(&profile)?;

    Ok(profile)
}

// ── validate_profile ──────────────────────────────────────────────────────────

/// Validate a parsed `PlatformProfile` against the schema rules.
///
/// Rules (contracts/platform-profile-schema.md):
/// 1. runtime MUST be one of wasm3 / wamr / wasmtime (enforced by serde).
/// 2. supervisor.modules MUST include "lifecycle" and "capability".
/// 3. (hal.drivers validation against arch is deferred to runtime — cannot
///    enumerate available drivers at parse time without a HalProvider.)
/// 4. (boot.apps manifest presence validation is deferred to boot time.)
/// 5. If runtime = wasm3 and min_ram_kb < 64 → warning logged (not an error).
/// 6. If runtime = wasmtime and min_ram_kb < 4096 → error.
/// 7. observability.tier = full requires runtime = wasmtime.
pub fn validate_profile(profile: &PlatformProfile) -> Result<(), ProfileError> {
    let modules = &profile.supervisor.modules;

    // Rule 2 — required supervisor modules
    if !modules.iter().any(|m| m == "lifecycle") {
        return Err(ProfileError::Validation(
            "supervisor.modules must include \"lifecycle\"".to_string(),
        ));
    }
    if !modules.iter().any(|m| m == "capability") {
        return Err(ProfileError::Validation(
            "supervisor.modules must include \"capability\"".to_string(),
        ));
    }

    // Rule 6 — Wasmtime needs at least 4 MB
    if profile.platform.runtime == Runtime::Wasmtime && profile.platform.min_ram_kb < 4096 {
        return Err(ProfileError::Validation(format!(
            "runtime wasmtime requires min_ram_kb >= 4096, got {}",
            profile.platform.min_ram_kb
        )));
    }

    // Rule 7 — full telemetry only supported with Wasmtime
    if profile.observability.tier == ObservabilityTier::Full
        && profile.platform.runtime != Runtime::Wasmtime
    {
        return Err(ProfileError::Validation(
            "observability.tier = \"full\" requires runtime = \"wasmtime\"".to_string(),
        ));
    }

    Ok(())
}
