// Health heartbeat emitter — T005 + T016
//
// Emits one JSON-line heartbeat record per running module on a configurable
// interval.  Format matches data-model.md §6 "Heartbeat Record".

use std::time::{Duration, Instant};

// ── ModuleStatus ──────────────────────────────────────────────────────────────

/// Health status reported in a heartbeat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleStatus {
    Healthy,
    Degraded,
    Unhealthy,
}

impl ModuleStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModuleStatus::Healthy => "healthy",
            ModuleStatus::Degraded => "degraded",
            ModuleStatus::Unhealthy => "unhealthy",
        }
    }
}

// ── Heartbeat ─────────────────────────────────────────────────────────────────

/// A single heartbeat snapshot for one module.
#[derive(Debug, Clone)]
pub struct Heartbeat {
    /// Always "heartbeat".
    pub record_type: &'static str,
    pub module: String,
    pub uptime_s: u64,
    pub mem_kb: u64,
    pub status: ModuleStatus,
    pub last_error: Option<String>,
}

impl Heartbeat {
    /// Construct a new heartbeat for `module`.
    pub fn new(module: impl Into<String>, uptime_s: u64, mem_kb: u64, status: ModuleStatus) -> Self {
        Self {
            record_type: "heartbeat",
            module: module.into(),
            uptime_s,
            mem_kb,
            status,
            last_error: None,
        }
    }

    /// Serialize to a single JSON line (no trailing newline).
    ///
    /// Uses manual string building to avoid pulling in `serde_json`, which
    /// is too heavy for MCU targets.
    pub fn to_json_line(&self) -> String {
        let err_field = match &self.last_error {
            Some(e) => {
                // Escape quotes and backslashes inside the error string.
                let escaped = e.replace('\\', "\\\\").replace('"', "\\\"");
                format!(r#","last_error":"{escaped}""#)
            }
            None => r#","last_error":null"#.to_string(),
        };

        format!(
            r#"{{"type":"{ty}","module":"{module}","uptime_s":{uptime},"mem_kb":{mem},"status":"{status}"{err}}}"#,
            ty = self.record_type,
            module = self.module,
            uptime = self.uptime_s,
            mem = self.mem_kb,
            status = self.status.as_str(),
            err = err_field,
        )
    }
}

// ── HeartbeatEmitter ──────────────────────────────────────────────────────────

/// Tracks per-module state and decides when the next heartbeat is due.
pub struct HeartbeatEmitter {
    pub module: String,
    pub interval: Duration,
    last_emitted: Option<Instant>,
    start: Instant,
}

impl HeartbeatEmitter {
    /// Create a new emitter for `module` with the given interval in seconds.
    pub fn new(module: impl Into<String>, interval_s: u32) -> Self {
        Self {
            module: module.into(),
            interval: Duration::from_secs(u64::from(interval_s)),
            last_emitted: None,
            start: Instant::now(),
        }
    }

    /// Return `true` if a heartbeat should be emitted now.
    pub fn is_due(&self) -> bool {
        match self.last_emitted {
            None => true,
            Some(last) => last.elapsed() >= self.interval,
        }
    }

    /// Build a heartbeat record and mark it as emitted.
    pub fn emit(&mut self, mem_kb: u64, status: ModuleStatus) -> Heartbeat {
        self.last_emitted = Some(Instant::now());
        Heartbeat::new(
            self.module.clone(),
            self.start.elapsed().as_secs(),
            mem_kb,
            status,
        )
    }
}
