---
title: Display Protocol
description: Reference for the VYOMA_DRAW display protocol v2.
order: 2
---

# VYOMA_DRAW Display Protocol

Apps with `display = true` in their manifest can draw to the framebuffer by writing commands to stdout. The supervisor parses these commands and renders them to the compositor.

## Color format

Colors are packed `u32` values: `(R << 24) | (G << 16) | (B << 8) | A`, printed as decimal.

```rust
const WHITE: u32 = 0xFFFFFFFF;  // 4294967295
const BLACK: u32 = 0x000000FF;  // 255
const RED:   u32 = 0xFF0000FF;  // 4278190335
const BG:    u32 = 0x1E1E2EFF;  // Catppuccin-like dark background
```

## Core commands (v1)

### fill_rect

Fill a rectangle with a solid color.

```
VYOMA_DRAW:fill_rect:<x>,<y>,<w>,<h>,<rgba>
```

### draw_text

Draw text at a position with a given size.

```
VYOMA_DRAW:draw_text:<x>,<y>,<rgba>,<size>,<text>
```

Sizes: `s` = 4x8, `m` = 8x16, `l` = 16x32

### draw_text_wrap

Draw text with word wrapping at a maximum width.

```
VYOMA_DRAW:draw_text_wrap:<x>,<y>,<max_w>,<rgba>,<size>,<text>
```

### flush

Commit the current frame to the display. Nothing appears on screen until flush is called.

```
VYOMA_DRAW:flush
```

## Extended commands (v2)

### fill_rect_r

Draw a rounded rectangle.

```
VYOMA_DRAW:fill_rect_r:<x>,<y>,<w>,<h>,<rgba>,<radius>
```

### draw_glyph

Draw a single Unicode glyph by codepoint.

```
VYOMA_DRAW:draw_glyph:<x>,<y>,<rgba>,<size>,<codepoint>
```

### draw_image

Draw an inline image (base64-encoded).

```
VYOMA_DRAW:draw_image:<x>,<y>,<w>,<h>,<format>,<base64data>
```

Format is `png` or `raw`.

## Example: drawing a simple UI

```rust
fn main() {
    let bg: u32 = 0x1E1E2EFF;
    let white: u32 = 0xFFFFFFFF;
    let accent: u32 = 0x89B4FAFF;

    // Clear background
    println!("VYOMA_DRAW:fill_rect:0,0,480,360,{bg}");

    // Title bar
    println!("VYOMA_DRAW:fill_rect:0,0,480,32,{accent}");
    println!("VYOMA_DRAW:draw_text:8,8,{bg},m,My App");

    // Body text
    println!("VYOMA_DRAW:draw_text:16,48,{white},m,Hello, VyomaOS!");

    // Rounded button
    println!("VYOMA_DRAW:fill_rect_r:16,80,120,32,{accent},8");
    println!("VYOMA_DRAW:draw_text:40,88,{bg},m,Click me");

    // Commit frame
    println!("VYOMA_DRAW:flush");
}
```

## Notes

- All coordinates are relative to the app's window, not the screen
- The compositor handles window positioning, decorations, and z-ordering
- Apps should call `flush` after completing a full frame
- Drawing without flush queues commands but displays nothing
