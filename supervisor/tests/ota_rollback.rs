// T021: Failing tests for OTA A/B slot rollback.
//
// Scenario: a new module version is deployed to the inactive slot.
// If the health checker fails (module silent / unhealthy), the supervisor
// rolls back to the previously active slot.

use supervisor::ota::{OtaManager, UpdateStatus};
use supervisor::ota::ab_slot::{AbSlot, SlotLabel};
use supervisor::ota::health_check::{HealthChecker, HealthCheckResult};

// ── T021-1: new AbSlot starts on slot A ───────────────────────────────────────

#[test]
fn test_ab_slot_initial_state_is_slot_a() {
    let slot = AbSlot::new("my-module", "/data/slots/a/my-module.wasm");
    assert_eq!(slot.active, SlotLabel::A);
    assert!(!slot.health_confirmed, "health not yet confirmed");
}

// ── T021-2: deploying to inactive slot writes to B path ──────────────────────

#[test]
fn test_deploy_writes_to_inactive_slot() {
    let mut slot = AbSlot::new("my-module", "/data/slots/a/my-module.wasm");

    // Simulate deploying a new binary to the inactive (B) slot.
    slot.slot_b_path = "/data/slots/b/my-module.wasm".to_string();

    // After deploy, swap to make B active.
    slot.swap();

    assert_eq!(slot.active, SlotLabel::B);
    assert_eq!(slot.active_path(), "/data/slots/b/my-module.wasm");
    assert!(!slot.health_confirmed, "health not confirmed after swap");
}

// ── T021-3: rollback reverts active slot back to A ────────────────────────────

#[test]
fn test_rollback_reverts_to_slot_a() {
    let mut slot = AbSlot::new("my-module", "/data/slots/a/my-module.wasm");
    slot.slot_b_path = "/data/slots/b/my-module.wasm".to_string();

    slot.swap(); // Now on B
    assert_eq!(slot.active, SlotLabel::B);

    slot.rollback(); // Back to A
    assert_eq!(slot.active, SlotLabel::A);
    assert_eq!(slot.active_path(), "/data/slots/a/my-module.wasm");
    assert!(slot.health_confirmed, "previous slot was already confirmed");
}

// ── T021-4: health checker fails when timeout exceeded before enough checks ───

#[test]
fn test_health_checker_fails_without_checks() {
    // 60 second timeout, needs 3 checks, but we never record any.
    let checker = HealthChecker::new("my-module", 60, 3);
    assert_eq!(checker.evaluate(), HealthCheckResult::Failed);
}

// ── T021-5: health checker passes when enough healthy signals received ─────────

#[test]
fn test_health_checker_passes_after_required_checks() {
    let mut checker = HealthChecker::new("my-module", 60, 3);
    checker.record_healthy();
    checker.record_healthy();
    let result = checker.record_healthy(); // third check
    assert_eq!(result, HealthCheckResult::Passed);
    assert_eq!(checker.evaluate(), HealthCheckResult::Passed);
}

// ── T021-6: health checker fails with partial checks ─────────────────────────

#[test]
fn test_health_checker_partial_checks_fails() {
    let mut checker = HealthChecker::new("my-module", 60, 3);
    checker.record_healthy();
    checker.record_healthy();
    // Only 2 out of 3 required checks recorded.
    assert_eq!(checker.evaluate(), HealthCheckResult::Failed);
}

// ── T021-7: OtaManager rolls back on health check failure ────────────────────

#[test]
fn test_ota_manager_rollback_on_health_failure() {
    let mut mgr = OtaManager::new();
    let idx = mgr.begin_update("sensor-app", "v1.0", "v1.1", SlotLabel::B);

    // Advance to health-checking state.
    mgr.set_status(idx, UpdateStatus::HealthChecking);

    // Health check fails — trigger rollback.
    mgr.rollback(idx, "module did not emit heartbeat within timeout");

    let record = mgr.latest_for("sensor-app").expect("record exists");
    assert_eq!(record.status, UpdateStatus::RolledBack);
    assert!(record.rollback_reason.is_some());
    let reason = record.rollback_reason.as_ref().unwrap();
    assert!(reason.contains("heartbeat"), "reason should mention heartbeat: {reason}");
}

// ── T021-8: OtaManager completes successfully after health checks pass ─────────

#[test]
fn test_ota_manager_complete_after_health_checks() {
    let mut mgr = OtaManager::new();
    let idx = mgr.begin_update("sensor-app", "v1.0", "v1.1", SlotLabel::B);

    mgr.set_status(idx, UpdateStatus::HealthChecking);
    mgr.record_health_check(idx);
    mgr.record_health_check(idx);
    mgr.record_health_check(idx);
    mgr.set_status(idx, UpdateStatus::Complete);

    let record = mgr.latest_for("sensor-app").expect("record exists");
    assert_eq!(record.status, UpdateStatus::Complete);
    assert_eq!(record.health_checks_passed, 3);
    assert!(record.rollback_reason.is_none());
}

// ── T021-9: SlotLabel alternates correctly ────────────────────────────────────

#[test]
fn test_slot_label_other() {
    assert_eq!(SlotLabel::A.other(), SlotLabel::B);
    assert_eq!(SlotLabel::B.other(), SlotLabel::A);
}

// ── T021-10: end-to-end OTA rollback scenario ────────────────────────────────

#[test]
fn test_end_to_end_ota_rollback_scenario() {
    // 1. Module starts on slot A.
    let mut ab = AbSlot::new("edge-fw", "/slots/a/edge-fw.wasm");
    ab.health_confirmed = true;

    // 2. New version deployed to slot B.
    ab.slot_b_path = "/slots/b/edge-fw.wasm".to_string();
    ab.swap();
    assert_eq!(ab.active, SlotLabel::B);
    assert!(!ab.health_confirmed);

    // 3. OTA manager tracks this update.
    let mut mgr = OtaManager::new();
    let idx = mgr.begin_update("edge-fw", "1.0.0", "1.1.0", SlotLabel::B);
    mgr.set_status(idx, UpdateStatus::HealthChecking);

    // 4. Health checker times out (no heartbeats received).
    let checker = HealthChecker::new("edge-fw", 30, 2);
    if checker.evaluate() == HealthCheckResult::Failed {
        // 5. Rollback both OTA record and A/B slot.
        mgr.rollback(idx, "health check timeout: no heartbeat");
        ab.rollback();
    }

    // 6. Assert final state: slot A active, OTA rolled back.
    assert_eq!(ab.active, SlotLabel::A);
    assert!(ab.health_confirmed);
    let rec = mgr.latest_for("edge-fw").unwrap();
    assert_eq!(rec.status, UpdateStatus::RolledBack);
}
