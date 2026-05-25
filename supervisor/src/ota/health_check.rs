// Post-update health validation — T004
//
// `HealthChecker` waits for a module to emit heartbeats after an OTA update.
// If the module does not emit a healthy heartbeat within `timeout_secs`, the
// supervisor triggers a rollback via the OtaManager.

use std::time::{Duration, Instant};

/// Outcome of a health-check run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthCheckResult {
    /// Module emitted enough healthy heartbeats within the timeout.
    Passed,
    /// Timeout elapsed without sufficient healthy heartbeats.
    Failed,
}

/// Tracks health check state for a single module after an OTA update.
pub struct HealthChecker {
    pub module_name: String,
    pub timeout: Duration,
    pub required_checks: u32,
    pub passed: u32,
    started_at: Instant,
}

impl HealthChecker {
    /// Create a new health checker.
    ///
    /// `timeout_secs` — how long to wait for health signals before failing.
    /// `required_checks` — number of healthy heartbeats needed to confirm health.
    pub fn new(module_name: impl Into<String>, timeout_secs: u32, required_checks: u32) -> Self {
        Self {
            module_name: module_name.into(),
            timeout: Duration::from_secs(u64::from(timeout_secs)),
            required_checks,
            passed: 0,
            started_at: Instant::now(),
        }
    }

    /// Record one successful health signal.
    /// Returns `HealthCheckResult::Passed` once `required_checks` is met.
    pub fn record_healthy(&mut self) -> HealthCheckResult {
        self.passed += 1;
        if self.passed >= self.required_checks {
            HealthCheckResult::Passed
        } else {
            HealthCheckResult::Failed
        }
    }

    /// Returns `true` if the timeout has elapsed without enough health checks.
    pub fn is_timed_out(&self) -> bool {
        self.started_at.elapsed() >= self.timeout
    }

    /// Evaluate current state: passed if enough checks recorded, failed if timeout expired.
    pub fn evaluate(&self) -> HealthCheckResult {
        if self.passed >= self.required_checks {
            HealthCheckResult::Passed
        } else {
            HealthCheckResult::Failed
        }
    }
}
