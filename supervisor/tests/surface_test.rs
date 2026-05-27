use supervisor::display::Surface;

#[test]
fn surface_fill_rect_opaque() {
    let mut s = Surface::new(100, 50);
    s.fill_rect(0, 0, 100, 50, 0xFF0000FF); // solid red
    // Check first pixel is red in BGRA: [0x00, 0x00, 0xFF, 0xFF]
    assert_eq!(s.buf[0], 0x00); // B
    assert_eq!(s.buf[1], 0x00); // G
    assert_eq!(s.buf[2], 0xFF); // R
    assert_eq!(s.buf[3], 0xFF); // A
}

#[test]
fn surface_fill_rect_clips() {
    let mut s = Surface::new(10, 10);
    s.fill_rect(8, 8, 100, 100, 0xFFFFFFFF); // extends far past boundary
    // Should not panic and pixel at (9,9) should be written
    let off = (9 * s.stride + 9 * 4) as usize;
    assert_eq!(s.buf[off + 2], 0xFF); // R channel = 0xFF (white)
}

#[test]
fn surface_new_is_zeroed() {
    let s = Surface::new(32, 32);
    assert!(s.buf.iter().all(|&b| b == 0), "fresh surface must be zeroed");
}

#[test]
fn surface_clear_zeroes_buf() {
    let mut s = Surface::new(32, 32);
    s.fill_rect(0, 0, 32, 32, 0xFFFFFFFF);
    s.clear();
    assert!(s.buf.iter().all(|&b| b == 0));
}

#[test]
fn blit_surface_copies_pixels() {
    use supervisor::display::surface::blit_surface;
    let mut src = Surface::new(4, 4);
    src.fill_rect(0, 0, 4, 4, 0x0000FFFF); // solid blue
    let fw = 20u32; let fh = 20u32; let fs = fw * 4;
    let mut fb_back = vec![0u8; (fs * fh) as usize];
    blit_surface(&mut fb_back, &src, 2, 3, 255, fs, fw, fh);
    // Pixel at screen (2, 3) should be blue BGRA = [0xFF, 0x00, 0x00, 0xFF]
    let off = (3 * fs + 2 * 4) as usize;
    assert_eq!(fb_back[off],     0xFF); // B
    assert_eq!(fb_back[off + 1], 0x00); // G
    assert_eq!(fb_back[off + 2], 0x00); // R
}
