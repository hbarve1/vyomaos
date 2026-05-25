// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Tiled window layout engine.
//!
//! Computes non-overlapping rectangular regions for n display apps using a
//! column-first grid: columns = ceil(sqrt(n)), rows = ceil(n / columns).
//! Last row's empty slots are absorbed by expanding those tiles to fill the row.

/// Minimum content width enforced on every tiled window (pixels).
pub const MIN_WIN_W: u32 = 200;
/// Minimum content height enforced on every tiled window (pixels).
pub const MIN_WIN_H: u32 = 100;

/// Clamp a tile's `(w, h)` so it is never smaller than `(min_w, min_h)`.
///
/// This is intentionally a pure function so it can be unit-tested without any
/// platform dependencies.
pub fn clamp_tile_size(w: u32, h: u32, min_w: u32, min_h: u32) -> (u32, u32) {
    (w.max(min_w), h.max(min_h))
}

/// Compute tiled window regions for `n` display apps on a `sw × sh` screen.
///
/// Returns one `(x, y, w, h)` tuple per app in left-to-right, top-to-bottom order.
/// `min_sizes` is a parallel slice of `(min_w, min_h)` hints per app; if an app's
/// `min_w > base_cell_w`, the algorithm gives it a full-row slot (best-effort).
/// Returns an empty Vec when `n == 0`; clamps to 9 when `n > 9`.
pub fn compute_tiling(n: usize, sw: u32, sh: u32) -> Vec<(u32, u32, u32, u32)> {
    compute_tiling_with_hints(n, sw, sh, &[])
}

pub fn compute_tiling_with_hints(
    n: usize,
    sw: u32,
    sh: u32,
    min_sizes: &[(u32, u32)],
) -> Vec<(u32, u32, u32, u32)> {
    if n == 0 || sw == 0 || sh == 0 {
        return vec![];
    }
    let n = n.min(9);

    let cols = ceil_sqrt(n) as u32;
    let rows = n.div_ceil(cols as usize) as u32;

    let base_cell_w = sw / cols;
    let cell_h = sh / rows;

    let mut regions = Vec::with_capacity(n);

    for i in 0..n {
        let col = (i as u32) % cols;
        let row = (i as u32) / cols;

        let is_last_row = row == rows - 1;
        let last_row_start = (rows - 1) * cols;
        let last_row_count = n as u32 - last_row_start;

        let (x, w) = if is_last_row {
            let expanded_w = sw / last_row_count;
            let last_col = col - (last_row_start % cols);
            let x = last_col * expanded_w;
            // Absorb remainder pixels into the last tile of the row
            let w = if last_col + 1 == last_row_count {
                sw - x
            } else {
                expanded_w
            };
            (x, w)
        } else {
            // Check if this app wants a full-row slot
            let wants_full_row = min_sizes
                .get(i)
                .map(|&(mw, _)| mw > base_cell_w)
                .unwrap_or(false);
            if wants_full_row {
                (0, sw)
            } else {
                // Absorb remainder pixels into last tile of each regular row
                let x = col * base_cell_w;
                let w = if col + 1 == cols { sw - x } else { base_cell_w };
                (x, w)
            }
        };

        // Absorb remainder pixels into last row
        let y = row * cell_h;
        let h = if row + 1 == rows { sh - y } else { cell_h };

        // Enforce minimum content dimensions so apps always have usable space.
        let (w, h) = clamp_tile_size(w, h, MIN_WIN_W, MIN_WIN_H);

        regions.push((x, y, w, h));
    }

    regions
}

// ── Menu-bar hit detection ────────────────────────────────────────────────────

/// X-coordinate where the first app-name label begins in the menu bar.
///
/// Layout: "VyomaOS" starts at x=12, is 8 chars × 8 px/char = 64 px wide,
/// ending at x=76.  App labels begin at x=88 (12 px gap after the brand name).
pub const MENUBAR_APPS_START_X: i32 = 88;

/// Width (px) of a single menu-bar app label for a name of `name_len` chars.
///
/// Each label has 8 px of padding on the left and 8 px on the right, plus
/// `name_len * 8` px for the text (medium font = 8 px per character).
#[inline]
pub fn menubar_label_width(name_len: usize) -> i32 {
    name_len as i32 * 8 + 16
}

/// Return which app name was clicked in the menu bar, or `None`.
///
/// * `cx`, `cy` — screen coordinates of the click.
/// * `menubar_h` — height of the menu bar in pixels (typically 24).
/// * `apps` — ordered slice of app names as drawn left-to-right in the bar,
///   starting at [`MENUBAR_APPS_START_X`].
///
/// Returns `None` when `cy >= menubar_h as i32` (click is not in the bar) or
/// when the click does not land on any label.
pub fn menubar_hit_app<'a>(cx: i32, cy: i32, menubar_h: u32, apps: &'a [&'a str]) -> Option<&'a str> {
    if cy < 0 || cy >= menubar_h as i32 {
        return None;
    }
    let mut x = MENUBAR_APPS_START_X;
    for &name in apps {
        let w = menubar_label_width(name.len());
        if cx >= x && cx < x + w {
            return Some(name);
        }
        x += w;
    }
    None
}

// ── Snap layout ───────────────────────────────────────────────────────────────

pub fn compute_snap_layout(
    screen_w: u32,
    screen_h: u32,
    menubar_h: u32,
    focused_idx: usize,
    n_apps: usize,
) -> Vec<(u32, u32, u32, u32)> {
    if n_apps == 0 {
        return vec![];
    }

    let usable_h = screen_h.saturating_sub(menubar_h);

    if n_apps == 1 {
        return vec![(0, menubar_h, screen_w, usable_h)];
    }

    let left_w  = screen_w * 2 / 3;
    let right_x = left_w;
    let right_w = screen_w - left_w;
    let n_others = n_apps - 1;
    let slot_h  = usable_h / n_others as u32;

    let mut regions = Vec::with_capacity(n_apps);
    let mut other_idx = 0usize;

    for i in 0..n_apps {
        if i == focused_idx {
            regions.push((0, menubar_h, left_w, usable_h));
        } else {
            let y = menubar_h + other_idx as u32 * slot_h;
            let h = if other_idx + 1 == n_others {
                usable_h - other_idx as u32 * slot_h
            } else {
                slot_h
            };
            regions.push((right_x, y, right_w, h));
            other_idx += 1;
        }
    }

    regions
}

/// Compute the drag delta between a drag-start position and the current cursor position.
///
/// Returns `(dx, dy)` where positive dx means moved right and positive dy means moved down.
/// This is a pure function — it has no side effects and depends only on its arguments.
pub fn drag_delta(x1: i32, y1: i32, x2: i32, y2: i32) -> (i32, i32) {
    (x2 - x1, y2 - y1)
}


/// Integer ceiling of sqrt(n).
fn ceil_sqrt(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    let mut s = (n as f64).sqrt().ceil() as usize;
    // Correct for floating-point rounding
    while s * s < n {
        s += 1;
    }
    while s > 1 && (s - 1) * (s - 1) >= n {
        s -= 1;
    }
    s
}
