// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

use super::{
    NOTES, SCALE_INTERVALS, SCALE_LEN, CHORD_INTERVALS, CIRCLE_POS,
    C_BG, C_BORDER, C_TEXT, C_HINT, C_ORANGE, C_SEL, C_GREEN, C_YELLOW, C_CARD,
};

pub fn lcg(s: u64) -> u64 {
    s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407)
}

pub fn scale_notes(root: usize, scale_idx: usize) -> Vec<usize> {
    let intervals = SCALE_INTERVALS[scale_idx];
    let len = SCALE_LEN[scale_idx];
    let mut notes = vec![root];
    let mut pos = root;
    for i in 0..len - 1 {
        pos = (pos + intervals[i] as usize) % 12;
        notes.push(pos);
    }
    notes
}

pub fn chord_notes(root: usize, chord_idx: usize) -> Vec<usize> {
    let intervals = CHORD_INTERVALS[chord_idx];
    let mut notes = vec![root];
    let mut pos = root;
    for i in 0..3 {
        pos = (pos + intervals[i] as usize) % 12;
        notes.push(pos);
    }
    notes
}

// Draw 12-note circle (note wheel). Center cx, cy, radius r.
pub fn draw_note_wheel(cx: i32, cy: i32, r: i32, active: &[usize], root: usize) {
    // Notes arranged like a clock: C at top, going clockwise
    for i in 0..12usize {
        // angle: i * 30 degrees, 0 = top, clockwise
        let (sin_i, cos_i) = CIRCLE_POS[i];
        let nx = cx + (r as i64 * sin_i / 1000) as i32;
        let ny = cy - (r as i64 * cos_i / 1000) as i32;

        let is_active = active.contains(&i);
        let is_root   = i == root;
        let bg = if is_root   { C_ORANGE }
                 else if is_active { C_SEL }
                 else { C_CARD };
        let bc = if is_root   { C_YELLOW }
                 else if is_active { C_GREEN }
                 else { C_BORDER };

        let nw = 24;
        let nh = 18;
        super::fill(nx - nw / 2, ny - nh / 2, nw, nh, bg);
        super::border(nx - nw / 2, ny - nh / 2, nw, nh, bc);
        let note = NOTES[i];
        let nc = if is_root { C_BG } else if is_active { C_TEXT } else { C_HINT };
        let nx_text = nx - (note.len() as i32 * 8) / 2;
        super::text(nx_text, ny - 8, nc, note);
    }
}
