// OTA update system — T004
//
// `OtaManager` tracks update state for all modules.
// `AbSlot` represents one of the two firmware slots (A or B).
// Detailed per-module logic lives in ab_slot.rs and health_check.rs.

pub mod ab_slot;
pub mod health_check;

pub use ab_slot::{AbSlot, SlotLabel};

// ── UpdateStatus ──────────────────────────────────────────────────────────────

/// Lifecycle state of an in-progress OTA update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateStatus {
    Downloading,
    Verifying,
    Deploying,
    HealthChecking,
    Complete,
    RolledBack,
    Failed(String),
}

// ── UpdateRecord ──────────────────────────────────────────────────────────────

/// Tracks one OTA update attempt for a single module.
#[derive(Debug, Clone)]
pub struct UpdateRecord {
    pub module_name: String,
    pub from_version: String,
    pub to_version: String,
    pub slot: SlotLabel,
    pub status: UpdateStatus,
    pub health_checks_passed: u32,
    pub rollback_reason: Option<String>,
}

impl UpdateRecord {
    pub fn new(
        module_name: impl Into<String>,
        from_version: impl Into<String>,
        to_version: impl Into<String>,
        slot: SlotLabel,
    ) -> Self {
        Self {
            module_name: module_name.into(),
            from_version: from_version.into(),
            to_version: to_version.into(),
            slot,
            status: UpdateStatus::Downloading,
            health_checks_passed: 0,
            rollback_reason: None,
        }
    }
}

// ── OtaManager ───────────────────────────────────────────────────────────────

/// Coordinates OTA updates across all modules.
///
/// In the current implementation this is a simple in-memory tracker.
/// A full implementation will add persistent state, signature verification,
/// and network download logic.
#[derive(Debug, Default)]
pub struct OtaManager {
    records: Vec<UpdateRecord>,
}

impl OtaManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin an update for `module_name` from `from_version` to `to_version`.
    /// Returns the index of the new `UpdateRecord`.
    pub fn begin_update(
        &mut self,
        module_name: impl Into<String>,
        from_version: impl Into<String>,
        to_version: impl Into<String>,
        slot: SlotLabel,
    ) -> usize {
        let rec = UpdateRecord::new(module_name, from_version, to_version, slot);
        self.records.push(rec);
        self.records.len() - 1
    }

    /// Advance an update record to the next status.
    pub fn set_status(&mut self, idx: usize, status: UpdateStatus) {
        if let Some(rec) = self.records.get_mut(idx) {
            rec.status = status;
        }
    }

    /// Record a successful health check for a module update.
    pub fn record_health_check(&mut self, idx: usize) {
        if let Some(rec) = self.records.get_mut(idx) {
            rec.health_checks_passed += 1;
        }
    }

    /// Mark an update as rolled back with a reason.
    pub fn rollback(&mut self, idx: usize, reason: impl Into<String>) {
        if let Some(rec) = self.records.get_mut(idx) {
            rec.status = UpdateStatus::RolledBack;
            rec.rollback_reason = Some(reason.into());
        }
    }

    /// Return the latest update record for `module_name`, if any.
    pub fn latest_for(&self, module_name: &str) -> Option<&UpdateRecord> {
        self.records.iter().rev().find(|r| r.module_name == module_name)
    }
}
