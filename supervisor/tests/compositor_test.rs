use supervisor::display::{composite_frame, CompositeEntry};

/// Helper: create a flat BGRA buffer filled with a single RGBA colour.
fn solid_bgra(w: u32, h: u32, rgba: u32) -> Vec<u8> {
    let r = ((rgba >> 24) & 0xFF) as u8;
    let g = ((rgba >> 16) & 0xFF) as u8;
    let b = ((rgba >>  8) & 0xFF) as u8;
    let a = (rgba & 0xFF) as u8;
    let pixel = [b, g, r, a];
    let mut buf = vec![0u8; (w * h * 4) as usize];
    for i in 0..(w * h) as usize {
        buf[i * 4..i * 4 + 4].copy_from_slice(&pixel);
    }
    buf
}

/// Read a BGRA pixel at (x, y) from a buffer with given stride and return (R, G, B, A).
fn read_pixel(buf: &[u8], x: u32, y: u32, stride: u32) -> (u8, u8, u8, u8) {
    let off = (y * stride + x * 4) as usize;
    (buf[off + 2], buf[off + 1], buf[off], buf[off + 3])
}

#[test]
fn test_composite_frame_single_surface() {
    let fb_w = 20u32;
    let fb_h = 20u32;
    let fb_stride = fb_w * 4;
    let mut back = vec![0u8; (fb_stride * fb_h) as usize];

    // Red 4x4 surface at (5, 5)
    let red = solid_bgra(4, 4, 0xFF0000FF);
    let entries = [CompositeEntry { x: 5, y: 5, w: 4, h: 4, pixels: &red, alpha: 255 }];
    composite_frame(&mut back, &entries, fb_stride, fb_w, fb_h);

    // Pixel at (5, 5) should be red
    let (r, g, b, _a) = read_pixel(&back, 5, 5, fb_stride);
    assert_eq!((r, g, b), (0xFF, 0x00, 0x00), "expected red at (5,5)");

    // Pixel at (0, 0) should be untouched (black/zero)
    let (r, g, b, _a) = read_pixel(&back, 0, 0, fb_stride);
    assert_eq!((r, g, b), (0, 0, 0), "expected black at (0,0)");
}

#[test]
fn test_composite_frame_z_order() {
    let fb_w = 20u32;
    let fb_h = 20u32;
    let fb_stride = fb_w * 4;
    let mut back = vec![0u8; (fb_stride * fb_h) as usize];

    // Red surface at (0,0) 10x10, then green surface at (5,5) 10x10.
    // Overlap region (5,5)-(9,9) should be green (later surface wins).
    let red   = solid_bgra(10, 10, 0xFF0000FF);
    let green = solid_bgra(10, 10, 0x00FF00FF);
    let entries = [
        CompositeEntry { x: 0, y: 0, w: 10, h: 10, pixels: &red,   alpha: 255 },
        CompositeEntry { x: 5, y: 5, w: 10, h: 10, pixels: &green, alpha: 255 },
    ];
    composite_frame(&mut back, &entries, fb_stride, fb_w, fb_h);

    // (2, 2) — only red covers this
    let (r, g, b, _) = read_pixel(&back, 2, 2, fb_stride);
    assert_eq!((r, g, b), (0xFF, 0, 0), "expected red at (2,2)");

    // (7, 7) — overlap region, green should overwrite red
    let (r, g, b, _) = read_pixel(&back, 7, 7, fb_stride);
    assert_eq!((r, g, b), (0, 0xFF, 0), "expected green at (7,7)");

    // (15, 15) — only green covers this
    let (r, g, b, _) = read_pixel(&back, 12, 12, fb_stride);
    assert_eq!((r, g, b), (0, 0xFF, 0), "expected green at (12,12)");
}

#[test]
fn test_composite_frame_alpha_blend() {
    let fb_w = 10u32;
    let fb_h = 10u32;
    let fb_stride = fb_w * 4;
    let mut back = vec![0u8; (fb_stride * fb_h) as usize];

    // Opaque white background surface (global_alpha=255 -> fast memcpy)
    let white = solid_bgra(10, 10, 0xFFFFFFFF);
    // Opaque red overlay but with global_alpha=128 -> per-pixel blend
    let red = solid_bgra(10, 10, 0xFF0000FF);

    let entries = [
        CompositeEntry { x: 0, y: 0, w: 10, h: 10, pixels: &white, alpha: 255 },
        CompositeEntry { x: 0, y: 0, w: 10, h: 10, pixels: &red, alpha: 128 },
    ];
    composite_frame(&mut back, &entries, fb_stride, fb_w, fb_h);

    let (r, g, b, _) = read_pixel(&back, 5, 5, fb_stride);
    // Red blended at ~50% over white: R high, G/B mid-range
    assert!(r > 200, "red channel should be high, got {r}");
    assert!(g < 180 && g > 50, "green channel should be mid-range, got {g}");
    assert!(b < 180 && b > 50, "blue channel should be mid-range, got {b}");
}

#[test]
fn test_composite_frame_global_alpha() {
    let fb_w = 10u32;
    let fb_h = 10u32;
    let fb_stride = fb_w * 4;
    let mut back = vec![0u8; (fb_stride * fb_h) as usize];

    // White background
    let white = solid_bgra(10, 10, 0xFFFFFFFF);
    // Opaque red but with global_alpha = 128 (should blend like semi-transparent)
    let red = solid_bgra(10, 10, 0xFF0000FF);

    let entries = [
        CompositeEntry { x: 0, y: 0, w: 10, h: 10, pixels: &white, alpha: 255 },
        CompositeEntry { x: 0, y: 0, w: 10, h: 10, pixels: &red, alpha: 128 },
    ];
    composite_frame(&mut back, &entries, fb_stride, fb_w, fb_h);

    let (r, g, b, _) = read_pixel(&back, 5, 5, fb_stride);
    // Should be a blend: not pure red, not pure white
    assert!(r > 150, "red channel should be elevated, got {r}");
    assert!(g > 50 && g < 200, "green should be mid, got {g}");
    assert!(b > 50 && b < 200, "blue should be mid, got {b}");
}

#[test]
fn test_composite_frame_zero_alpha_skipped() {
    let fb_w = 10u32;
    let fb_h = 10u32;
    let fb_stride = fb_w * 4;
    let mut back = vec![0u8; (fb_stride * fb_h) as usize];

    let red = solid_bgra(10, 10, 0xFF0000FF);
    // alpha=0 should be completely invisible
    let entries = [
        CompositeEntry { x: 0, y: 0, w: 10, h: 10, pixels: &red, alpha: 0 },
    ];
    composite_frame(&mut back, &entries, fb_stride, fb_w, fb_h);

    let (r, g, b, _) = read_pixel(&back, 5, 5, fb_stride);
    assert_eq!((r, g, b), (0, 0, 0), "alpha=0 surface should be invisible");
}
