// Tests for FontCache::measure_str — verifies proportionality and non-zero output.
// Only compiled on Linux (where the font module is available).

#[cfg(target_os = "linux")]
mod tests {
    use supervisor::font::cache::FontCache;

    fn make_cache() -> FontCache {
        // Font files may not exist in CI; FontCache falls back to pt*0.6 per glyph.
        FontCache::load("", "", "")
    }

    #[test]
    fn measure_str_nonzero() {
        let mut cache = make_cache();
        let w = cache.measure_str("Hi", 12, false, false);
        assert!(w > 0, "measure_str must return non-zero width for non-empty text");
    }

    #[test]
    fn measure_str_proportional() {
        let mut cache = make_cache();
        let short = cache.measure_str("Hi", 13, true, false);
        let long  = cache.measure_str("Hello World", 13, true, false);
        assert!(long > short, "longer text must be wider");
    }

    #[test]
    fn measure_str_scales_with_pt() {
        let mut cache = make_cache();
        let small = cache.measure_str("VyomaOS", 10, false, false);
        let large = cache.measure_str("VyomaOS", 20, false, false);
        assert!(large > small, "larger pt must produce wider measurement");
    }

    #[test]
    fn measure_str_empty_is_zero() {
        let mut cache = make_cache();
        assert_eq!(cache.measure_str("", 12, false, false), 0);
    }
}
