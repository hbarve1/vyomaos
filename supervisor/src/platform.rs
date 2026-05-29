// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Platform profile loader (T017) and watchdog backoff helper (P19).

use supervisor::profile;
use crate::{log_info, log_warn};
use supervisor::logging::Subsystem;

const PLATFORM_PROFILE_DIR: &str = "/etc/vyoma/profiles";
const DEFAULT_PROFILE_NAME: &str = "desktop-full";

/// Load the active platform profile from disk.
///
/// Resolution order:
/// 1. `PLATFORM` environment variable (e.g. `PLATFORM=iot-edge`)
/// 2. Default: `desktop-full`
///
/// Profile file is looked up at `PLATFORM_PROFILE_DIR/<name>.toml`.
/// If the file does not exist, logs a warning and returns `None`
/// (system continues with defaults).
pub fn load_platform_profile() -> Option<profile::PlatformProfile> {
    let name = std::env::var("PLATFORM")
        .unwrap_or_else(|_| DEFAULT_PROFILE_NAME.to_string());
    let path = format!("{PLATFORM_PROFILE_DIR}/{name}.toml");
    match profile::load_profile(std::path::Path::new(&path)) {
        Ok(p) => {
            log_info!(
                Subsystem::Lifecycle,
                None,
                "platform profile loaded: {} (runtime={:?}, ram={}KB)",
                p.platform.name,
                p.platform.runtime,
                p.platform.min_ram_kb
            );
            Some(p)
        }
        Err(profile::ProfileError::Io(_)) => {
            // Profile file absent — acceptable on desktop where no profile is deployed.
            log_info!(
                Subsystem::Lifecycle,
                None,
                "no platform profile at {path}, using defaults"
            );
            None
        }
        Err(e) => {
            log_warn!(Subsystem::Lifecycle, None, "platform profile error: {e}");
            None
        }
    }
}

/// Exponential-backoff delay (seconds) before restarting a watchdog-killed app.
pub fn watchdog_next_backoff(watchdog_secs: u32, restarts: u32) -> u64 {
    let base = watchdog_secs as u64;
    let factor = 1u64 << restarts.min(8);
    (base * factor).min(300)
}
