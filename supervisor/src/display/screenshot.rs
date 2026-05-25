// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! PPM screenshot export for the back-buffer (debug / testing utility).

use super::Framebuffer;

impl Framebuffer {
    /// Write the current back-buffer as a raw PPM (P6) file to `path`.
    /// Pixels are in BGRA order in the back-buffer; output is RGB.
    pub fn screenshot(&self, path: &str) -> Result<(), String> {
        use std::io::Write as IoWrite;
        let mut rgb = Vec::with_capacity(3 * (self.width * self.height) as usize);
        for row in 0..self.height {
            for col in 0..self.width {
                let off = (row * self.stride + col * 4) as usize;
                if off + 4 <= self.back.len() {
                    let b = self.back[off];
                    let g = self.back[off + 1];
                    let r = self.back[off + 2];
                    rgb.push(r);
                    rgb.push(g);
                    rgb.push(b);
                }
            }
        }
        let header = format!("P6\n{} {}\n255\n", self.width, self.height);
        let mut f = std::fs::File::create(path)
            .map_err(|e| format!("create {path}: {e}"))?;
        f.write_all(header.as_bytes()).map_err(|e| format!("write: {e}"))?;
        f.write_all(&rgb).map_err(|e| format!("write: {e}"))?;
        Ok(())
    }
}
