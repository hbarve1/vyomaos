// OTA A/B slot and health-check tests — Week 1, Task 1c
//
// Tests for supervisor::ota::{AbSlot, SlotLabel, OtaManager}
// and supervisor::ota::health_check::{HealthChecker, HealthCheckResult}.

use supervisor::ota::{AbSlot, SlotLabel, OtaManager, UpdateStatus};
use supervisor::ota::health_check::{HealthChecker, HealthCheckResult};

// ── SlotLabel ────────────────────────────────────────────────────────────────

#[test]
fn slot_label_other_a_to_b() {
    assert_eq!(SlotLabel::A.other(), SlotLabel::B);
}

#[test]
fn slot_label_other_b_to_a() {
    assert_eq!(SlotLabel::B.other(), SlotLabel::A);
}

#[test]
fn slot_label_other_round_trip() {
    assert_eq!(SlotLabel::A.other().other(), SlotLabel::A);
    assert_eq!(SlotLabel::B.other().other(), SlotLabel::B);
}

#[test]
fn slot_label_display() {
    assert_eq!(format!("{}", SlotLabel::A), "A");
    assert_eq!(format!("{}", SlotLabel::B), "B");
}

// ── AbSlot ───────────────────────────────────────────────────────────────────

#[test]
fn ab_slot_new_defaults_to_slot_a() {
    let slot = AbSlot::new("my-module", "/slot-a/my-module.wasm");
    assert_eq!(slot.active, SlotLabel::A);
    assert!(!slot.health_confirmed);
    assert_eq!(slot.module_name, "my-module");
}

#[test]
fn ab_slot_active_path_returns_slot_a_initially() {
    let slot = AbSlot::new("test", "/a/test.wasm");
    assert_eq!(slot.active_path(), "/a/test.wasm");
}

#[test]
fn ab_slot_active_path_after_swap() {
    let mut slot = AbSlot::new("test", "/a/test.wasm");
    slot.slot_b_path = "/b/test.wasm".to_string();
    slot.swap();
    assert_eq!(slot.active, SlotLabel::B);
    assert_eq!(slot.active_path(), "/b/test.wasm");
}

#[test]
fn ab_slot_swap_clears_health_confirmed() {
    let mut slot = AbSlot::new("test", "/a/test.wasm");
    slot.health_confirmed = true;
    slot.swap();
    assert!(!slot.health_confirmed, "swap should clear health_confirmed");
}

#[test]
fn ab_slot_rollback_restores_previous_slot() {
    let mut slot = AbSlot::new("test", "/a/test.wasm");
    slot.slot_b_path = "/b/test.wasm".to_string();
    // Simulate: swap to B (after update), then rollback to A
    slot.swap();
    assert_eq!(slot.active, SlotLabel::B);
    slot.rollback();
    assert_eq!(slot.active, SlotLabel::A);
}

#[test]
fn ab_slot_rollback_sets_health_confirmed() {
    let mut slot = AbSlot::new("test", "/a/test.wasm");
    slot.swap();
    slot.rollback();
    assert!(slot.health_confirmed, "rollback should mark health as confirmed");
}

#[test]
fn ab_slot_standby_path_initially_empty() {
    let slot = AbSlot::new("test", "/a/test.wasm");
    assert!(slot.slot_b_path.is_empty());
}

// ── HealthChecker ────────────────────────────────────────────────────────────

#[test]
fn health_checker_starts_at_zero() {
    let hc = HealthChecker::new("test", 30, 3);
    assert_eq!(hc.passed, 0);
    assert_eq!(hc.required_checks, 3);
    assert_eq!(hc.module_name, "test");
}

#[test]
fn health_checker_evaluate_fails_when_no_checks() {
    let hc = HealthChecker::new("test", 30, 3);
    assert_eq!(hc.evaluate(), HealthCheckResult::Failed);
}

#[test]
fn health_checker_record_healthy_increments() {
    let mut hc = HealthChecker::new("test", 30, 3);
    assert_eq!(hc.record_healthy(), HealthCheckResult::Failed);
    assert_eq!(hc.passed, 1);
    assert_eq!(hc.record_healthy(), HealthCheckResult::Failed);
    assert_eq!(hc.passed, 2);
    assert_eq!(hc.record_healthy(), HealthCheckResult::Passed);
    assert_eq!(hc.passed, 3);
}

#[test]
fn health_checker_evaluate_passes_at_threshold() {
    let mut hc = HealthChecker::new("test", 30, 2);
    hc.record_healthy();
    assert_eq!(hc.evaluate(), HealthCheckResult::Failed);
    hc.record_healthy();
    assert_eq!(hc.evaluate(), HealthCheckResult::Passed);
}

#[test]
fn health_checker_evaluate_passes_above_threshold() {
    let mut hc = HealthChecker::new("test", 30, 2);
    hc.record_healthy();
    hc.record_healthy();
    hc.record_healthy(); // one extra
    assert_eq!(hc.evaluate(), HealthCheckResult::Passed);
}

#[test]
fn health_checker_single_check_required() {
    let mut hc = HealthChecker::new("test", 10, 1);
    assert_eq!(hc.record_healthy(), HealthCheckResult::Passed);
}

#[test]
fn health_checker_is_timed_out_initially_false() {
    // With a 30-second timeout, a brand-new checker should not be timed out.
    let hc = HealthChecker::new("test", 30, 3);
    assert!(!hc.is_timed_out());
}

#[test]
fn health_checker_zero_timeout_is_immediately_timed_out() {
    // A zero-second timeout means any elapsed time triggers timeout.
    let hc = HealthChecker::new("test", 0, 3);
    // Allow a tiny spin to ensure at least one nanosecond passes
    std::thread::sleep(std::time::Duration::from_millis(1));
    assert!(hc.is_timed_out());
}

// ── OtaManager ───────────────────────────────────────────────────────────────

#[test]
fn ota_manager_begin_update_returns_index() {
    let mut mgr = OtaManager::new();
    let idx = mgr.begin_update("my-mod", "1.0", "1.1", SlotLabel::B);
    assert_eq!(idx, 0);
    let idx2 = mgr.begin_update("other", "0.1", "0.2", SlotLabel::A);
    assert_eq!(idx2, 1);
}

#[test]
fn ota_manager_initial_status_is_downloading() {
    let mut mgr = OtaManager::new();
    let idx = mgr.begin_update("mod", "1.0", "2.0", SlotLabel::B);
    let rec = mgr.latest_for("mod").unwrap();
    assert_eq!(rec.status, UpdateStatus::Downloading);
    assert_eq!(rec.slot, SlotLabel::B);
    let _ = idx;
}

#[test]
fn ota_manager_set_status_transitions() {
    let mut mgr = OtaManager::new();
    let idx = mgr.begin_update("mod", "1.0", "2.0", SlotLabel::B);
    mgr.set_status(idx, UpdateStatus::Verifying);
    assert_eq!(mgr.latest_for("mod").unwrap().status, UpdateStatus::Verifying);
    mgr.set_status(idx, UpdateStatus::Deploying);
    assert_eq!(mgr.latest_for("mod").unwrap().status, UpdateStatus::Deploying);
    mgr.set_status(idx, UpdateStatus::HealthChecking);
    assert_eq!(mgr.latest_for("mod").unwrap().status, UpdateStatus::HealthChecking);
    mgr.set_status(idx, UpdateStatus::Complete);
    assert_eq!(mgr.latest_for("mod").unwrap().status, UpdateStatus::Complete);
}

#[test]
fn ota_manager_record_health_check() {
    let mut mgr = OtaManager::new();
    let idx = mgr.begin_update("mod", "1.0", "2.0", SlotLabel::B);
    mgr.record_health_check(idx);
    mgr.record_health_check(idx);
    let rec = mgr.latest_for("mod").unwrap();
    assert_eq!(rec.health_checks_passed, 2);
}

#[test]
fn ota_manager_rollback() {
    let mut mgr = OtaManager::new();
    let idx = mgr.begin_update("mod", "1.0", "2.0", SlotLabel::B);
    mgr.rollback(idx, "health check failed");
    let rec = mgr.latest_for("mod").unwrap();
    assert_eq!(rec.status, UpdateStatus::RolledBack);
    assert_eq!(rec.rollback_reason.as_deref(), Some("health check failed"));
}

#[test]
fn ota_manager_latest_for_returns_none_for_unknown() {
    let mgr = OtaManager::new();
    assert!(mgr.latest_for("nonexistent").is_none());
}

#[test]
fn ota_manager_latest_for_returns_most_recent() {
    let mut mgr = OtaManager::new();
    let idx1 = mgr.begin_update("mod", "1.0", "2.0", SlotLabel::B);
    mgr.set_status(idx1, UpdateStatus::Complete);
    let idx2 = mgr.begin_update("mod", "2.0", "3.0", SlotLabel::A);
    mgr.set_status(idx2, UpdateStatus::Deploying);
    // latest_for should return the second record
    let rec = mgr.latest_for("mod").unwrap();
    assert_eq!(rec.to_version, "3.0");
    assert_eq!(rec.status, UpdateStatus::Deploying);
}

// ── Combined OTA + health check scenario ─────────────────────────────────────

#[test]
fn ota_full_update_lifecycle() {
    let mut mgr = OtaManager::new();
    let mut slot = AbSlot::new("app", "/a/app.wasm");
    slot.slot_b_path = "/b/app.wasm".to_string();

    // Begin update targeting slot B
    let idx = mgr.begin_update("app", "1.0", "2.0", SlotLabel::B);
    mgr.set_status(idx, UpdateStatus::Verifying);
    mgr.set_status(idx, UpdateStatus::Deploying);

    // Swap to slot B
    slot.swap();
    assert_eq!(slot.active, SlotLabel::B);

    // Health checking
    mgr.set_status(idx, UpdateStatus::HealthChecking);
    let mut hc = HealthChecker::new("app", 30, 3);
    for _ in 0..3 {
        mgr.record_health_check(idx);
        hc.record_healthy();
    }
    assert_eq!(hc.evaluate(), HealthCheckResult::Passed);
    slot.health_confirmed = true;
    mgr.set_status(idx, UpdateStatus::Complete);

    assert!(slot.health_confirmed);
    assert_eq!(mgr.latest_for("app").unwrap().status, UpdateStatus::Complete);
    assert_eq!(mgr.latest_for("app").unwrap().health_checks_passed, 3);
}

#[test]
fn ota_rollback_on_health_failure() {
    let mut mgr = OtaManager::new();
    let mut slot = AbSlot::new("app", "/a/app.wasm");
    slot.slot_b_path = "/b/app.wasm".to_string();

    let idx = mgr.begin_update("app", "1.0", "2.0", SlotLabel::B);
    mgr.set_status(idx, UpdateStatus::Deploying);
    slot.swap();
    assert_eq!(slot.active, SlotLabel::B);

    // Health check fails — only 1 of required 3
    mgr.set_status(idx, UpdateStatus::HealthChecking);
    let mut hc = HealthChecker::new("app", 30, 3);
    hc.record_healthy(); // only 1
    assert_eq!(hc.evaluate(), HealthCheckResult::Failed);

    // Rollback
    slot.rollback();
    mgr.rollback(idx, "insufficient health checks");

    assert_eq!(slot.active, SlotLabel::A);
    assert!(slot.health_confirmed);
    assert_eq!(mgr.latest_for("app").unwrap().status, UpdateStatus::RolledBack);
    assert_eq!(
        mgr.latest_for("app").unwrap().rollback_reason.as_deref(),
        Some("insufficient health checks")
    );
}
