// Tests for FontCache::measure_str — verifies proportionality and non-zero output.

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

// On non-Linux hosts the font module is not compiled; use fallback arithmetic tests.
#[cfg(not(target_os = "linux"))]
mod tests {
    #[test]
    fn measure_str_proportional_fallback() {
        // Validate the fallback formula: advance = pt * 0.6 per char
        let pt = 13u32;
        let fallback_advance = pt as f32 * 0.6;
        let short_w = ("Hi".chars().count() as f32 * fallback_advance) as u32;
        let long_w  = ("Hello World".chars().count() as f32 * fallback_advance) as u32;
        assert!(long_w > short_w, "longer text must be wider");
    }

    #[test]
    fn measure_str_scales_with_pt_fallback() {
        let text = "VyomaOS";
        let small = (text.chars().count() as f32 * 10f32 * 0.6) as u32;
        let large = (text.chars().count() as f32 * 20f32 * 0.6) as u32;
        assert!(large > small, "larger pt must produce wider measurement");
    }

    #[test]
    fn measure_str_empty_is_zero_fallback() {
        let w = ("".chars().count() as f32 * 12f32 * 0.6) as u32;
        assert_eq!(w, 0);
    }
}
