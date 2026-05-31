// draw_cmd parser tests — Week 1, Task 1b
//
// Tests for supervisor::parse_color which is the platform-independent
// color parser extracted from draw_cmd.rs.

#[test]
fn test_parse_color_decimal() {
    // Maximum u32 value: 4294967295 == 0xFFFFFFFF
    assert_eq!(supervisor::parse_color("4294967295"), Some(0xFFFF_FFFF));
}

#[test]
fn test_parse_color_hex_lower() {
    assert_eq!(supervisor::parse_color("0x0d1117ff"), Some(0x0D11_17FF));
}

#[test]
fn test_parse_color_hex_upper() {
    assert_eq!(supervisor::parse_color("0X0D1117FF"), Some(0x0D11_17FF));
}

#[test]
fn test_parse_color_zero() {
    assert_eq!(supervisor::parse_color("0"), Some(0));
}

#[test]
fn test_parse_color_invalid() {
    // "abc" is not a valid decimal and has no 0x prefix
    assert_eq!(supervisor::parse_color("abc"), None);
}

#[test]
fn test_parse_color_empty() {
    assert_eq!(supervisor::parse_color(""), None);
}

#[test]
fn test_parse_color_whitespace() {
    // Leading/trailing whitespace should be trimmed; "0xFF" → 255
    assert_eq!(supervisor::parse_color(" 0xFF "), Some(0xFF));
}

#[test]
fn test_parse_color_overflow() {
    // 99999999999 exceeds u32::MAX (4294967295)
    assert_eq!(supervisor::parse_color("99999999999"), None);
}

#[test]
fn test_parse_color_hex_overflow() {
    // 0x1FFFFFFFF exceeds u32::MAX
    assert_eq!(supervisor::parse_color("0x1FFFFFFFF"), None);
}

#[test]
fn test_parse_color_common_rgba_values() {
    // Pure red: 0xFF0000FF
    assert_eq!(supervisor::parse_color("0xFF0000FF"), Some(0xFF00_00FF));
    // Transparent black
    assert_eq!(supervisor::parse_color("0x00000000"), Some(0));
    // Catppuccin base: 0x1E1E2EFF
    assert_eq!(supervisor::parse_color("0x1E1E2EFF"), Some(0x1E1E_2EFF));
}

#[test]
fn test_parse_color_decimal_midrange() {
    // 218169855 == 0x0D0117FF
    assert_eq!(supervisor::parse_color("218169855"), Some(218_169_855));
}

#[test]
fn test_parse_color_whitespace_only() {
    assert_eq!(supervisor::parse_color("   "), None);
}

#[test]
fn test_parse_color_negative() {
    // Negative numbers are not valid u32
    assert_eq!(supervisor::parse_color("-1"), None);
}

#[test]
fn test_parse_color_hex_mixed_case() {
    assert_eq!(supervisor::parse_color("0xaAbBcCdD"), Some(0xAABB_CCDD));
}
