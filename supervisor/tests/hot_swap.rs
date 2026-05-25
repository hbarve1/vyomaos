// T031: Failing unit tests for hot-swap coordinator.
//
// Tests verify that replacing a running module does not affect sibling modules.
// The `HotSwapCoordinator` struct is implemented in T036.
//
// Test strategy: use an in-process module registry that records which modules
// were stopped/started.  The coordinator must:
//   1. Identify the target module by name.
//   2. Stop only that module (no others).
//   3. Start the replacement module in its place.
//   4. Report success.

use supervisor::hot_swap::{HotSwapCoordinator, HotSwapError, ModuleState};

// ── helpers ───────────────────────────────────────────────────────────────────

fn make_coordinator(names: &[&str]) -> HotSwapCoordinator {
    let mut c = HotSwapCoordinator::new();
    for name in names {
        c.register(name.to_string());
    }
    c
}

// ── T031-1: swapping an existing module succeeds ──────────────────────────────

#[test]
fn test_hotswap_existing_module_succeeds() {
    let mut coord = make_coordinator(&["camera", "lidar", "planner", "motor"]);
    let result = coord.swap("planner", "planner-v2");
    assert!(result.is_ok(), "hot-swap should succeed: {:?}", result);
}

// ── T031-2: swap does NOT stop sibling modules ────────────────────────────────

#[test]
fn test_hotswap_does_not_stop_siblings() {
    let mut coord = make_coordinator(&["camera", "lidar", "planner", "motor"]);
    coord.swap("planner", "planner-v2").expect("swap should succeed");

    // All other modules must still be Running.
    for name in &["camera", "lidar", "motor"] {
        assert_eq!(
            coord.state(name),
            Some(ModuleState::Running),
            "{name} should still be running after hot-swap"
        );
    }
}

// ── T031-3: replaced module is running under the new name ────────────────────

#[test]
fn test_hotswap_new_module_is_running() {
    let mut coord = make_coordinator(&["camera", "lidar", "planner", "motor"]);
    coord.swap("planner", "planner-v2").expect("swap should succeed");

    assert_eq!(
        coord.state("planner-v2"),
        Some(ModuleState::Running),
        "planner-v2 should be running after swap"
    );
}

// ── T031-4: old module name is gone after swap ────────────────────────────────

#[test]
fn test_hotswap_old_module_removed() {
    let mut coord = make_coordinator(&["camera", "lidar", "planner", "motor"]);
    coord.swap("planner", "planner-v2").expect("swap should succeed");

    assert_eq!(
        coord.state("planner"),
        None,
        "old planner should be removed after swap"
    );
}

// ── T031-5: swapping a non-existent module returns an error ──────────────────

#[test]
fn test_hotswap_nonexistent_module_returns_err() {
    let mut coord = make_coordinator(&["camera", "motor"]);
    let result = coord.swap("nonexistent", "new-module");
    assert!(matches!(result, Err(HotSwapError::NotFound(_))),
        "swapping unknown module should return NotFound error");
}

// ── T031-6: four modules scenario — only planner replaced ────────────────────

#[test]
fn test_hotswap_four_module_scenario() {
    let mut coord = make_coordinator(&["camera", "lidar", "planner", "motor"]);
    let result = coord.swap("planner", "planner-v2");
    assert!(result.is_ok());

    // All three surviving modules unaffected.
    assert_eq!(coord.state("camera"),     Some(ModuleState::Running));
    assert_eq!(coord.state("lidar"),      Some(ModuleState::Running));
    assert_eq!(coord.state("motor"),      Some(ModuleState::Running));
    assert_eq!(coord.state("planner-v2"), Some(ModuleState::Running));
    assert_eq!(coord.state("planner"),    None);
    assert_eq!(coord.module_count(), 4);
}
