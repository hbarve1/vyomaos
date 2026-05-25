// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

// ── Cursor sprite constants ───────────────────────────────────────────────────

pub const CURSOR_W: u32 = 12;
pub const CURSOR_H: u32 = 19;

// Standard arrow cursor bitmask: each u16 is one row, MSB = leftmost pixel.
// 12 columns wide, 19 rows tall.
pub const CURSOR_MASK: [u16; 19] = [
    0b1000_0000_0000_0000, // row 0
    0b1100_0000_0000_0000,
    0b1110_0000_0000_0000,
    0b1111_0000_0000_0000,
    0b1111_1000_0000_0000,
    0b1111_1100_0000_0000,
    0b1111_1110_0000_0000,
    0b1111_1111_0000_0000,
    0b1111_1111_1000_0000,
    0b1111_1111_1100_0000,
    0b1111_1111_0000_0000, // row 10
    0b1111_0110_0000_0000,
    0b1110_0110_0000_0000,
    0b1100_0011_0000_0000,
    0b1000_0011_0000_0000,
    0b0000_0001_1000_0000,
    0b0000_0001_1000_0000,
    0b0000_0000_0000_0000,
    0b0000_0000_0000_0000, // row 18
];

// ── Cursor state ──────────────────────────────────────────────────────────────

pub struct CursorState {
    pub cx:          i32,
    pub cy:          i32,
    pub visible:     bool,
    /// Pixels saved from the back-buffer before the cursor was drawn (BGRA).
    pub saved_under: Vec<u8>,
    pub drawn:       bool,
}
