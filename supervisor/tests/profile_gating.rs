// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! T075/T077/T078: Tests for profile gating flags.
//!
//! Uses simulation structs that mirror the production logic so tests run
//! without a framebuffer or app registry.

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

// ── T077: windowed_mode full-screen expansion ─────────────────────────────────

/// Simulation that mirrors the windowed_mode guard in `apply_tiling_layout`.
struct WindowedLayoutSim {
    windowed_mode: bool,
    screen_w: u32,
    screen_h: u32,
}

impl WindowedLayoutSim {
    fn new(windowed_mode: bool) -> Self {
        Self { windowed_mode, screen_w: 1440, screen_h: 900 }
    }

    /// Returns the region assigned to the first tiled app (index 0).
    /// Mirrors the T077 branch: when !windowed_mode the first app gets (0,0,w,h).
    fn first_app_region(&self, tiled_count: usize) -> Option<(u32, u32, u32, u32)> {
        if tiled_count == 0 { return None; }
        if !self.windowed_mode {
            Some((0, 0, self.screen_w, self.screen_h))
        } else {
            // Normal tiling: first app gets roughly half the usable area
            let menubar_h: u32 = 24;
            let usable_h = self.screen_h.saturating_sub(menubar_h);
            Some((0, menubar_h, self.screen_w / tiled_count as u32, usable_h))
        }
    }
}

// T077-1: full-screen mode expands first app to full screen dimensions
#[test]
fn fullscreen_mode_expands_first_app() {
    let sim = WindowedLayoutSim::new(false);
    let region = sim.first_app_region(1).expect("should assign a region");
    assert_eq!(region, (0, 0, 1440, 900),
        "!windowed_mode → first app must fill the full screen (0,0,sw,sh)");
}

// T077-2: windowed_mode=true keeps normal tiled regions (not full-screen)
#[test]
fn windowed_mode_keeps_tiled_regions() {
    let sim = WindowedLayoutSim::new(true);
    let region = sim.first_app_region(2).expect("should assign a region");
    // In windowed mode the app starts below the menu bar and is not full-screen
    assert_ne!(region.1, 0, "windowed_mode=true → y must be > 0 (below menu bar)");
    assert!(region.2 < 1440, "windowed_mode=true → width must be less than full screen (2-app split)");
}

// ── T078: focus_ring profile flag ────────────────────────────────────────────

/// Simulation that mirrors the FOCUS_RING guard in `draw_chrome_onto`.
struct FocusRingSim {
    focus_ring: bool,
}

impl FocusRingSim {
    fn new(focus_ring: bool) -> Self { Self { focus_ring } }

    /// Returns true when the focus ring should be drawn for a focused/hovered window.
    fn should_draw_ring(&self, is_focused: bool, is_hovered: bool) -> bool {
        self.focus_ring && (is_focused || is_hovered)
    }
}

// T078-1: focus ring drawn when profile enables it and window is focused
#[test]
fn focus_ring_drawn_when_profile_enables_it() {
    let sim = FocusRingSim::new(true);
    assert!(sim.should_draw_ring(true, false),
        "focus_ring=true + focused window → ring must be drawn");
    assert!(sim.should_draw_ring(false, true),
        "focus_ring=true + hovered window → ring must be drawn");
}

// T078-2: focus ring skipped when profile disables it
#[test]
fn focus_ring_skipped_when_disabled() {
    let sim = FocusRingSim::new(false);
    assert!(!sim.should_draw_ring(true, false),
        "focus_ring=false → ring must NOT be drawn even for focused window");
    assert!(!sim.should_draw_ring(false, true),
        "focus_ring=false → ring must NOT be drawn even for hovered window");
}
