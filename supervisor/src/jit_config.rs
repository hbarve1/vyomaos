// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! P92: Cranelift JIT optimization settings.
//!
//! Provides a `JitConfig` model that controls Wasmtime compilation behavior:
//! optimization level, compiled-module cache, instruction fuel limits, and
//! epoch-based interruption.  Configuration persists to `/data/jit-config.toml`
//! and is exposed via `@supervisor:` IPC commands.

use std::fs;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use crate::{log_info, log_warn, send_reply, Inbox};
use supervisor::logging::Subsystem;

// ── JIT cache directory ──────────────────────────────────────────────────────

const CONFIG_PATH: &str = "/data/jit-config.toml";
const DEFAULT_CACHE_DIR: &str = "/data/cache/jit/";

// ── OptLevel ─────────────────────────────────────────────────────────────────

/// Cranelift optimization level mapped to `wasmtime --optimize` flag values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OptLevel {
    /// No optimizations (`-O0`).  Fastest compile, slowest execution.
    None,
    /// Optimize for execution speed (`-O speed`).
    Speed,
    /// Balance between speed and binary/memory size (`-O speed-and-size`).
    SpeedAndSize,
}

impl OptLevel {
    /// Parse from a user-facing string (IPC argument).
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().trim() {
            "none" | "0" => Some(Self::None),
            "speed" | "1" => Some(Self::Speed),
            "size" | "speed-and-size" | "speedandsize" | "2" => Some(Self::SpeedAndSize),
            _ => None,
        }
    }

    /// Short label for display / IPC replies.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Speed => "speed",
            Self::SpeedAndSize => "speed-and-size",
        }
    }

    /// Wasmtime CLI value for the `--optimize` flag (cranelift codegen level).
    fn wasmtime_flag(self) -> &'static str {
        match self {
            Self::None => "0",
            Self::Speed => "1",
            Self::SpeedAndSize => "2",
        }
    }
}

// ── JitConfig ────────────────────────────────────────────────────────────────

/// Full JIT configuration model for wasmtime invocations.
#[derive(Debug, Clone)]
pub struct JitConfig {
    /// Cranelift optimization level.
    pub optimization_level: OptLevel,
    /// Enable the compiled-module cache (wasmtime `--cache`).
    pub cache_enabled: bool,
    /// Directory for cached compiled modules.
    pub cache_dir: String,
    /// Per-app instruction fuel limit.  `None` = unlimited.
    pub fuel_limit: Option<u64>,
    /// Enable epoch-based interruption (cooperative preemption).
    pub epoch_interruption: bool,
}

impl Default for JitConfig {
    fn default() -> Self {
        let optimization_level = if cfg!(debug_assertions) {
            OptLevel::None
        } else {
            OptLevel::SpeedAndSize
        };
        Self {
            optimization_level,
            cache_enabled: true,
            cache_dir: DEFAULT_CACHE_DIR.to_string(),
            fuel_limit: None,
            epoch_interruption: true,
        }
    }
}

impl JitConfig {
    /// Generate CLI flags suitable for a `wasmtime run` invocation.
    pub fn to_wasmtime_args(&self) -> Vec<String> {
        let mut args: Vec<String> = Vec::new();

        // Optimization level
        args.push("-O".to_string());
        args.push(format!("opt-level={}", self.optimization_level.wasmtime_flag()));

        // Cache
        if self.cache_enabled {
            args.push("--cache".to_string());
            args.push(self.cache_dir.clone());
        }

        // Fuel limit
        if let Some(fuel) = self.fuel_limit {
            args.push("--fuel".to_string());
            args.push(fuel.to_string());
        }

        // Epoch interruption
        if self.epoch_interruption {
            args.push("-W".to_string());
            args.push("epoch-interruption=y".to_string());
        }

        args
    }

    /// Serialize to TOML and write to `CONFIG_PATH`.
    pub fn save(&self) -> Result<(), String> {
        let toml = self.to_toml_string();
        if let Some(parent) = Path::new(CONFIG_PATH).parent() {
            let _ = fs::create_dir_all(parent);
        }
        fs::write(CONFIG_PATH, toml).map_err(|e| format!("save jit-config: {e}"))
    }

    /// Load from `CONFIG_PATH`, falling back to defaults on any error.
    pub fn load() -> Self {
        let raw = match fs::read_to_string(CONFIG_PATH) {
            Ok(s) => s,
            Err(_) => return Self::default(),
        };
        Self::from_toml_str(&raw).unwrap_or_default()
    }

    /// Render as a human-readable TOML string.
    fn to_toml_string(&self) -> String {
        let fuel_str = match self.fuel_limit {
            Some(v) => v.to_string(),
            None => "\"unlimited\"".to_string(),
        };
        format!(
            "[jit]\n\
             optimization_level = \"{}\"\n\
             cache_enabled = {}\n\
             cache_dir = \"{}\"\n\
             fuel_limit = {}\n\
             epoch_interruption = {}\n",
            self.optimization_level.as_str(),
            self.cache_enabled,
            self.cache_dir,
            fuel_str,
            self.epoch_interruption,
        )
    }

    /// Parse from a TOML string (tolerant of missing fields).
    fn from_toml_str(raw: &str) -> Option<Self> {
        let table: toml::Value = toml::from_str(raw).ok()?;
        let jit = table.get("jit")?;

        let opt = jit.get("optimization_level")
            .and_then(|v| v.as_str())
            .and_then(OptLevel::from_str_loose)
            .unwrap_or(OptLevel::SpeedAndSize);

        let cache_enabled = jit.get("cache_enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let cache_dir = jit.get("cache_dir")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_CACHE_DIR)
            .to_string();

        let fuel_limit = jit.get("fuel_limit").and_then(|v| {
            if v.is_str() { None } else { v.as_integer().map(|n| n as u64) }
        });

        let epoch_interruption = jit.get("epoch_interruption")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        Some(Self {
            optimization_level: opt,
            cache_enabled,
            cache_dir,
            fuel_limit,
            epoch_interruption,
        })
    }

    /// Format a short summary for IPC display.
    pub fn summary(&self) -> String {
        let fuel_str = match self.fuel_limit {
            Some(v) => format!("{v}"),
            None => "unlimited".to_string(),
        };
        format!(
            "opt={} cache={} dir={} fuel={} epoch={}",
            self.optimization_level.as_str(),
            self.cache_enabled,
            self.cache_dir,
            fuel_str,
            self.epoch_interruption,
        )
    }
}

// ── Global singleton ─────────────────────────────────────────────────────────

static JIT_CONFIG: OnceLock<Mutex<JitConfig>> = OnceLock::new();

/// Get (or lazily initialize) the global JIT config.
pub fn config() -> &'static Mutex<JitConfig> {
    JIT_CONFIG.get_or_init(|| Mutex::new(JitConfig::load()))
}

// ── IPC handler ──────────────────────────────────────────────────────────────

/// Handle `@supervisor: jit-*` IPC commands.  Returns `true` if the verb was
/// recognized, `false` otherwise.
pub fn handle_jit_command(
    verb:   &str,
    parts:  &[&str],
    sender: &str,
    inbox:  &Inbox,
) -> bool {
    match verb {
        // jit-config — show current configuration
        "jit-config" => {
            let cfg = config().lock().unwrap();
            send_reply(sender, &format!("REPLY:{}", cfg.summary()), inbox);
            log_info!(Subsystem::Lifecycle, None, "jit-config query from {sender}");
        }

        // jit-set-opt <none|speed|size>
        "jit-set-opt" => {
            let arg = parts.get(1).unwrap_or(&"").trim();
            match OptLevel::from_str_loose(arg) {
                Some(level) => {
                    let mut cfg = config().lock().unwrap();
                    cfg.optimization_level = level;
                    if let Err(e) = cfg.save() {
                        log_warn!(Subsystem::Lifecycle, None, "jit-config save failed: {e}");
                    }
                    log_info!(Subsystem::Lifecycle, None,
                        "jit opt-level set to {} by {sender}", level.as_str());
                    send_reply(
                        sender,
                        &format!("REPLY:jit opt-level set to {}", level.as_str()),
                        inbox,
                    );
                }
                None => {
                    send_reply(
                        sender,
                        "REPLY:error: usage: jit-set-opt <none|speed|size>",
                        inbox,
                    );
                }
            }
        }

        // jit-cache-clear — remove all files in the cache directory
        "jit-cache-clear" => {
            let dir = config().lock().unwrap().cache_dir.clone();
            let removed = clear_cache_dir(&dir);
            log_info!(Subsystem::Lifecycle, None,
                "jit cache cleared: {removed} file(s) removed from {dir}");
            send_reply(
                sender,
                &format!("REPLY:jit cache cleared ({removed} files)"),
                inbox,
            );
        }

        // jit-set-fuel <limit|unlimited>
        "jit-set-fuel" => {
            let arg = parts.get(1).unwrap_or(&"").trim();
            let fuel = parse_fuel_arg(arg);
            match fuel {
                Some(limit) => {
                    let mut cfg = config().lock().unwrap();
                    cfg.fuel_limit = limit;
                    if let Err(e) = cfg.save() {
                        log_warn!(Subsystem::Lifecycle, None, "jit-config save failed: {e}");
                    }
                    let label = match limit {
                        Some(v) => format!("{v}"),
                        None => "unlimited".to_string(),
                    };
                    log_info!(Subsystem::Lifecycle, None,
                        "jit fuel set to {label} by {sender}");
                    send_reply(
                        sender,
                        &format!("REPLY:jit fuel set to {label}"),
                        inbox,
                    );
                }
                None => {
                    send_reply(
                        sender,
                        "REPLY:error: usage: jit-set-fuel <number|unlimited>",
                        inbox,
                    );
                }
            }
        }

        _ => return false,
    }
    true
}

/// Parse fuel argument: returns `Some(Some(n))` for a valid number,
/// `Some(None)` for "unlimited", `None` for invalid input.
fn parse_fuel_arg(s: &str) -> Option<Option<u64>> {
    let s = s.trim().to_lowercase();
    if s == "unlimited" || s == "none" || s == "0" {
        return Some(None);
    }
    s.parse::<u64>().ok().map(Some)
}

/// Remove all files in `dir`.  Returns number of files removed.
fn clear_cache_dir(dir: &str) -> usize {
    let entries = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return 0,
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            if fs::remove_file(&path).is_ok() {
                count += 1;
            }
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_debug_mode() {
        let cfg = JitConfig::default();
        assert_eq!(cfg.optimization_level, OptLevel::None); // debug_assertions=true
        assert!(cfg.cache_enabled);
        assert_eq!(cfg.cache_dir, "/data/cache/jit/");
        assert!(cfg.fuel_limit.is_none());
        assert!(cfg.epoch_interruption);
    }

    #[test]
    fn opt_level_parsing() {
        assert_eq!(OptLevel::from_str_loose("none"), Some(OptLevel::None));
        assert_eq!(OptLevel::from_str_loose("0"), Some(OptLevel::None));
        assert_eq!(OptLevel::from_str_loose("speed"), Some(OptLevel::Speed));
        assert_eq!(OptLevel::from_str_loose("1"), Some(OptLevel::Speed));
        assert_eq!(OptLevel::from_str_loose("size"), Some(OptLevel::SpeedAndSize));
        assert_eq!(OptLevel::from_str_loose("speed-and-size"), Some(OptLevel::SpeedAndSize));
        assert_eq!(OptLevel::from_str_loose("2"), Some(OptLevel::SpeedAndSize));
        assert_eq!(OptLevel::from_str_loose("invalid"), None);
        assert_eq!(OptLevel::from_str_loose(""), None);
    }

    #[test]
    fn opt_level_roundtrip() {
        for level in [OptLevel::None, OptLevel::Speed, OptLevel::SpeedAndSize] {
            assert_eq!(OptLevel::from_str_loose(level.as_str()), Some(level));
        }
    }

    #[test]
    fn wasmtime_args_default() {
        let cfg = JitConfig {
            optimization_level: OptLevel::SpeedAndSize, cache_enabled: true,
            cache_dir: "/data/cache/jit/".into(), fuel_limit: None, epoch_interruption: true,
        };
        let args = cfg.to_wasmtime_args();
        assert!(args.contains(&"opt-level=2".into()));
        assert!(args.contains(&"--cache".into()));
        assert!(!args.contains(&"--fuel".into()));
        assert!(args.contains(&"epoch-interruption=y".into()));
    }

    #[test]
    fn wasmtime_args_with_fuel() {
        let cfg = JitConfig {
            optimization_level: OptLevel::Speed, cache_enabled: false,
            cache_dir: "/tmp/cache".into(), fuel_limit: Some(100_000), epoch_interruption: false,
        };
        let args = cfg.to_wasmtime_args();
        assert!(args.contains(&"opt-level=1".into()));
        assert!(!args.contains(&"--cache".into()));
        assert!(args.contains(&"--fuel".into()));
        assert!(args.contains(&"100000".into()));
        assert!(!args.contains(&"-W".into()));
    }

    #[test]
    fn wasmtime_args_no_optimization() {
        let cfg = JitConfig {
            optimization_level: OptLevel::None, cache_enabled: false,
            cache_dir: String::new(), fuel_limit: None, epoch_interruption: false,
        };
        let args = cfg.to_wasmtime_args();
        assert!(args.contains(&"opt-level=0".into()));
        assert!(!args.contains(&"--cache".into()));
        assert!(!args.contains(&"--fuel".into()));
        assert!(!args.contains(&"-W".into()));
    }

    #[test]
    fn toml_roundtrip() {
        let cfg = JitConfig {
            optimization_level: OptLevel::Speed, cache_enabled: true,
            cache_dir: "/custom/cache".into(), fuel_limit: Some(50_000), epoch_interruption: false,
        };
        let parsed = JitConfig::from_toml_str(&cfg.to_toml_string()).unwrap();
        assert_eq!(parsed.optimization_level, OptLevel::Speed);
        assert!(parsed.cache_enabled);
        assert_eq!(parsed.cache_dir, "/custom/cache");
        assert_eq!(parsed.fuel_limit, Some(50_000));
        assert!(!parsed.epoch_interruption);
    }

    #[test]
    fn toml_roundtrip_unlimited_fuel() {
        let cfg = JitConfig { fuel_limit: None, ..JitConfig::default() };
        let parsed = JitConfig::from_toml_str(&cfg.to_toml_string()).unwrap();
        assert!(parsed.fuel_limit.is_none());
    }

    #[test]
    fn summary_format() {
        let s = JitConfig::default().summary();
        assert!(s.contains("opt=") && s.contains("cache=") && s.contains("fuel=") && s.contains("epoch="));
    }

    #[test]
    fn parse_fuel_arg_variants() {
        assert_eq!(parse_fuel_arg("unlimited"), Some(None));
        assert_eq!(parse_fuel_arg("none"), Some(None));
        assert_eq!(parse_fuel_arg("0"), Some(None));
        assert_eq!(parse_fuel_arg("100000"), Some(Some(100_000)));
        assert_eq!(parse_fuel_arg("abc"), None);
        assert_eq!(parse_fuel_arg(""), None);
    }

    #[test]
    fn from_toml_str_missing_fields() {
        let cfg = JitConfig::from_toml_str("[jit]\noptimization_level = \"speed\"\n").unwrap();
        assert_eq!(cfg.optimization_level, OptLevel::Speed);
        assert!(cfg.cache_enabled);
        assert_eq!(cfg.cache_dir, DEFAULT_CACHE_DIR);
        assert!(cfg.fuel_limit.is_none());
        assert!(cfg.epoch_interruption);
    }

    #[test]
    fn from_toml_str_invalid_returns_none() {
        assert!(JitConfig::from_toml_str("not valid toml {{{").is_none());
        assert!(JitConfig::from_toml_str("[other]\nkey = 1").is_none());
    }
}
