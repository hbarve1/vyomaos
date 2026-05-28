// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! T075: Tests for SHOW_DOCK profile gating.
//!
//! Uses a simulation struct that mirrors the dock-gate logic:
//! when show_dock is false the dock strip must not be reserved.

// ── Simulation struct mirroring the dock-gate logic ───────────────────────────

struct DockLayoutSim {
    show_dock: bool,
}

impl DockLayoutSim {
    fn new(show_dock: bool) -> Self {
        Self { show_dock }
    }

    /// Returns true if a pinned app would receive a dock-strip region.
    /// Mirrors the guard in `apply_tiling_layout` keyed on SHOW_DOCK.
    fn dock_strip_assigned(&self, has_pinned_apps: bool) -> bool {
        if !self.show_dock {
            return false;
        }
        has_pinned_apps
    }
}

// ── T075-1: dock renders when show_dock = true ────────────────────────────────

#[test]
fn dock_renders_when_show_dock_true() {
    let sim = DockLayoutSim::new(true);
    assert!(
        sim.dock_strip_assigned(true),
        "dock strip should be assigned when show_dock=true and pinned apps exist"
    );
}

// ── T075-2: dock skips when show_dock = false ─────────────────────────────────

#[test]
fn dock_skips_when_show_dock_false() {
    let sim = DockLayoutSim::new(false);
    assert!(
        !sim.dock_strip_assigned(true),
        "dock strip must not be assigned when show_dock=false"
    );
}

// ── T075-3: no pinned apps → no dock strip regardless of show_dock ───────────

#[test]
fn no_pinned_apps_no_dock_strip() {
    let sim_on  = DockLayoutSim::new(true);
    let sim_off = DockLayoutSim::new(false);
    assert!(!sim_on .dock_strip_assigned(false), "no pinned apps → no dock even when show_dock=true");
    assert!(!sim_off.dock_strip_assigned(false), "no pinned apps → no dock when show_dock=false");
}
