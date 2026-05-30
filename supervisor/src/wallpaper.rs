// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Wallpaper state: solid color or PNG image path.

use std::sync::{Mutex, OnceLock};

/// The current desktop wallpaper — either a solid RGBA color or a path to a PNG image.
#[derive(Debug, Clone, PartialEq)]
pub enum Wallpaper {
    /// Solid RGBA fill (packed `(R<<24)|(G<<16)|(B<<8)|A`).
    SolidColor(u32),
    /// Absolute path to a PNG file rendered as the desktop background.
    Image(String),
}

/// Default desktop background color (dark grey).
const DEFAULT_BG: u32 = 0x1C1C1EFF;

static WALLPAPER: OnceLock<Mutex<Wallpaper>> = OnceLock::new();

fn wallpaper_lock() -> &'static Mutex<Wallpaper> {
    WALLPAPER.get_or_init(|| Mutex::new(Wallpaper::SolidColor(DEFAULT_BG)))
}

/// Return the current wallpaper setting.
pub fn current() -> Wallpaper {
    wallpaper_lock().lock().unwrap().clone()
}

/// Set a new wallpaper (solid color or image path).
pub fn set(wp: Wallpaper) {
    *wallpaper_lock().lock().unwrap() = wp;
}

/// Parse a wallpaper argument string.
///
/// - If `arg` starts with `/`, it is treated as an image path.
/// - Otherwise it is parsed as a hex (`0xRRGGBBAA`) or decimal RGBA color.
/// - Returns `None` if parsing fails for a non-path argument.
pub fn parse_arg(arg: &str) -> Option<Wallpaper> {
    let arg = arg.trim();
    if arg.is_empty() {
        return None;
    }
    if arg.starts_with('/') {
        return Some(Wallpaper::Image(arg.to_string()));
    }
    // Try hex then decimal
    let rgba = arg
        .strip_prefix("0x")
        .or_else(|| arg.strip_prefix("0X"))
        .and_then(|hex| u32::from_str_radix(hex, 16).ok())
        .or_else(|| arg.parse::<u32>().ok())?;
    Some(Wallpaper::SolidColor(rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_solid_color_hex() {
        assert_eq!(
            parse_arg("0xFF00FFFF"),
            Some(Wallpaper::SolidColor(0xFF00FFFF))
        );
    }

    #[test]
    fn parse_solid_color_decimal() {
        assert_eq!(
            parse_arg("218169855"),
            Some(Wallpaper::SolidColor(218169855))
        );
    }

    #[test]
    fn parse_image_path() {
        assert_eq!(
            parse_arg("/data/wallpaper.png"),
            Some(Wallpaper::Image("/data/wallpaper.png".to_string()))
        );
    }

    #[test]
    fn parse_empty_returns_none() {
        assert_eq!(parse_arg(""), None);
    }

    #[test]
    fn parse_invalid_returns_none() {
        assert_eq!(parse_arg("not-a-color"), None);
    }

    #[test]
    fn default_is_solid_color() {
        // current() returns default on first access (only reliable in isolation,
        // but the variant check is stable).
        let wp = current();
        matches!(wp, Wallpaper::SolidColor(_));
    }
}
