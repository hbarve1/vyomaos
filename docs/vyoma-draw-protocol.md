# Contract: VYOMA_DRAW Protocol

**Version**: 2.0 | **Date**: 2026-05-30 | **Stability**: Stable

## Overview

The VYOMA_DRAW protocol is a line-oriented text protocol that display-capable WASM apps write to `stdout`. The VyomaOS supervisor intercepts lines matching the `VYOMA_DRAW:` prefix and dispatches them to the framebuffer driver. All other stdout lines are either routed as IPC messages or logged.

**Capability required**: `display = true` in `vyoma.toml`. Lines emitted by apps without the `display` capability are ignored (not an error).

---

## Commands

### `fill_rect`

Fill a rectangular region with a solid color.

**Format**: `VYOMA_DRAW:fill_rect:<x>,<y>,<w>,<h>,<rgba>`

| Parameter | Type | Description |
|---|---|---|
| `x` | `u32` | Left edge in pixels, relative to app window origin (0 = left) |
| `y` | `u32` | Top edge in pixels, relative to app window origin (0 = top) |
| `w` | `u32` | Width in pixels |
| `h` | `u32` | Height in pixels |
| `rgba` | `u32` | Packed color: `(R << 24) | (G << 16) | (B << 8) | A` printed as decimal |

**Behavior**:
- If `w == 0` or `h == 0`: no-op, no error
- If rect extends beyond the window boundary: clipped to window bounds silently
- If framebuffer is absent (headless boot): silent no-op
- Alpha channel (`A`) is accepted but ignored; all rendering is opaque

**Example** (Rust):
```rust
// Draw a 100×50 red rectangle at (10, 20)
let rgba: u32 = (255 << 24) | (0 << 16) | (0 << 8) | 0xFF; // 0xFF0000FF
println!("VYOMA_DRAW:fill_rect:10,20,100,50,{}", rgba);
```

---

### `draw_text`

Render a string of ASCII text using the built-in bitmap font.

**Format (current)**: `VYOMA_DRAW:draw_text:<x>,<y>,<rgba>,<size>,<text>`

**Format (legacy, still supported)**: `VYOMA_DRAW:draw_text:<x>,<y>,<rgba>,<text>`

| Parameter | Type | Description |
|---|---|---|
| `x` | `u32` | Left edge of first character, relative to app window origin |
| `y` | `u32` | Top edge of first character baseline, relative to app window origin |
| `rgba` | `u32` | Text foreground color (same encoding as `fill_rect`) |
| `size` | `char` | Font size: `s` = Small (4×8), `m` = Medium (8×16), `l` = Large (16×32). Omit for legacy Medium. |
| `text` | `String` | Text to render; ASCII printable range only; no embedded commas in 5-field format |

**Glyph dimensions**:

| Size | Glyph W | Glyph H | Use case |
|---|---|---|---|
| `s` | 4 px | 8 px | Dense HUD overlays |
| `m` | 8 px | 16 px | Standard UI text |
| `l` | 16 px | 32 px | Large headings |

**Behavior**:
- Characters outside ASCII printable range (0x20–0x7E) render as a blank glyph
- Text that would extend beyond the right window edge is truncated (no wrapping)
- Background pixels are not cleared; render a `fill_rect` first for a background
- Legacy 4-field format is equivalent to `size=m`

**Example** (Rust):
```rust
// Render "Hello" in white, medium font, at (8, 4)
let white: u32 = 0xFFFFFFFF;
println!("VYOMA_DRAW:draw_text:8,4,{},m,Hello", white);

// Legacy format (equivalent):
println!("VYOMA_DRAW:draw_text:8,4,{},Hello", white);
```

---

### `flush` / `present`

Blit the back-buffer to the visible framebuffer, making all pending draw calls visible.

**Format**: `VYOMA_DRAW:flush`  or  `VYOMA_DRAW:present` (alias)

**Behavior**:
- All `fill_rect` and `draw_text` calls accumulate in a back-buffer; nothing is visible until `flush`
- If called without any pending draw calls: harmless no-op
- If called in headless mode (no framebuffer): silent no-op
- Window decoration chrome (title bar, close button) is painted by the supervisor on top of app content before each blit — apps should not draw in the top 20px of their window region

**Recommended frame pattern**:
```rust
// 1. Clear background
println!("VYOMA_DRAW:fill_rect:0,0,960,720,{}", BG_COLOR);
// 2. Draw content
println!("VYOMA_DRAW:draw_text:8,4,{},m,Status: OK", WHITE);
// 3. Commit frame — do this exactly once per logical frame
println!("VYOMA_DRAW:flush");
let _ = io::stdout().flush();
```

---

### `draw_text_wrap`

Render text that automatically wraps at a maximum width.

**Format**: `VYOMA_DRAW:draw_text_wrap:<x>,<y>,<max_w>,<rgba>,<size>,<text>`

| Parameter | Type | Description |
|---|---|---|
| `x`, `y`  | `u32` | Top-left position of text, relative to app window origin |
| `max_w`   | `u32` | Maximum width in pixels before wrapping to next line |
| `rgba`    | `u32` | Text foreground color (same encoding as `fill_rect`) |
| `size`    | `char` | Font size: `s`, `m`, or `l` (same as `draw_text`) |
| `text`    | `String` | Text to render; wrapped at word boundaries within `max_w` |

**Example** (Rust):
```rust
let white: u32 = 0xFFFFFFFF;
println!("VYOMA_DRAW:draw_text_wrap:8,4,400,{},m,Long text wraps at 400px", white);
```

---

## v2 Commands

> **Note**: All v1 commands above remain fully supported and unchanged. v2 adds the following new commands. Apps can detect v2 supervisor support by reading `VYOMA_SYSTEM:draw_version:2` from stdin at startup.

### `draw_glyph`

Render text using scalable font rendering (replaces bitmap fonts for high-fidelity UI).

**Format**: `VYOMA_DRAW:draw_glyph:<x>,<y>,<rgba>,<pt>,<weight>,<text>`

| Parameter | Type | Description |
|---|---|---|
| `x`, `y` | `u32` | Top-left baseline origin in pixels, relative to app window origin |
| `rgba` | `u32` | Text color (same packed encoding as `fill_rect`) |
| `pt` | `u32` | Font size in points (8--96) |
| `weight` | `str` | Font weight: `regular`, `bold`, or `mono` |
| `text` | `str` | UTF-8 text to render; no embedded commas (use `draw_glyph_escaped` for commas) |

**Example** (Rust):
```rust
let white: u32 = 0xFFFFFFFF;
println!("VYOMA_DRAW:draw_glyph:40,60,{},16,bold,System Preferences", white);
```

---

### `draw_image`

Blit a PNG image to the framebuffer, scaled to the specified dimensions.

**Format**: `VYOMA_DRAW:draw_image:<x>,<y>,<w>,<h>,<path>`

| Parameter | Type | Description |
|---|---|---|
| `x`, `y` | `u32` | Top-left position on screen, relative to app window origin |
| `w`, `h` | `u32` | Target width and height in pixels (image is scaled to fit) |
| `path` | `str` | Absolute path to a PNG file in the initramfs |

**Behavior**:
- Supervisor decodes PNG on first use and caches the decoded RGBA buffer
- Image is scaled with nearest-neighbor (speed) or bilinear (quality) based on supervisor config
- If the path does not exist or is not a valid PNG: log WARN to stderr, skip command

**Example** (Rust):
```rust
println!("VYOMA_DRAW:draw_image:8,8,48,48,/apps/settings/icon.png");
```

---

### `fill_rect_r`

Fill a rounded rectangle with alpha compositing support.

**Format**: `VYOMA_DRAW:fill_rect_r:<x>,<y>,<w>,<h>,<rgba>,<radius>`

| Parameter | Type | Description |
|---|---|---|
| `x`, `y` | `u32` | Top-left corner in pixels, relative to app window origin |
| `w`, `h` | `u32` | Width and height in pixels |
| `rgba` | `u32` | Fill color with alpha channel (alpha is composited using Porter-Duff "over") |
| `radius` | `u32` | Corner radius in pixels (0 = square, equivalent to `fill_rect`) |

**Behavior**:
- Uses distance-field coverage mask for anti-aliased corners
- Alpha channel is fully supported (unlike v1 `fill_rect` where alpha is ignored)
- If `radius` exceeds half of `w` or `h`, it is clamped

**Example** (Rust):
```rust
// Purple at ~60% alpha with 12px rounded corners
let purple_alpha: u32 = 0x9900FFFF; // 2566914303 in decimal
println!("VYOMA_DRAW:fill_rect_r:20,20,400,300,{},12", purple_alpha);
```

---

### `set_layer_alpha`

Set the alpha multiplier for the app's entire compositing layer, enabling fade-in/fade-out animations.

**Format**: `VYOMA_DRAW:set_layer_alpha:<value>`

| Parameter | Type | Description |
|---|---|---|
| `value` | `u8` | Alpha multiplier for the entire app layer (0 = fully transparent, 255 = fully opaque) |

**Behavior**:
- Affects all content rendered by this app during compositing
- Can be driven by the supervisor for window animations, or set directly by apps
- Default is 255 (fully opaque) if never set

**Example** (Rust):
```rust
// Fade to 50% opacity
println!("VYOMA_DRAW:set_layer_alpha:128");
```

---

## Coordinate System

- Origin `(0, 0)` is the **top-left corner of the app's window region** (not the physical screen)
- X increases to the right; Y increases downward
- Apps with a `[window]` section in `vyoma.toml` render into that region; clipping is automatic
- Apps without a `[window]` section render to the full framebuffer (960×720 default)

## Color Encoding

Colors are packed `u32` values: `(R << 24) | (G << 16) | (B << 8) | A`

```rust
// Convenience function
fn rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    ((r as u32) << 24) | ((g as u32) << 16) | ((b as u32) << 8) | (a as u32)
}

// Common colors
const BLACK:  u32 = 0x000000FF;
const WHITE:  u32 = 0xFFFFFFFF;
const RED:    u32 = 0xFF0000FF;
const GREEN:  u32 = 0x00FF00FF;
const BLUE:   u32 = 0x0000FFFF;
```

The value is printed as a **decimal integer** (Rust's default `{}` formatter for `u32`).

## Error Handling

| Condition | Supervisor behavior |
|---|---|
| Malformed `fill_rect` args (wrong count / non-numeric) | Log WARN to stderr; skip command |
| Malformed `draw_text` args | Log WARN to stderr; skip command |
| Malformed `draw_glyph` args (wrong count / invalid weight) | Log WARN to stderr; skip command |
| Malformed `draw_image` args (missing path / invalid PNG) | Log WARN to stderr; skip command |
| Malformed `fill_rect_r` args (wrong count / non-numeric) | Log WARN to stderr; skip command |
| Malformed `set_layer_alpha` value (out of range / non-numeric) | Log WARN to stderr; skip command |
| Unknown command (e.g., `VYOMA_DRAW:blit_surface:...`) | Log WARN to stderr; skip command |
| App lacks `display` capability | Entire `VYOMA_DRAW:` line silently ignored |
| No framebuffer (`/dev/fb0` absent) | All draw commands are silent no-ops |

## Protocol Versioning

This document describes VYOMA_DRAW **v2.0**. Apps can detect the protocol version at startup: if the supervisor supports v2, it writes `VYOMA_SYSTEM:draw_version:2` to the app's stdin before normal operation begins. Apps that do not read this line can safely ignore it and continue using v1 commands only.

Future additions will be backward-compatible (new command prefixes; existing commands unchanged). Breaking changes will increment the major version and require a supervisor update.

## Version History

| Version | Date | Changes |
|---|---|---|
| **v1.0** | 2026-05-24 | Initial protocol: `fill_rect`, `draw_text`, `draw_text_wrap`, `flush`/`present` |
| **v2.0** | 2026-05-30 | Added `draw_glyph` (scalable fonts), `draw_image` (PNG blit), `fill_rect_r` (rounded rects with alpha), `set_layer_alpha` (layer compositing). All v1 commands remain fully supported. |

## Supervisor-to-App Protocol Extensions (v2)

At app startup (if `display = true`), the supervisor writes capability broadcasts to the app's stdin:

```
VYOMA_SYSTEM:draw_version:2
VYOMA_SYSTEM:screen:1440,900
VYOMA_SYSTEM:display_profile:desktop
```

| Field | Description |
|---|---|
| `draw_version` | Protocol version supported by the supervisor (integer) |
| `screen` | Screen dimensions in pixels (`<width>,<height>`) |
| `display_profile` | Active form factor: `desktop`, `mobile`, `iot-edge`, etc. Apps can adapt layout accordingly. |
