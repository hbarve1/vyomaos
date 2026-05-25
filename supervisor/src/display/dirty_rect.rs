// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

/// Bounding scanline range for back-buffer writes since the last flush.
/// Empty state: `y0 > y1` (use `DirtyRect::empty(height)`).
#[derive(Clone, Copy)]
pub struct DirtyRect {
    pub y0: u32,
    pub y1: u32,
}

impl DirtyRect {
    pub fn empty(height: u32) -> Self { Self { y0: height, y1: 0 } }
    pub fn is_empty(&self) -> bool { self.y0 > self.y1 }
    pub fn expand(&mut self, row_start: u32, row_end_inclusive: u32) {
        self.y0 = self.y0.min(row_start);
        self.y1 = self.y1.max(row_end_inclusive);
    }
}
