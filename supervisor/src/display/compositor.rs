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

/// Blit an RGBA image onto the framebuffer using nearest-neighbor scaling.
/// `img_rgba`: raw RGBA bytes (4 bytes per pixel).
/// `img_w`, `img_h`: source image dimensions.
/// `dst_x`, `dst_y`: destination top-left on screen.
/// `dst_w`, `dst_h`: target dimensions (image is scaled to fit).
pub fn blit_image(
    back: &mut Vec<u8>,
    img_rgba: &[u8],
    img_w: u32,
    img_h: u32,
    dst_x: u32,
    dst_y: u32,
    dst_w: u32,
    dst_h: u32,
    fb_stride: u32,
    fb_width: u32,
    fb_height: u32,
) {
    if img_w == 0 || img_h == 0 || dst_w == 0 || dst_h == 0 { return; }

    for dy in 0..dst_h {
        let fy = dy * img_h / dst_h;
        let screen_y = dst_y + dy;
        if screen_y >= fb_height { break; }

        for dx in 0..dst_w {
            let fx = dx * img_w / dst_w;
            let screen_x = dst_x + dx;
            if screen_x >= fb_width { continue; }

            let src_off = ((fy * img_w + fx) * 4) as usize;
            if src_off + 4 > img_rgba.len() { continue; }

            let src_r = img_rgba[src_off];
            let src_g = img_rgba[src_off + 1];
            let src_b = img_rgba[src_off + 2];
            let src_a = img_rgba[src_off + 3];

            let fb_off = (screen_y * fb_stride + screen_x * 4) as usize;
            if fb_off + 4 > back.len() { continue; }

            if src_a == 255 {
                back[fb_off]     = src_b;
                back[fb_off + 1] = src_g;
                back[fb_off + 2] = src_r;
                back[fb_off + 3] = 0xFF;
            } else if src_a > 0 {
                let src = pack(src_r, src_g, src_b, src_a);
                let dst = read_bgra(back, fb_off);
                let blended = blend_over(src, dst);
                write_bgra(back, fb_off, blended);
            }
        }
    }
}

/// Compute per-pixel coverage for a rounded rectangle using signed-distance field.
/// Returns a value in 0–255: 255 = fully inside, 0 = fully outside.
/// `px`, `py`: pixel coordinates relative to rect origin (0,0).
/// `w`, `h`: rectangle dimensions. `radius`: corner radius in pixels.
#[inline]
pub fn rounded_rect_coverage(px: i32, py: i32, w: u32, h: u32, radius: u32) -> u8 {
    let r = radius as i32;
    let w = w as i32;
    let h = h as i32;
    if px < 0 || py < 0 || px >= w || py >= h { return 0; }

    // Distance to the nearest corner arc center
    let cx = if px < r { r } else if px >= w - r { w - r - 1 } else { px };
    let cy = if py < r { r } else if py >= h - r { h - r - 1 } else { py };

    let is_corner = (px < r || px >= w - r) && (py < r || py >= h - r);
    if !is_corner { return 255; }

    let dx = px - cx;
    let dy = py - cy;
    let dist_sq = dx * dx + dy * dy;
    let r_sq = r * r;

    if dist_sq <= r_sq {
        255
    } else {
        // Anti-alias: one pixel feather at the edge
        let dist = (dist_sq as f32).sqrt();
        let edge = r as f32;
        let cov = (edge + 1.0 - dist).max(0.0).min(1.0);
        (cov * 255.0) as u8
    }
}

/// Draw a rounded rectangle with alpha compositing.
pub fn draw_rounded_rect(
    back: &mut Vec<u8>,
    x: u32, y: u32, w: u32, h: u32,
    rgba: u32, radius: u32,
    fb_stride: u32, fb_width: u32, fb_height: u32,
) {
    let (r, g, b, a) = unpack(rgba);
    for row in 0..h {
        let screen_y = y + row;
        if screen_y >= fb_height { break; }
        for col in 0..w {
            let screen_x = x + col;
            if screen_x >= fb_width { continue; }
            let cov = rounded_rect_coverage(col as i32, row as i32, w, h, radius);
            if cov == 0 { continue; }
            let eff_alpha = (a as u32 * cov as u32 / 255) as u8;
            let src = pack(r, g, b, eff_alpha);
            let fb_off = (screen_y * fb_stride + screen_x * 4) as usize;
            if fb_off + 4 > back.len() { continue; }
            let dst = read_bgra(back, fb_off);
            let blended = blend_over(src, dst);
            write_bgra(back, fb_off, blended);
        }
    }
}

/// Generate a drop shadow alpha mask using 2-pass separable box blur.
/// Returns a Vec<u8> of `w × h` bytes, where 255 = fully shadowed.
/// `shape_w`, `shape_h`: size of the window casting the shadow.
/// `blur_r`: box blur radius in pixels.
#[allow(dead_code)]
pub fn generate_shadow_mask(shape_w: u32, shape_h: u32, blur_r: u32) -> Vec<u8> {
    let bw = shape_w + blur_r * 2;
    let bh = shape_h + blur_r * 2;
    let n = (bw * bh) as usize;
    let mut mask = vec![0u8; n];

    // Fill the window shape as 255 in the centre
    for row in 0..shape_h {
        let base = ((row + blur_r) * bw + blur_r) as usize;
        for col in 0..shape_w {
            mask[base + col as usize] = 255;
        }
    }

    // Horizontal pass
    let mut tmp = vec![0u8; n];
    let k = blur_r * 2 + 1;
    for row in 0..bh {
        let mut sum: u32 = 0;
        for col in 0..k.min(bw) {
            sum += mask[(row * bw + col) as usize] as u32;
        }
        for col in 0..bw {
            tmp[(row * bw + col) as usize] = (sum / k) as u8;
            let add_col = col + k;
            if add_col < bw {
                sum += mask[(row * bw + add_col) as usize] as u32;
            }
            if col >= 1 {
                let sub_col = col - 1;
                if sub_col < bw {
                    sum = sum.saturating_sub(mask[(row * bw + sub_col) as usize] as u32);
                }
            }
        }
    }

    // Vertical pass
    let mut out = vec![0u8; n];
    for col in 0..bw {
        let mut sum: u32 = 0;
        for row in 0..k.min(bh) {
            sum += tmp[(row * bw + col) as usize] as u32;
        }
        for row in 0..bh {
            out[(row * bw + col) as usize] = (sum / k) as u8;
            let add_row = row + k;
            if add_row < bh {
                sum += tmp[(add_row * bw + col) as usize] as u32;
            }
            if row >= 1 {
                let sub_row = row - 1;
                if sub_row < bh {
                    sum = sum.saturating_sub(tmp[(sub_row * bw + col) as usize] as u32);
                }
            }
        }
    }
    out
}

/// Descriptor for a single surface to be composited onto the back-buffer.
///
/// Fields: `(x, y, w, h, surface_pixels, alpha)` where `surface_pixels`
/// is a BGRA byte slice and `alpha` is the global opacity (0–255).
#[allow(dead_code)]
pub struct CompositeEntry<'a> {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub pixels: &'a [u8],
    pub alpha: u8,
}

/// Composite a list of surfaces onto a back-buffer in order (painter's algorithm).
///
/// Each entry is blitted at its `(x, y)` position with per-surface global alpha.
/// Surfaces later in the list paint over earlier ones.  The caller is responsible
/// for clearing the back-buffer to the desired background colour beforehand.
#[allow(dead_code)]
pub fn composite_frame(
    back: &mut [u8],
    surfaces: &[CompositeEntry<'_>],
    fb_stride: u32,
    fb_width: u32,
    fb_height: u32,
) {
    for entry in surfaces {
        if entry.alpha == 0 || entry.w == 0 { continue; }
        let src_stride = entry.w * 4;

        if entry.alpha == 255 {
            // Fast path: row-level memcpy (no per-pixel blend).
            let visible_w = entry.w.min(fb_width.saturating_sub(entry.x));
            if visible_w == 0 { continue; }
            let copy_bytes = (visible_w * 4) as usize;
            for row in 0..entry.h {
                let screen_y = entry.y + row;
                if screen_y >= fb_height { break; }
                let src_start = (row * src_stride) as usize;
                let fb_start  = (screen_y * fb_stride + entry.x * 4) as usize;
                if src_start + copy_bytes > entry.pixels.len() { break; }
                if fb_start  + copy_bytes > back.len()         { break; }
                back[fb_start..fb_start + copy_bytes]
                    .copy_from_slice(&entry.pixels[src_start..src_start + copy_bytes]);
            }
        } else {
            // Slow path: per-pixel alpha blend with attenuated global alpha.
            for row in 0..entry.h {
                let screen_y = entry.y + row;
                if screen_y >= fb_height { break; }
                for col in 0..entry.w {
                    let screen_x = entry.x + col;
                    if screen_x >= fb_width { continue; }
                    let src_off = (row * src_stride + col * 4) as usize;
                    if src_off + 4 > entry.pixels.len() { continue; }
                    let fb_off = (screen_y * fb_stride + screen_x * 4) as usize;
                    if fb_off + 4 > back.len() { continue; }
                    let src = read_bgra(entry.pixels, src_off);
                    let (sr, sg, sb, sa) = unpack(src);
                    let eff_a = (sa as u32 * entry.alpha as u32 / 255) as u8;
                    let attenuated = pack(sr, sg, sb, eff_a);
                    let dst = read_bgra(back, fb_off);
                    write_bgra(back, fb_off, blend_over(attenuated, dst));
                }
            }
        }
    }
}

/// Fill a rectangular region with a vertical linear gradient interpolating
/// between `top_rgba` and `bottom_rgba`. Each row is a single blended color.
pub fn draw_vertical_gradient(
    back: &mut Vec<u8>,
    x: u32, y: u32, w: u32, h: u32,
    top_rgba: u32, bottom_rgba: u32,
    stride: u32, sw: u32, sh: u32,
) {
    if h == 0 || w == 0 { return; }
    let (tr, tg, tb, ta) = unpack(top_rgba);
    let (br, bg, bb, ba) = unpack(bottom_rgba);
    let denom = (h - 1).max(1) as f32;
    for row in 0..h {
        let py = y + row;
        if py >= sh { break; }
        let t = row as f32 / denom;
        let r = (tr as f32 + (br as f32 - tr as f32) * t) as u8;
        let g = (tg as f32 + (bg as f32 - tg as f32) * t) as u8;
        let b = (tb as f32 + (bb as f32 - tb as f32) * t) as u8;
        let a = (ta as f32 + (ba as f32 - ta as f32) * t) as u8;
        let row_color = pack(r, g, b, a);
        for col in 0..w {
            let px = x + col;
            if px >= sw { break; }
            let fb_off = (py * stride + px * 4) as usize;
            if fb_off + 4 > back.len() { continue; }
            if a == 255 {
                write_bgra(back, fb_off, row_color);
            } else if a > 0 {
                let dst = read_bgra(back, fb_off);
                let blended = blend_over(row_color, dst);
                write_bgra(back, fb_off, blended);
            }
        }
    }
}

/// Fill a region with a multi-stop vertical gradient.
/// `stops` is a slice of `(position_0_to_1, rgba)` pairs, sorted by position.
pub fn draw_multi_gradient(
    back: &mut Vec<u8>,
    x: u32, y: u32, w: u32, h: u32,
    stops: &[(f32, u32)],
    stride: u32, sw: u32, sh: u32,
) {
    if h == 0 || w == 0 || stops.is_empty() { return; }
    if stops.len() == 1 {
        // Single stop: solid fill with that color.
        let rgba = stops[0].1;
        for row in 0..h {
            let py = y + row;
            if py >= sh { break; }
            for col in 0..w {
                let px = x + col;
                if px >= sw { break; }
                let fb_off = (py * stride + px * 4) as usize;
                if fb_off + 4 > back.len() { continue; }
                write_bgra(back, fb_off, rgba);
            }
        }
        return;
    }
    let denom = (h - 1).max(1) as f32;
    for row in 0..h {
        let py = y + row;
        if py >= sh { break; }
        let t = row as f32 / denom;
        // Find the two stops that bracket `t`.
        let (lo_pos, lo_rgba, hi_pos, hi_rgba) = find_bracket(stops, t);
        let seg_t = if (hi_pos - lo_pos).abs() < 1e-6 { 0.0 }
                    else { (t - lo_pos) / (hi_pos - lo_pos) };
        let row_color = lerp_rgba(lo_rgba, hi_rgba, seg_t);
        let (_, _, _, a) = unpack(row_color);
        for col in 0..w {
            let px = x + col;
            if px >= sw { break; }
            let fb_off = (py * stride + px * 4) as usize;
            if fb_off + 4 > back.len() { continue; }
            if a == 255 {
                write_bgra(back, fb_off, row_color);
            } else if a > 0 {
                let dst = read_bgra(back, fb_off);
                let blended = blend_over(row_color, dst);
                write_bgra(back, fb_off, blended);
            }
        }
    }
}

/// Find the two gradient stops that bracket parameter `t` (0..1).
fn find_bracket(stops: &[(f32, u32)], t: f32) -> (f32, u32, f32, u32) {
    if t <= stops[0].0 { return (stops[0].0, stops[0].1, stops[0].0, stops[0].1); }
    let last = stops.len() - 1;
    if t >= stops[last].0 { return (stops[last].0, stops[last].1, stops[last].0, stops[last].1); }
    for i in 0..last {
        if t >= stops[i].0 && t <= stops[i + 1].0 {
            return (stops[i].0, stops[i].1, stops[i + 1].0, stops[i + 1].1);
        }
    }
    (stops[last].0, stops[last].1, stops[last].0, stops[last].1)
}

/// Linearly interpolate between two RGBA colors.
fn lerp_rgba(a: u32, b: u32, t: f32) -> u32 {
    let (ar, ag, ab, aa) = unpack(a);
    let (br, bg, bb, ba) = unpack(b);
    let r = (ar as f32 + (br as f32 - ar as f32) * t) as u8;
    let g = (ag as f32 + (bg as f32 - ag as f32) * t) as u8;
    let bl = (ab as f32 + (bb as f32 - ab as f32) * t) as u8;
    let al = (aa as f32 + (ba as f32 - aa as f32) * t) as u8;
    pack(r, g, bl, al)
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
