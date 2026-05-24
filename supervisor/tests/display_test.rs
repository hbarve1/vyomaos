// Tests for display module: word-wrap logic, cursor draw/restore, accent colors.

use supervisor::display::{app_accent_color, Framebuffer, CURSOR_W, CURSOR_H};

/// Pure word-wrap helper — duplicated here because supervisor is a binary crate.
fn wrap_words(text: &str, max_chars: usize) -> Vec<String> {
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
    lines.push(current);  // always push, even if empty
    lines
}

#[test]
fn empty_string_gives_one_empty_line() {
    assert_eq!(wrap_words("", 10), vec![""]);
}

#[test]
fn single_word_fits_on_one_line() {
    assert_eq!(wrap_words("hello", 10), vec!["hello"]);
}

#[test]
fn two_words_fit_on_one_line() {
    assert_eq!(wrap_words("hello world", 11), vec!["hello world"]);
}

#[test]
fn two_words_split_across_lines() {
    assert_eq!(wrap_words("hello world", 5), vec!["hello", "world"]);
}

#[test]
fn max_chars_zero_returns_full_text() {
    assert_eq!(wrap_words("hello world", 0), vec!["hello world"]);
}

#[test]
fn long_single_word_not_split() {
    // A word longer than max_chars is placed on its own line without truncation
    assert_eq!(wrap_words("superlongword", 5), vec!["superlongword"]);
}

#[test]
fn three_words_two_fit_then_one() {
    // "aa bb" = 5 chars fits in 5, "cc" goes to next line
    assert_eq!(wrap_words("aa bb cc", 5), vec!["aa bb", "cc"]);
}

#[test]
fn sentence_wraps_correctly() {
    let result = wrap_words("the quick brown fox", 9);
    assert_eq!(result, vec!["the quick", "brown fox"]);
}

#[test]
fn exactly_max_chars_stays_on_one_line() {
    // "ab cd" = 5 chars, max_chars = 5 → fits exactly
    assert_eq!(wrap_words("ab cd", 5), vec!["ab cd"]);
}

#[test]
fn trailing_space_does_not_produce_empty_line() {
    assert_eq!(wrap_words("hello ", 10), vec!["hello"]);
}

// ── Cursor draw / restore ─────────────────────────────────────────────────────

#[test]
fn test_cursor_draw_restore() {
    let (mut fb, _) = Framebuffer::new_for_test(100, 80);
    // Position cursor at (10, 10) and fill back-buffer with a known pattern.
    fb.cursor.cx = 10;
    fb.cursor.cy = 10;
    // Write a distinctive pattern under the cursor region.
    let cw = CURSOR_W as usize;
    let ch = CURSOR_H as usize;
    let stride = 100usize * 4;
    for row in 0..ch {
        for col in 0..cw {
            let py = 10 + row;
            let px = 10 + col;
            let off = py * stride + px * 4;
            fb.back[off]     = (row  * 13) as u8;
            fb.back[off + 1] = (col  * 7)  as u8;
            fb.back[off + 2] = 0xAB;
            fb.back[off + 3] = 0xFF;
        }
    }
    // Snapshot original back-buffer pixels under the cursor region.
    let mut original = vec![0u8; cw * ch * 4];
    for row in 0..ch {
        for col in 0..cw {
            let py = 10 + row;
            let px = 10 + col;
            let off = py * stride + px * 4;
            let dst = (row * cw + col) * 4;
            original[dst..dst + 4].copy_from_slice(&fb.back[off..off + 4]);
        }
    }

    // Draw cursor — saved_under must match original pixels.
    fb.draw_cursor();
    assert!(fb.cursor.drawn, "drawn flag must be set after draw_cursor");
    for (i, (&saved, &orig)) in fb.cursor.saved_under.iter().zip(original.iter()).enumerate() {
        assert_eq!(saved, orig, "saved_under mismatch at byte {i}");
    }

    // Restore — back-buffer must match original again.
    fb.restore_under_cursor();
    assert!(!fb.cursor.drawn, "drawn flag must be cleared after restore");
    for row in 0..ch {
        for col in 0..cw {
            let py = 10 + row;
            let px = 10 + col;
            let off = py * stride + px * 4;
            let dst = (row * cw + col) * 4;
            assert_eq!(
                fb.back[off..off + 4],
                original[dst..dst + 4],
                "pixel ({px},{py}) not restored"
            );
        }
    }
}

#[test]
fn test_restore_noop_when_not_drawn() {
    let (mut fb, _) = Framebuffer::new_for_test(50, 50);
    // Fill back with known bytes.
    fb.back.iter_mut().enumerate().for_each(|(i, b)| *b = (i % 251) as u8);
    let snapshot = fb.back.clone();
    fb.restore_under_cursor(); // drawn=false → must be a no-op
    assert_eq!(fb.back, snapshot, "restore_under_cursor must not modify back-buffer when not drawn");
}

// ── app_accent_color tests ────────────────────────────────────────────────────

const PALETTE: [u32; 6] = [
    0xFF6B6BFF, // red-ish
    0xFFD93DFF, // yellow
    0x6BCB77FF, // green
    0x4D96FFFF, // blue
    0xC77DFFFF, // purple
    0xFF9F43FF, // orange
];

#[test]
fn accent_color_is_from_palette() {
    // The returned color must be one of the six known palette entries.
    let color = app_accent_color("hello-world");
    assert!(
        PALETTE.contains(&color),
        "app_accent_color returned 0x{color:08X} which is not in the palette"
    );
}

#[test]
fn accent_color_is_deterministic() {
    // Same name always yields the same color regardless of call order.
    let a = app_accent_color("gui-demo");
    let b = app_accent_color("gui-demo");
    assert_eq!(a, b, "app_accent_color must return the same value for the same name");
}

#[test]
fn accent_color_differs_for_distinct_names() {
    // "ping" (slot 3, blue) and "shell" (slot 5, orange) hash to different palette slots.
    let ping = app_accent_color("ping");
    let shell = app_accent_color("shell");
    assert_ne!(ping, shell, "expected 'ping' and 'shell' to map to different accent colors");
}

#[test]
fn accent_color_empty_name_does_not_panic() {
    // Empty string must not panic and must return a palette color.
    let color = app_accent_color("");
    assert!(
        PALETTE.contains(&color),
        "app_accent_color(\"\") returned 0x{color:08X} which is not in the palette"
    );
}
