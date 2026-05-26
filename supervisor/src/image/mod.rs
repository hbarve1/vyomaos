// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! PNG image loading and in-memory cache.

use std::collections::HashMap;

/// Decoded RGBA image.
pub struct RgbaImage {
    /// Raw RGBA bytes, 4 bytes per pixel, row-major.
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Cache of decoded PNG images keyed by absolute file path.
pub struct ImageCache {
    cache: HashMap<String, Option<RgbaImage>>,
}

impl ImageCache {
    pub fn new() -> Self {
        Self { cache: HashMap::new() }
    }

    /// Return a reference to the decoded image at `path`, decoding on first access.
    /// Returns `None` if the file is absent or not a valid PNG.
    pub fn get(&mut self, path: &str) -> Option<&RgbaImage> {
        if !self.cache.contains_key(path) {
            let img = Self::load(path);
            self.cache.insert(path.to_string(), img);
        }
        self.cache[path].as_ref()
    }

    fn load(path: &str) -> Option<RgbaImage> {
        match lodepng::decode32_file(path) {
            Ok(bmp) => {
                let mut rgba = Vec::with_capacity(bmp.width * bmp.height * 4);
                for px in &bmp.buffer {
                    rgba.push(px.r);
                    rgba.push(px.g);
                    rgba.push(px.b);
                    rgba.push(px.a);
                }
                Some(RgbaImage {
                    rgba,
                    width:  bmp.width  as u32,
                    height: bmp.height as u32,
                })
            }
            Err(e) => {
                eprintln!("[image] WARNING: cannot decode {path}: {e}");
                None
            }
        }
    }
}
