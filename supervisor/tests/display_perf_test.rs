#[cfg(target_os = "linux")]
mod tests {
    use supervisor::display;

    fn make_fb() -> display::Framebuffer {
        let (fb, _) = display::Framebuffer::new_for_test(200, 100);
        fb
    }

    #[test]
    fn dirty_rect_starts_empty() {
        let fb = make_fb();
        assert!(fb.dirty.y0 > fb.dirty.y1,
            "expected empty dirty rect, got y0={} y1={}", fb.dirty.y0, fb.dirty.y1);
    }

    #[test]
    fn fill_rect_expands_dirty() {
        let mut fb = make_fb();
        fb.fill_rect(10, 5, 50, 20, 0xFFFFFFFF);
        assert_eq!(fb.dirty.y0, 5);
        assert_eq!(fb.dirty.y1, 24);  // y + h - 1 = 5 + 20 - 1
    }

    #[test]
    fn two_fill_rects_merge_dirty() {
        let mut fb = make_fb();
        fb.fill_rect(0, 10, 50, 5, 0xFF0000FF);  // rows 10–14
        fb.fill_rect(0, 60, 50, 8, 0x00FF00FF);  // rows 60–67
        assert_eq!(fb.dirty.y0, 10);
        assert_eq!(fb.dirty.y1, 67);
    }

    #[test]
    fn flush_resets_dirty() {
        let mut fb = make_fb();
        fb.fill_rect(0, 0, 200, 100, 0xFF0000FF);
        fb.flush();
        assert!(fb.dirty.y0 > fb.dirty.y1,
            "dirty rect should be cleared after flush");
    }
}
