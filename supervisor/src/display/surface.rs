// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Per-window pixel surface buffer.
//!
//! Each running display app owns a `Surface` sized to its content area.
//! Draw commands write to the surface at local (0-based) coordinates.
//! The supervisor compositor blits surfaces in Z-order onto the framebuffer
//! back-buffer on every flush.

use super::compositor::{blend_over, read_bgra, write_bgra};
use crate::font;

/// Window-content pixel buffer in BGRA 32bpp format.
#[allow(dead_code)]
pub struct Surface {
    pub buf:    Vec<u8>,
    pub width:  u32,
    pub height: u32,
    pub stride: u32,  // bytes per scanline = width * 4
}

#[allow(dead_code)]
impl Surface {
    /// Create a zeroed surface of the given pixel dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width * 4;
        Self { buf: vec![0u8; (stride * height) as usize], width, height, stride }
    }

    /// Fill a rectangle with an RGBA colour (packed 0xRRGGBBAA).
    /// Fully opaque fill uses a fast path; semi-transparent blends via Porter-Duff "over".
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: u32) {
        let a = (rgba & 0xFF) as u8;
        let x1 = (x + w).min(self.width);
        let y1 = (y + h).min(self.height);
        if x1 <= x || y1 <= y { return; }
        if a == 255 {
            let r = ((rgba >> 24) & 0xFF) as u8;
            let g = ((rgba >> 16) & 0xFF) as u8;
            let b = ((rgba >>  8) & 0xFF) as u8;
            let pixel = [b, g, r, 0xFF_u8];
            for row in y..y1 {
                let base = (row * self.stride + x * 4) as usize;
                let end  = (row * self.stride + x1 * 4) as usize;
                if end > self.buf.len() { break; }
                for off in (base..end).step_by(4) {
                    self.buf[off..off + 4].copy_from_slice(&pixel);
                }
            }
        } else {
            for row in y..y1 {
                for col in x..x1 {
                    let off = (row * self.stride + col * 4) as usize;
                    if off + 4 > self.buf.len() { break; }
                    let dst = read_bgra(&self.buf, off);
                    write_bgra(&mut self.buf, off, blend_over(rgba, dst));
                }
            }
        }
    }

    /// Render a string at (x, y) using the embedded 8×16 bitmap font.
    pub fn draw_text_bitmap(&mut self, x: u32, y: u32, text: &str, rgba: u32) {
        let r = ((rgba >> 24) & 0xFF) as u8;
        let g = ((rgba >> 16) & 0xFF) as u8;
        let b = ((rgba >>  8) & 0xFF) as u8;
        let fg = [b, g, r, 0xFF_u8];
        let (glyph_w, glyph_h) = (font::GLYPH_W, font::GLYPH_H);
        let mut cx = x;
        for ch in text.chars() {
            if cx + glyph_w > self.width { break; }
            let glyph_base = if (ch as u32) >= font::FIRST_CHAR as u32
                && (ch as u32) <= font::LAST_CHAR as u32
            {
                (ch as usize - font::FIRST_CHAR as usize) * glyph_h as usize
            } else { 0 };
            for row in 0..glyph_h {
                let scan_y = y + row;
                if scan_y >= self.height { break; }
                let byte = font::FONT[glyph_base + row as usize];
                for bit in 0..glyph_w {
                    if byte & (0x80 >> bit) != 0 {
                        let px = cx + bit;
                        let off = (scan_y * self.stride + px * 4) as usize;
                        if off + 4 <= self.buf.len() {
                            self.buf[off..off + 4].copy_from_slice(&fg);
                        }
                    }
                }
            }
            cx += glyph_w;
        }
    }

    /// Draw a 1-pixel border rectangle (no fill).
    pub fn rect_border(&mut self, x: u32, y: u32, w: u32, h: u32, rgba: u32) {
        if w == 0 || h == 0 { return; }
        self.fill_rect(x, y, w, 1, rgba);
        self.fill_rect(x, y + h - 1, w, 1, rgba);
        self.fill_rect(x, y, 1, h, rgba);
        self.fill_rect(x + w - 1, y, 1, h, rgba);
    }

    /// Clear a region to transparent black.
    pub fn clear_region(&mut self, x: u32, y: u32, w: u32, h: u32) {
        self.fill_rect(x, y, w, h, 0x00000000);
    }

    /// Zero the entire surface.
    pub fn clear(&mut self) {
        self.buf.iter_mut().for_each(|b| *b = 0);
    }
}

/// Composite a `Surface` onto a framebuffer back-buffer at screen position `(dx, dy)`.
///
/// `global_alpha`: 255 = fully opaque (fast path), 0 = invisible, 1–254 = animation fade.
#[allow(dead_code)]
pub fn blit_surface(
    fb_back: &mut Vec<u8>,
    surface: &Surface,
    dx: u32,
    dy: u32,
    global_alpha: u8,
    fb_stride: u32,
    fb_width: u32,
    fb_height: u32,
) {
    if global_alpha == 0 { return; }
    for row in 0..surface.height {
        let screen_y = dy + row;
        if screen_y >= fb_height { break; }
        for col in 0..surface.width {
            let screen_x = dx + col;
            if screen_x >= fb_width { continue; }
            let src_off = (row * surface.stride + col * 4) as usize;
            if src_off + 4 > surface.buf.len() { continue; }
            let fb_off = (screen_y * fb_stride + screen_x * 4) as usize;
            if fb_off + 4 > fb_back.len() { continue; }
            if global_alpha == 255 {
                fb_back[fb_off..fb_off + 4].copy_from_slice(&surface.buf[src_off..src_off + 4]);
            } else {
                let src = read_bgra(&surface.buf, src_off);
                let (sr, sg, sb, _) = super::compositor::unpack(src);
                let attenuated = super::compositor::pack(sr, sg, sb, global_alpha);
                let dst = read_bgra(fb_back, fb_off);
                write_bgra(fb_back, fb_off, blend_over(attenuated, dst));
            }
        }
    }
}
