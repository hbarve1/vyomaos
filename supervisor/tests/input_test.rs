// Tests for ESC sequence classification logic.
// We extract the classifier into a pure function so it's testable without tty hardware.

/// Classify a raw ESC sequence (the 2 bytes after 0x1B).
/// Returns Some(msg) to forward, or None to discard.
fn classify_esc(esc: [u8; 2]) -> Option<String> {
    match esc {
        [0x5B, 0x41] => Some("\x1b[A".to_string()), // ↑ up arrow
        [0x5B, 0x42] => Some("\x1b[B".to_string()), // ↓ down arrow
        _            => None,                         // discard all other ESC seqs
    }
}

#[test]
fn up_arrow_forwards() {
    assert_eq!(classify_esc([0x5B, 0x41]), Some("\x1b[A".to_string()));
}

#[test]
fn down_arrow_forwards() {
    assert_eq!(classify_esc([0x5B, 0x42]), Some("\x1b[B".to_string()));
}

#[test]
fn other_esc_discarded() {
    assert_eq!(classify_esc([0x5B, 0x43]), None); // right arrow
    assert_eq!(classify_esc([0x5B, 0x44]), None); // left arrow
    assert_eq!(classify_esc([0x4F, 0x48]), None); // Home (xterm)
    assert_eq!(classify_esc([0x00, 0x00]), None); // garbage
}
