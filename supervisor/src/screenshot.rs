// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

//! Screenshot capture — encodes the framebuffer back-buffer as PNG and saves to
//! `/data/screenshots/screenshot_<timestamp>.png`.

use crate::BOOT_INSTANT;

/// Directory where screenshots are saved.
const SCREENSHOT_DIR: &str = "/data/screenshots";

/// Generate a screenshot filename using seconds since boot as the timestamp.
pub fn generate_filename() -> String {
    let secs = BOOT_INSTANT
        .get()
        .map(|i| i.elapsed().as_secs())
        .unwrap_or(0);
    format!("{SCREENSHOT_DIR}/screenshot_{secs}.png")
}

/// Capture the current framebuffer back-buffer as a PNG file.
///
/// The back-buffer stores pixels in BGRA byte order (little-endian on x86).
/// lodepng expects RGBA, so we swizzle B and R channels during conversion.
///
/// Creates the screenshot directory if it does not exist.
/// Returns the file path on success.
#[cfg(target_os = "linux")]
pub fn capture_screenshot(
    fb: &crate::display::Framebuffer,
) -> Result<String, String> {
    let path = generate_filename();

    // Ensure the screenshot directory exists.
    std::fs::create_dir_all(SCREENSHOT_DIR)
        .map_err(|e| format!("mkdir {SCREENSHOT_DIR}: {e}"))?;

    // Convert BGRA back-buffer to RGBA for lodepng.
    let pixel_count = (fb.width * fb.height) as usize;
    let mut rgba = Vec::with_capacity(pixel_count * 4);
    for row in 0..fb.height {
        for col in 0..fb.width {
            let off = (row * fb.stride + col * 4) as usize;
            if off + 4 <= fb.back.len() {
                let b = fb.back[off];
                let g = fb.back[off + 1];
                let r = fb.back[off + 2];
                let a = fb.back[off + 3];
                rgba.push(r);
                rgba.push(g);
                rgba.push(b);
                rgba.push(a);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 255]);
            }
        }
    }

    lodepng::encode32_file(
        &path,
        &rgba,
        fb.width as usize,
        fb.height as usize,
    )
    .map_err(|e| format!("png encode: {e}"))?;

    Ok(path)
}

// ── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_contains_dir_and_extension() {
        // BOOT_INSTANT may or may not be initialised in tests; either way the
        // filename must start with the screenshot dir and end with .png.
        let name = generate_filename();
        assert!(
            name.starts_with(SCREENSHOT_DIR),
            "expected dir prefix, got {name}"
        );
        assert!(name.ends_with(".png"), "expected .png suffix, got {name}");
        // The stem must match `screenshot_<digits>`
        let stem = name
            .strip_prefix(&format!("{SCREENSHOT_DIR}/"))
            .unwrap()
            .strip_suffix(".png")
            .unwrap();
        assert!(
            stem.starts_with("screenshot_"),
            "unexpected stem: {stem}"
        );
        let digits = stem.strip_prefix("screenshot_").unwrap();
        assert!(
            digits.chars().all(|c| c.is_ascii_digit()),
            "timestamp part is not all digits: {digits}"
        );
    }
}
