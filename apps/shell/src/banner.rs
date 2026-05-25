// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Welcome banner drawn once on shell startup before the interactive loop.

use crate::ui::{fill, text, flush, PX, PY};
use std::io::Write;

// Banner dimensions (local window coords)
const BANNER_X: u32 = PX + 8;
const BANNER_Y: u32 = PY + 32; // below title bar
const LINE_H:   u32 = 16;

// Colors
const C_ACCENT:  u32 = 0x58A6FFFF;
const C_DIM:     u32 = 0x8B949EFF;
const C_GREEN:   u32 = 0x3FB950FF;

// ASCII art lines — kept to 46 chars to fit in 680px window (8px/char medium font)
const BANNER_LINES: &[&str] = &[
    " __   __                      ___  ___ ",
    r" \ \ / /_  _  ___  _ __  __ _/ _ \/ __|",
    r"  \ V / || || || || '_ \/ _` | (_) \__ \",
    r"   \_/ \_, |_/ \/ ||_.__/\__,_|\___/|___/",
    "       |__/",
];

const SUBTITLE: &str = " VyomaOS Desktop -- Press any key to begin";
const STATUS:   &str = " Screen: 1440x900  |  Apps: loading...";

pub fn draw_banner(sw: u32) {
    let pw = sw.saturating_sub(PX * 2);

    // Clear banner area
    let banner_h = (BANNER_LINES.len() as u32 + 3) * LINE_H + 8;
    fill(PX + 8, BANNER_Y, pw.saturating_sub(16), banner_h, 0x161B22FF);

    // Draw ASCII art in accent color
    for (i, &line) in BANNER_LINES.iter().enumerate() {
        let y = BANNER_Y + i as u32 * LINE_H;
        text(BANNER_X, y, C_ACCENT, line);
    }

    // Subtitle line
    let sub_y = BANNER_Y + BANNER_LINES.len() as u32 * LINE_H + 4;
    text(BANNER_X, sub_y, C_GREEN, SUBTITLE);

    // Status line
    let stat_y = sub_y + LINE_H + 2;
    text(BANNER_X, stat_y, C_DIM, STATUS);

    let _ = std::io::stdout().flush();
    flush();
}
