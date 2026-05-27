// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Scalable font cache using fontdue for TrueType rasterization.

use std::collections::HashMap;

/// Rasterized glyph coverage bitmap.
pub struct GlyphBitmap {
    /// Alpha coverage mask: one byte per pixel (0=transparent, 255=opaque).
    pub coverage: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Horizontal advance in pixels.
    pub advance_x: f32,
    /// Vertical offset: pixels above the baseline (positive = up).
    pub bearing_y: i32,
}

/// Cache key for rasterized glyphs.
#[derive(PartialEq, Eq, Hash, Clone)]
struct GlyphKey {
    ch: char,
    pt_x10: u32,    // pt_size × 10, stored as integer to allow Hash
    bold: bool,
    mono: bool,
}

/// Loaded fonts and their rasterized glyph cache.
pub struct FontCache {
    ui:      Option<fontdue::Font>,
    ui_bold: Option<fontdue::Font>,
    mono:    Option<fontdue::Font>,
    cache:   HashMap<GlyphKey, GlyphBitmap>,
}

impl FontCache {
    /// Load fonts from TTF paths. Missing files produce a warning; rendering falls back to blank glyphs.
    pub fn load(ui_path: &str, bold_path: &str, mono_path: &str) -> Self {
        fn load_font(path: &str) -> Option<fontdue::Font> {
            match std::fs::read(path) {
                Err(e) => {
                    eprintln!("[font] WARNING: cannot read {path}: {e}");
                    None
                }
                Ok(bytes) => {
                    fontdue::Font::from_bytes(
                        bytes.as_slice(),
                        fontdue::FontSettings::default(),
                    )
                    .map_err(|e| eprintln!("[font] WARNING: cannot parse {path}: {e}"))
                    .ok()
                }
            }
        }
        Self {
            ui:      load_font(ui_path),
            ui_bold: load_font(bold_path),
            mono:    load_font(mono_path),
            cache:   HashMap::new(),
        }
    }

    /// Rasterize a character. Returns a cached GlyphBitmap; blank if font unavailable.
    pub fn rasterize(&mut self, ch: char, pt: u32, bold: bool, mono: bool) -> &GlyphBitmap {
        let key = GlyphKey { ch, pt_x10: pt * 10, bold, mono };
        if !self.cache.contains_key(&key) {
            let bitmap = Self::compute_glyph(&self.ui, &self.ui_bold, &self.mono, ch, pt, bold, mono);
            self.cache.insert(key.clone(), bitmap);
        }
        &self.cache[&key]
    }

    /// Sum the advance widths of all characters in `text` at the given size.
    /// Returns total pixel width as `u32` (truncated from `f32`).
    pub fn measure_str(&mut self, text: &str, pt: u32, bold: bool, mono: bool) -> u32 {
        text.chars()
            .map(|ch| self.rasterize(ch, pt, bold, mono).advance_x)
            .sum::<f32>() as u32
    }

    fn compute_glyph(
        ui: &Option<fontdue::Font>,
        ui_bold: &Option<fontdue::Font>,
        mono: &Option<fontdue::Font>,
        ch: char,
        pt: u32,
        bold: bool,
        is_mono: bool,
    ) -> GlyphBitmap {
        let font_opt: Option<&fontdue::Font> =
            if is_mono { mono.as_ref() }
            else if bold { ui_bold.as_ref().or(ui.as_ref()) }
            else { ui.as_ref() };

        match font_opt {
            None => GlyphBitmap {
                coverage: vec![],
                width: 0,
                height: 0,
                advance_x: pt as f32 * 0.6,
                bearing_y: 0,
            },
            Some(font) => {
                let (metrics, coverage) = font.rasterize(ch, pt as f32);
                GlyphBitmap {
                    coverage,
                    width:     metrics.width as u32,
                    height:    metrics.height as u32,
                    advance_x: metrics.advance_width,
                    bearing_y: metrics.ymin as i32 + metrics.height as i32,
                }
            }
        }
    }
}
