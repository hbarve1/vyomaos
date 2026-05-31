// Tests for ESC sequence classification logic.
// We extract the classifier into a pure function so it's testable without tty hardware.

// ── Mirror of InputAction + classify_input_sequence from main.rs ──────────────

/// Decoded action for a raw TTY input byte sequence.
#[derive(Debug, PartialEq)]
enum InputAction {
    AltTab,
    AltShiftTab,
    AltW,
    AltF4,
    AltF,
    AltQuestion,
    PassThrough,
}

/// Classify a raw input byte sequence (starting from the first byte, including ESC).
///
/// Recognised sequences:
///   `[0x1B, 0x09]`        → AltTab        (ESC + TAB)
///   `[0x1B, 0x5B, 0x5A]`  → AltShiftTab   (ESC + [ + Z  i.e. \x1b[Z)
///   `[0x1B, 0x77]`        → AltW          (ESC + 'w')
///   `[0x1B, 0x66]`        → AltF          (ESC + 'f')
///   `[0x1B, 0x3F]`        → AltQuestion   (ESC + '?')
///   anything else         → PassThrough
fn classify_input_sequence(bytes: &[u8]) -> InputAction {
    match bytes {
        [0x1B, 0x09]                    => InputAction::AltTab,
        [0x1B, 0x5B, 0x5A]             => InputAction::AltShiftTab,
        [0x1B, 0x77]                    => InputAction::AltW,
        [0x1B, 0x5B, 0x31, 0x34, 0x7E] => InputAction::AltF4,
        [0x1B, 0x66]                    => InputAction::AltF,
        [0x1B, 0x3F]                    => InputAction::AltQuestion,
        _                               => InputAction::PassThrough,
    }
}

/// Return the static help text listing all window-management keyboard shortcuts.
fn shortcut_help_text() -> &'static str {
    "Alt+Tab: next  Alt+W/F4: close  Alt+F: fullscreen  Ctrl+S: screenshot  Alt+?: help"
}

// ── Legacy ESC-pair classifier (kept for backwards-compatible arrow-key tests) ─

/// Classify a raw ESC sequence (the 2 bytes after 0x1B).
/// Returns Some(msg) to forward, or None to discard.
fn classify_esc(esc: [u8; 2]) -> Option<String> {
    match esc {
        [0x5B, 0x41] => Some("\x1b[A".to_string()), // ↑ up arrow
        [0x5B, 0x42] => Some("\x1b[B".to_string()), // ↓ down arrow
        _            => None,                         // discard all other ESC seqs
    }
}

// ── focus cycle helpers (mirrored from main.rs for unit testing) ──────────────

fn cycle_focus_forward(names: &[String], current: Option<&str>) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(idx) => Some(names[(idx + 1) % names.len()].clone()),
        None      => Some(names[0].clone()),
    }
}

fn cycle_focus_backward(names: &[String], current: Option<&str>) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    match current.and_then(|c| names.iter().position(|n| n == c)) {
        Some(0)   => Some(names[names.len() - 1].clone()),
        Some(idx) => Some(names[idx - 1].clone()),
        None      => Some(names[names.len() - 1].clone()),
    }
}

// ── Legacy arrow-key tests ────────────────────────────────────────────────────

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

// ── New: classify_input_sequence tests ───────────────────────────────────────

#[test]
fn test_alt_key_sequence_classification() {
    // Alt+Tab: ESC + TAB (0x1B 0x09)
    assert_eq!(classify_input_sequence(&[0x1B, 0x09]), InputAction::AltTab);

    // Alt+W: ESC + 'w' (0x1B 0x77)
    assert_eq!(classify_input_sequence(&[0x1B, 0x77]), InputAction::AltW);

    // Alt+F: ESC + 'f' (0x1B 0x66)
    assert_eq!(classify_input_sequence(&[0x1B, 0x66]), InputAction::AltF);

    // Plain 'A' → PassThrough
    assert_eq!(classify_input_sequence(&[0x41]), InputAction::PassThrough);

    // Alt+Shift+Tab: ESC + '[' + 'Z' (0x1B 0x5B 0x5A)
    assert_eq!(classify_input_sequence(&[0x1B, 0x5B, 0x5A]), InputAction::AltShiftTab);
}

#[test]
fn test_classify_passthrough_variants() {
    // Up/down arrows are handled by the caller before classify_input_sequence
    // and return PassThrough from the full-sequence classifier (no special casing).
    assert_eq!(classify_input_sequence(&[0x1B, 0x5B, 0x41]), InputAction::PassThrough); // up arrow
    assert_eq!(classify_input_sequence(&[0x1B, 0x5B, 0x42]), InputAction::PassThrough); // down arrow
    assert_eq!(classify_input_sequence(&[0x1B, 0x5B, 0x43]), InputAction::PassThrough); // right arrow
    assert_eq!(classify_input_sequence(&[0x1B, 0x5B, 0x44]), InputAction::PassThrough); // left arrow
    assert_eq!(classify_input_sequence(&[0x0D]),              InputAction::PassThrough); // Enter
    assert_eq!(classify_input_sequence(&[0x61]),              InputAction::PassThrough); // 'a'
    assert_eq!(classify_input_sequence(&[]),                  InputAction::PassThrough); // empty
}

#[test]
fn test_cycle_focus_forward_wraps() {
    let names: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    // Forward from last should wrap to first
    assert_eq!(cycle_focus_forward(&names, Some("c")), Some("a".to_string()));
    // Forward from middle
    assert_eq!(cycle_focus_forward(&names, Some("a")), Some("b".to_string()));
    assert_eq!(cycle_focus_forward(&names, Some("b")), Some("c".to_string()));
    // No current → first
    assert_eq!(cycle_focus_forward(&names, None), Some("a".to_string()));
    // Unknown current → first
    assert_eq!(cycle_focus_forward(&names, Some("z")), Some("a".to_string()));
    // Empty list → None
    assert_eq!(cycle_focus_forward(&[], None), None);
}

#[test]
fn test_cycle_focus_backward_wraps() {
    let names: Vec<String> = vec!["a".into(), "b".into(), "c".into()];
    // Backward from first should wrap to last
    assert_eq!(cycle_focus_backward(&names, Some("a")), Some("c".to_string()));
    // Backward from middle
    assert_eq!(cycle_focus_backward(&names, Some("c")), Some("b".to_string()));
    assert_eq!(cycle_focus_backward(&names, Some("b")), Some("a".to_string()));
    // No current → last
    assert_eq!(cycle_focus_backward(&names, None), Some("c".to_string()));
    // Unknown current → last
    assert_eq!(cycle_focus_backward(&names, Some("z")), Some("c".to_string()));
    // Empty list → None
    assert_eq!(cycle_focus_backward(&[], None), None);
}

#[test]
fn test_cycle_single_app() {
    let names: Vec<String> = vec!["only".into()];
    // Single app: forward and backward both stay on the same app
    assert_eq!(cycle_focus_forward(&names, Some("only")), Some("only".to_string()));
    assert_eq!(cycle_focus_backward(&names, Some("only")), Some("only".to_string()));
}

// ── Alt+? shortcut overlay tests ─────────────────────────────────────────────

#[test]
fn test_alt_question_classified() {
    // Alt+? (ESC + '?', bytes 0x1B 0x3F) must decode to AltQuestion.
    assert_eq!(classify_input_sequence(&[0x1B, 0x3F]), InputAction::AltQuestion);
}

#[test]
fn test_alt_tab_still_classified() {
    // Regression: existing Alt+Tab mapping must remain unchanged.
    assert_eq!(classify_input_sequence(&[0x1B, 0x09]), InputAction::AltTab);
}

#[test]
fn test_shortcut_help_text_content() {
    let text = shortcut_help_text();
    assert!(text.contains("Alt+Tab"), "help text must mention Alt+Tab");
    assert!(text.contains("Alt+W"),   "help text must mention Alt+W");
    assert!(text.contains("Alt+F"),   "help text must mention Alt+F");
    assert!(text.contains("Alt+?"),   "help text must mention Alt+?");
    assert!(text.contains("fullscreen"), "help text must mention fullscreen");
    assert!(text.contains("F4"),     "help text must mention F4");
}

// ── P82: Alt+F4 tests ──────────────────────────────────────────────────────

#[test]
fn test_alt_f4_classified() {
    // Alt+F4: ESC + CSI F4 = \x1b[14~ → bytes [0x1B, 0x5B, 0x31, 0x34, 0x7E]
    assert_eq!(
        classify_input_sequence(&[0x1B, 0x5B, 0x31, 0x34, 0x7E]),
        InputAction::AltF4,
    );
}

#[test]
fn test_alt_f4_does_not_match_other_function_keys() {
    // F1 = \x1b[11~
    assert_eq!(
        classify_input_sequence(&[0x1B, 0x5B, 0x31, 0x31, 0x7E]),
        InputAction::PassThrough,
    );
    // F5 = \x1b[15~
    assert_eq!(
        classify_input_sequence(&[0x1B, 0x5B, 0x31, 0x35, 0x7E]),
        InputAction::PassThrough,
    );
}

#[test]
fn test_alt_f_still_classified_as_fullscreen() {
    // Alt+F: ESC + 'f' (0x1B 0x66) → AltF (fullscreen toggle)
    assert_eq!(classify_input_sequence(&[0x1B, 0x66]), InputAction::AltF);
}

// ── P82: Tab focus within menus ─────────────────────────────────────────────

/// Mirror of dropdown key handler to test Tab navigation.
fn handle_dropdown_key(key: &str, selected: usize, n_items: usize) -> usize {
    match key {
        "\x1b[B" | "j" | "\t" => { if selected + 1 < n_items { selected + 1 } else { selected } }
        "\x1b[A" | "k" => { if selected > 0 { selected - 1 } else { selected } }
        _ => selected,
    }
}

#[test]
fn test_tab_advances_menu_selection() {
    // Tab should move to next item just like ArrowDown
    assert_eq!(handle_dropdown_key("\t", 0, 3), 1);
    assert_eq!(handle_dropdown_key("\t", 1, 3), 2);
    // At last item, Tab should not advance past end
    assert_eq!(handle_dropdown_key("\t", 2, 3), 2);
}

#[test]
fn test_tab_and_arrow_down_equivalent() {
    for sel in 0..3 {
        assert_eq!(
            handle_dropdown_key("\t", sel, 4),
            handle_dropdown_key("\x1b[B", sel, 4),
            "Tab and ArrowDown should produce the same result at position {sel}",
        );
    }
}
