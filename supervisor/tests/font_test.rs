// Tests for FontSize glyph dimensions and pixel-double rendering.

#[derive(Clone, Copy, PartialEq, Debug)]
enum FontSize { Small, Medium, Large }

fn glyph_dims(size: FontSize) -> (u32, u32) {
    match size {
        FontSize::Small  => (8, 8),
        FontSize::Medium => (8, 16),
        FontSize::Large  => (16, 32),
    }
}

fn parse_size(s: &str) -> Option<FontSize> {
    match s {
        "s" => Some(FontSize::Small),
        "m" => Some(FontSize::Medium),
        "l" => Some(FontSize::Large),
        _   => None,
    }
}

/// Given a Medium (8×16) glyph byte at `row`, return the two Large (16×32) rows.
/// Each bit in the source byte becomes 2 bits side-by-side; the row is repeated twice.
fn pixel_double_row(byte: u8) -> u16 {
    let mut out: u16 = 0;
    for bit in 0..8u8 {
        if byte & (0x80 >> bit) != 0 {
            let pos = 14 - (bit * 2); // bit 0 → positions 14,15; bit 7 → positions 0,1
            out |= 3 << pos;
        }
    }
    out
}

#[test]
fn small_dims() {
    assert_eq!(glyph_dims(FontSize::Small), (8, 8));
}

#[test]
fn medium_dims() {
    assert_eq!(glyph_dims(FontSize::Medium), (8, 16));
}

#[test]
fn large_dims() {
    assert_eq!(glyph_dims(FontSize::Large), (16, 32));
}

#[test]
fn parse_size_roundtrip() {
    assert_eq!(parse_size("s"), Some(FontSize::Small));
    assert_eq!(parse_size("m"), Some(FontSize::Medium));
    assert_eq!(parse_size("l"), Some(FontSize::Large));
    assert_eq!(parse_size("x"), None);
    assert_eq!(parse_size(""),  None);
}

#[test]
fn pixel_double_full_byte() {
    // 0xFF = all 8 bits set → all 16 bits set
    assert_eq!(pixel_double_row(0xFF), 0xFFFF);
}

#[test]
fn pixel_double_empty_byte() {
    assert_eq!(pixel_double_row(0x00), 0x0000);
}

#[test]
fn pixel_double_single_bit() {
    // MSB (bit 7 of byte = 0x80) → bits 14-15 of output
    let result = pixel_double_row(0x80);
    assert_eq!(result & 0xC000, 0xC000); // top 2 bits set
    assert_eq!(result & 0x3FFF, 0x0000); // rest clear
}
