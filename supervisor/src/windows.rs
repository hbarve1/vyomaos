// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Tiled window layout engine.
//!
//! Computes non-overlapping rectangular regions for n display apps using a
//! column-first grid: columns = ceil(sqrt(n)), rows = ceil(n / columns).
//! Last row's empty slots are absorbed by expanding those tiles to fill the row.

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

        regions.push((x, y, w, h));
    }

    regions
}

/// Compute window regions for the "Alt+F snap" layout.
///
/// The focused app expands to fill the left 2/3 of the usable area; all other
/// display apps share a right column (1/3 width) stacked vertically.  If there
/// is only one app it receives the full screen (minus the menu bar).
///
/// # Parameters
/// - `screen_w`, `screen_h`: total framebuffer dimensions in pixels.
/// - `menubar_h`: height of the global menu bar (pixels to reserve at the top).
/// - `focused_idx`: index of the focused app within the sorted app list.
/// - `n_apps`: total number of display apps.
///
/// # Returns
/// One `(wx, wy, ww, wh)` tuple per app in the same order as the input list.
/// Returns an empty `Vec` when `n_apps == 0`.
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
    let mut other_idx = 0usize; // counter for non-focused apps

    for i in 0..n_apps {
        if i == focused_idx {
            regions.push((0, menubar_h, left_w, usable_h));
        } else {
            let y = menubar_h + other_idx as u32 * slot_h;
            // Last slot absorbs any remainder pixels.
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
