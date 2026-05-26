// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Alpha compositing primitives (Porter-Duff "over" operator).
//!
//! RGBA packing convention (same as VYOMA_DRAW protocol):
//!   u32 = (R << 24) | (G << 16) | (B << 8) | A

/// Unpack an RGBA u32 into (r, g, b, a) bytes.
#[inline]
pub fn unpack(rgba: u32) -> (u8, u8, u8, u8) {
    (
        ((rgba >> 24) & 0xFF) as u8,
        ((rgba >> 16) & 0xFF) as u8,
        ((rgba >>  8) & 0xFF) as u8,
        (rgba & 0xFF) as u8,
    )
}

/// Pack (r, g, b, a) bytes into an RGBA u32.
#[inline]
pub fn pack(r: u8, g: u8, b: u8, a: u8) -> u32 {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | (a as u32)
}

/// Porter-Duff "over" operator: composite `src` over `dst`.
///
/// Both `src` and `dst` are packed RGBA u32 values (R<<24|G<<16|B<<8|A).
/// Returns the composited result as a packed RGBA u32.
///
/// Formula (straight alpha):
///   α_out = α_src + α_dst × (1 − α_src / 255)
///   C_out  = (C_src × α_src + C_dst × α_dst × (255 − α_src)) / (α_out × 255)
#[inline]
pub fn blend_over(src: u32, dst: u32) -> u32 {
    let (sr, sg, sb, sa) = unpack(src);
    let (dr, dg, db, da) = unpack(dst);

    if sa == 255 {
        return src; // fast path: fully opaque source
    }
    if sa == 0 {
        return dst; // fast path: fully transparent source
    }

    let sa = sa as u32;
    let ia = 255 - sa; // inverse alpha

    // Premultiplied-like blend without full premultiplication
    let da = da as u32;
    let a_out = sa + (da * ia + 127) / 255;

    if a_out == 0 {
        return 0;
    }

    let blend_ch = |s: u8, d: u8| -> u8 {
        let s = s as u32;
        let d = d as u32;
        ((s * sa + d * (da * ia / 255)) / a_out) as u8
    };

    pack(
        blend_ch(sr, dr),
        blend_ch(sg, dg),
        blend_ch(sb, db),
        a_out.min(255) as u8,
    )
}

/// Read a packed BGRA pixel from a back-buffer slice (4 bytes at offset `off`).
/// The framebuffer is BGRA little-endian: [B, G, R, X].
/// Returns an RGBA u32: (R<<24)|(G<<16)|(B<<8)|A.
#[inline]
pub fn read_bgra(back: &[u8], off: usize) -> u32 {
    let b = back[off]     as u32;
    let g = back[off + 1] as u32;
    let r = back[off + 2] as u32;
    let a = back[off + 3] as u32;
    (r << 24) | (g << 16) | (b << 8) | a
}

/// Write a packed RGBA u32 as BGRA bytes at offset `off` in the back-buffer.
#[inline]
pub fn write_bgra(back: &mut [u8], off: usize, rgba: u32) {
    let (r, g, b, _a) = unpack(rgba);
    back[off]     = b;
    back[off + 1] = g;
    back[off + 2] = r;
    back[off + 3] = 0xFF; // framebuffer X channel always opaque
}

/// Composite a fontdue glyph bitmap onto the framebuffer back-buffer.
///
/// `coverage`: alpha mask from fontdue (one byte per pixel, row-major).
/// `text_rgba`: text color as RGBA u32 (R<<24|G<<16|B<<8|A).
/// `fb_stride`: framebuffer stride in bytes per row.
/// `fb_width`, `fb_height`: framebuffer dimensions.
pub fn composite_glyph(
    back: &mut Vec<u8>,
    coverage: &[u8],
    glyph_w: u32,
    glyph_h: u32,
    x: i32,
    y: i32,
    text_rgba: u32,
    fb_stride: u32,
    fb_width: u32,
    fb_height: u32,
) {
    let (tr, tg, tb, ta) = unpack(text_rgba);
    for row in 0..glyph_h {
        let dst_y = y + row as i32;
        if dst_y < 0 || dst_y >= fb_height as i32 { continue; }
        for col in 0..glyph_w {
            let dst_x = x + col as i32;
            if dst_x < 0 || dst_x >= fb_width as i32 { continue; }
            let cov_idx = (row * glyph_w + col) as usize;
            let cov = *coverage.get(cov_idx).unwrap_or(&0);
            if cov == 0 { continue; }
            // Modulate text alpha by coverage
            let eff_alpha = (ta as u32 * cov as u32 / 255) as u8;
            let src = pack(tr, tg, tb, eff_alpha);
            let fb_off = (dst_y as u32 * fb_stride + dst_x as u32 * 4) as usize;
            if fb_off + 4 > back.len() { continue; }
            let dst = read_bgra(back, fb_off);
            let blended = blend_over(src, dst);
            write_bgra(back, fb_off, blended);
        }
    }
}
