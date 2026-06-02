// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Interactive window resize via mouse drag on window edges/corners.
//! Extracted into its own module to respect the 500-line file limit.

use std::sync::{Mutex, OnceLock};

use crate::{log_info, AppRegistry, FocusedApp, Inbox};
use supervisor::logging::Subsystem;

// ── Constants ────────────────────────────────────────────────────────────────

/// Width of the resize-sensitive border zone in pixels.
pub const RESIZE_BORDER: i32 = 6;

/// Minimum window dimensions during resize.
const MIN_W: u32 = 200;
const MIN_H: u32 = 100;

// ── Resize edge enum ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResizeEdge {
    Top,
    Bottom,
    Left,
    Right,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

// ── Edge detection ───────────────────────────────────────────────────────────

/// Detect whether the mouse position `(mx, my)` falls on a resize edge of the
/// window at `(wx, wy)` with dimensions `(ww, wh)`.  Returns `None` if the
/// cursor is not within `RESIZE_BORDER` pixels of any edge.
pub fn detect_resize_edge(
    mx: i32,
    my: i32,
    wx: u32,
    wy: u32,
    ww: u32,
    wh: u32,
) -> Option<ResizeEdge> {
    let left   = wx as i32;
    let top    = wy as i32;
    let right  = (wx + ww) as i32;
    let bottom = (wy + wh) as i32;

    // Must be within the outer border band (slightly outside or inside the window edge)
    let in_x = mx >= left - RESIZE_BORDER && mx < right + RESIZE_BORDER;
    let in_y = my >= top - RESIZE_BORDER && my < bottom + RESIZE_BORDER;
    if !in_x || !in_y {
        return None;
    }

    let near_left   = mx >= left - RESIZE_BORDER && mx < left + RESIZE_BORDER;
    let near_right  = mx >= right - RESIZE_BORDER && mx < right + RESIZE_BORDER;
    let near_top    = my >= top - RESIZE_BORDER && my < top + RESIZE_BORDER;
    let near_bottom = my >= bottom - RESIZE_BORDER && my < bottom + RESIZE_BORDER;

    match (near_left, near_right, near_top, near_bottom) {
        (true, _, true, _)  => Some(ResizeEdge::TopLeft),
        (true, _, _, true)  => Some(ResizeEdge::BottomLeft),
        (_, true, true, _)  => Some(ResizeEdge::TopRight),
        (_, true, _, true)  => Some(ResizeEdge::BottomRight),
        (true, _, _, _)     => Some(ResizeEdge::Left),
        (_, true, _, _)     => Some(ResizeEdge::Right),
        (_, _, true, _)     => Some(ResizeEdge::Top),
        (_, _, _, true)     => Some(ResizeEdge::Bottom),
        _                   => None,
    }
}

// ── Resize drag state ────────────────────────────────────────────────────────

pub struct ResizeDrag {
    pub app:        String,
    pub edge:       ResizeEdge,
    pub start_mx:   i32,
    pub start_my:   i32,
    pub orig_region: (u32, u32, u32, u32), // (x, y, w, h)
}

static RESIZE_DRAG: OnceLock<Mutex<Option<ResizeDrag>>> = OnceLock::new();

pub fn resize_drag() -> &'static Mutex<Option<ResizeDrag>> {
    RESIZE_DRAG.get_or_init(|| Mutex::new(None))
}

/// Returns true if a resize drag is currently in progress.
pub fn is_resizing() -> bool {
    resize_drag().lock().unwrap().is_some()
}

// ── Begin / update / finish ──────────────────────────────────────────────────

/// Start a resize drag for the given app.
pub fn begin_resize(app: String, edge: ResizeEdge, mx: i32, my: i32, region: (u32, u32, u32, u32)) {
    *resize_drag().lock().unwrap() = Some(ResizeDrag {
        app,
        edge,
        start_mx: mx,
        start_my: my,
        orig_region: region,
    });
}

/// Called on every mouse-move while the left button is held and a resize is
/// active.  Computes new window geometry from the drag delta, enforces minimum
/// size, and updates the app's `win_region` in the registry.
#[cfg(target_os = "linux")]
pub fn apply_resize_update(
    cx: i32,
    cy: i32,
    screen_w: i32,
    screen_h: i32,
    registry: &AppRegistry,
    focused: &FocusedApp,
) {
    let snap = {
        let guard = resize_drag().lock().unwrap();
        guard.as_ref().map(|rd| ResizeDrag {
            app:         rd.app.clone(),
            edge:        rd.edge,
            start_mx:    rd.start_mx,
            start_my:    rd.start_my,
            orig_region: rd.orig_region,
        })
    };
    let Some(rd) = snap else { return };

    let dx = cx - rd.start_mx;
    let dy = cy - rd.start_my;
    let (ox, oy, ow, oh) = rd.orig_region;

    let (nx, ny, nw, nh) = compute_new_geometry(rd.edge, ox, oy, ow, oh, dx, dy, screen_w, screen_h);

    {
        let reg = registry.lock().unwrap();
        if let Some(st_arc) = reg.get(&rd.app) {
            st_arc.lock().unwrap().win_region = Some((nx, ny, nw, nh));
        }
    }
    crate::draw_cmd::force_repaint(registry, focused);
}

/// Finish the resize drag on mouse-up: clear state and notify the app of its
/// new dimensions via `VYOMA_SYSTEM:resize:<w>,<h>`.
#[cfg(target_os = "linux")]
pub fn finish_resize(
    registry: &AppRegistry,
    focused: &FocusedApp,
    inbox: &Inbox,
) {
    let finished = resize_drag().lock().unwrap().take();
    let Some(rd) = finished else { return };

    let region = {
        let reg = registry.lock().unwrap();
        reg.get(&rd.app)
            .and_then(|st| st.lock().unwrap().win_region)
            .unwrap_or(rd.orig_region)
    };
    let (_, _, nw, nh) = region;

    crate::send_reply(&rd.app, &format!("VYOMA_SYSTEM:resize:{nw},{nh}"), inbox);
    crate::draw_cmd::force_repaint(registry, focused);

    log_info!(Subsystem::Input, Some(rd.app.as_str()),
        "resize finished: {}x{}", nw, nh);
}

/// Compute new (x, y, w, h) given the drag delta and the edge being dragged.
/// Enforces minimum size and clamps to screen bounds.
fn compute_new_geometry(
    edge: ResizeEdge,
    ox: u32, oy: u32, ow: u32, oh: u32,
    dx: i32, dy: i32,
    screen_w: i32, screen_h: i32,
) -> (u32, u32, u32, u32) {
    let (mut nx, mut ny, mut nw, mut nh) = (ox as i32, oy as i32, ow as i32, oh as i32);

    match edge {
        ResizeEdge::Right => {
            nw += dx;
        }
        ResizeEdge::Bottom => {
            nh += dy;
        }
        ResizeEdge::Left => {
            nx += dx;
            nw -= dx;
        }
        ResizeEdge::Top => {
            ny += dy;
            nh -= dy;
        }
        ResizeEdge::TopLeft => {
            nx += dx; nw -= dx;
            ny += dy; nh -= dy;
        }
        ResizeEdge::TopRight => {
            nw += dx;
            ny += dy; nh -= dy;
        }
        ResizeEdge::BottomLeft => {
            nx += dx; nw -= dx;
            nh += dy;
        }
        ResizeEdge::BottomRight => {
            nw += dx;
            nh += dy;
        }
    }

    // Enforce minimum size — if shrinking past minimum, clamp dimension and
    // adjust origin so the opposite edge stays fixed.
    if nw < MIN_W as i32 {
        match edge {
            ResizeEdge::Left | ResizeEdge::TopLeft | ResizeEdge::BottomLeft => {
                nx = (ox + ow) as i32 - MIN_W as i32;
            }
            _ => {}
        }
        nw = MIN_W as i32;
    }
    if nh < MIN_H as i32 {
        match edge {
            ResizeEdge::Top | ResizeEdge::TopLeft | ResizeEdge::TopRight => {
                ny = (oy + oh) as i32 - MIN_H as i32;
            }
            _ => {}
        }
        nh = MIN_H as i32;
    }

    // Clamp to screen bounds
    let menu_h = crate::chrome::MENUBAR_H as i32;
    nx = nx.clamp(0, (screen_w - MIN_W as i32).max(0));
    ny = ny.clamp(menu_h, (screen_h - MIN_H as i32).max(menu_h));
    if nx + nw > screen_w { nw = screen_w - nx; }
    if ny + nh > screen_h { nh = screen_h - ny; }

    (nx as u32, ny as u32, nw as u32, nh as u32)
}

// ── Left-click initiation helper ─────────────────────────────────────────

/// On left-click, check if the cursor is on a resize edge of any window
/// (z-order aware).  If so, begin a resize drag and return `true`.
/// Otherwise return `false` so the caller can fall through to titlebar drag.
pub fn try_begin_resize_from_click(
    cx: i32,
    cy: i32,
    registry: &AppRegistry,
) -> bool {
    let z_snap: Vec<String> = crate::Z_ORDER.get()
        .map(|m| m.lock().unwrap().clone()).unwrap_or_default();
    let hit: Option<(String, ResizeEdge, (u32, u32, u32, u32))> = {
        let reg = registry.lock().unwrap();
        let mut found = None;
        for name in &z_snap {
            let Some(st_arc) = reg.get(name) else { continue };
            let st = st_arc.lock().unwrap();
            let Some((wx, wy, ww, wh)) = st.win_region else { continue };
            if let Some(edge) = detect_resize_edge(cx, cy, wx, wy, ww, wh) {
                found = Some((name.clone(), edge, (wx, wy, ww, wh)));
                break;
            }
        }
        found
    };
    if let Some((name, edge, region)) = hit {
        begin_resize(name, edge, cx, cy, region);
        true
    } else {
        false
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edge_detection_corners() {
        // Bottom-right corner
        let edge = detect_resize_edge(299, 199, 0, 0, 300, 200);
        assert_eq!(edge, Some(ResizeEdge::BottomRight));

        // Top-left corner
        let edge = detect_resize_edge(1, 1, 0, 0, 300, 200);
        assert_eq!(edge, Some(ResizeEdge::TopLeft));
    }

    #[test]
    fn edge_detection_sides() {
        // Right edge, middle
        let edge = detect_resize_edge(299, 100, 0, 0, 300, 200);
        assert_eq!(edge, Some(ResizeEdge::Right));

        // Bottom edge, middle
        let edge = detect_resize_edge(150, 199, 0, 0, 300, 200);
        assert_eq!(edge, Some(ResizeEdge::Bottom));

        // Left edge, middle
        let edge = detect_resize_edge(1, 100, 0, 0, 300, 200);
        assert_eq!(edge, Some(ResizeEdge::Left));

        // Top edge, middle
        let edge = detect_resize_edge(150, 1, 0, 0, 300, 200);
        assert_eq!(edge, Some(ResizeEdge::Top));
    }

    #[test]
    fn edge_detection_interior_none() {
        // Center of window — no edge
        let edge = detect_resize_edge(150, 100, 0, 0, 300, 200);
        assert_eq!(edge, None);
    }

    #[test]
    fn edge_detection_outside_none() {
        // Far outside
        let edge = detect_resize_edge(500, 500, 0, 0, 300, 200);
        assert_eq!(edge, None);
    }

    #[test]
    fn compute_geometry_right_drag() {
        let (x, y, w, h) = compute_new_geometry(
            ResizeEdge::Right, 100, 100, 400, 300, 50, 0, 1440, 900,
        );
        assert_eq!((x, y, w, h), (100, 100, 450, 300));
    }

    #[test]
    fn compute_geometry_min_size_enforced() {
        // Drag left edge far right — should clamp to MIN_W
        let (x, y, w, h) = compute_new_geometry(
            ResizeEdge::Left, 100, 100, 400, 300, 350, 0, 1440, 900,
        );
        assert_eq!(w, MIN_W);
        // Origin should adjust so right edge stays at 500
        assert_eq!(x, 500 - MIN_W);
        assert_eq!(h, 300);
        let _ = y; // y unchanged
    }

    #[test]
    fn compute_geometry_bottom_left() {
        let (x, y, w, h) = compute_new_geometry(
            ResizeEdge::BottomLeft, 200, 100, 400, 300, -30, 40, 1440, 900,
        );
        assert_eq!((x, y, w, h), (170, 100, 430, 340));
    }
}
