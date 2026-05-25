// Copyright (c) 2025-2026 Himank Barve. Licensed under the VyomaOS Community License.
// See LICENSE (community) and LICENSE-COMMERCIAL (commercial) at the repository root.

/// Alpha-blend `fg` over `bg` using the given alpha value `a` (0 = fully transparent, 255 = fully opaque).
///
/// Both `fg` and `bg` are packed RGBA `u32` values (`0xRRGGBBAA`).  The output
/// alpha byte is always `0xFF` (the result is fully opaque).  This is a pure
/// function with no global state.
#[allow(dead_code)]
pub fn blend_alpha(fg: u32, bg: u32, a: u8) -> u32 {
    let af = a as u32;
    let blend = |f: u32, b: u32| ((f * af + b * (255 - af)) / 255) & 0xFF;
    let r = blend((fg >> 24) & 0xFF, (bg >> 24) & 0xFF);
    let g = blend((fg >> 16) & 0xFF, (bg >> 16) & 0xFF);
    let b = blend((fg >>  8) & 0xFF, (bg >>  8) & 0xFF);
    (r << 24) | (g << 16) | (b << 8) | 0xFF
}

/// Return the RGBA title-bar background colour for a window.
///
/// - `focused = true`  → bright blue accent  (`0x388BFDFF`)
/// - `focused = false` → dark grey           (`0x30363DFF`)
///
/// Pure function: no I/O, no side-effects.
#[allow(dead_code)]
pub fn titlebar_color(focused: bool) -> u32 {
    if focused { 0x388BFDFF } else { 0x30363DFF }
}

/// Return the 2px focus-border color for a window.
///
/// Focused windows get a bright blue accent (`0x388BFDFF`).
/// Unfocused windows get a dim gray (`0x30363DFF`).
/// This is a pure function — no I/O, no side effects.
pub fn border_color(focused: bool) -> u32 {
    if focused { 0x388BFDFF } else { 0x30363DFF }
}

/// Return a deterministic accent color for an app based on its name.
///
/// The color is derived from a djb2 hash of the app name, then mapped to one
/// of six palette entries.  The function is pure — no randomness, no global state.
pub fn app_accent_color(name: &str) -> u32 {
    const PALETTE: [u32; 6] = [
        0xFF6B6BFF, // red-ish
        0xFFD93DFF, // yellow
        0x6BCB77FF, // green
        0x4D96FFFF, // blue
        0xC77DFFFF, // purple
        0xFF9F43FF, // orange
    ];
    let hash = name
        .bytes()
        .fold(5381u32, |h, b| h.wrapping_mul(33).wrapping_add(b as u32));
    PALETTE[(hash % 6) as usize]
}

/// Compute a display string for flush rate from raw counters.
///
/// `flushes` is the number of `VYOMA_DRAW:flush` calls recorded in
/// `elapsed_ms` milliseconds.  Returns `"0fps"` when `elapsed_ms == 0`
/// to avoid a divide-by-zero.
///
/// This function is **pure**: it has no side-effects and does not
/// touch any global state.
pub fn format_fps(flushes: u64, elapsed_ms: u64) -> String {
    if elapsed_ms == 0 {
        return "0fps".to_string();
    }
    let fps = (flushes * 1000) / elapsed_ms;
    format!("{fps}fps")
}

/// Word-wrap `text` so each line is at most `max_chars` wide.
/// Long single words are placed on their own line without truncation.
/// If `max_chars` is 0, returns the full text as a single line.
/// SYNC: algorithm duplicated in supervisor/tests/display_test.rs — keep in lockstep.
pub fn wrap_words(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![text.to_string()];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        if current.is_empty() {
            current.push_str(word);
        } else if current.len() + 1 + word.len() <= max_chars {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    lines.push(current);  // always push, even if empty — ensures empty input returns vec![""]
    lines
}
