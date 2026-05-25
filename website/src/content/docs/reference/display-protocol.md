---
title: Display Protocol
description: The VYOMA_DRAW line-oriented framebuffer protocol.
---

Apps render to the screen by writing `VYOMA_DRAW:` commands to stdout. The supervisor intercepts these commands and renders them to the DRM/virtio-gpu framebuffer.

## Protocol Format

Each command is a single line written to stdout:

```
VYOMA_DRAW:<command>:<parameters>
```

Commands are processed in order. Nothing appears on screen until `flush` is called.

## Commands

### `fill_rect`

Fill a rectangle with a solid color.

```
VYOMA_DRAW:fill_rect:<x>,<y>,<width>,<height>,<rgba>
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `x` | integer | Left edge (pixels) |
| `y` | integer | Top edge (pixels) |
| `width` | integer | Width (pixels) |
| `height` | integer | Height (pixels) |
| `rgba` | u32 | Color as `(R<<24)\|(G<<16)\|(B<<8)\|A` printed as decimal |

### `draw_text`

Draw a single line of text.

```
VYOMA_DRAW:draw_text:<x>,<y>,<rgba>,<size>,<text>
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `x` | integer | Left edge (pixels) |
| `y` | integer | Top edge (pixels) |
| `rgba` | u32 | Text color |
| `size` | char | `s` = 4x8, `m` = 8x16, `l` = 16x32 |
| `text` | string | Text content (rest of line) |

### `draw_text_wrap`

Draw text with automatic word wrapping.

```
VYOMA_DRAW:draw_text_wrap:<x>,<y>,<max_width>,<rgba>,<size>,<text>
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `x` | integer | Left edge (pixels) |
| `y` | integer | Top edge (pixels) |
| `max_width` | integer | Maximum width before wrapping (pixels) |
| `rgba` | u32 | Text color |
| `size` | char | `s`, `m`, or `l` |
| `text` | string | Text content |

### `flush`

Commit the current frame to the framebuffer. Nothing is visible until this is called.

```
VYOMA_DRAW:flush
```

## Color Format

Colors are packed as a `u32` in RGBA format:

```
(Red << 24) | (Green << 16) | (Blue << 8) | Alpha
```

Printed as a decimal integer in the protocol string.

### Common Colors

| Color | Decimal | Hex |
|-------|---------|-----|
| White | `4294967295` | `0xFFFFFFFF` |
| Black | `255` | `0x000000FF` |
| Red | `4278190335` | `0xFF0000FF` |
| Green | `16744703` | `0x00FF00FF` |
| Blue | `65535` | `0x0000FFFF` |
| Catppuccin BG | `505290495` | `0x1E1E2EFF` |

In Rust, define colors as constants:

```rust
const WHITE: u32 = 0xFFFFFFFF;
const BG:    u32 = 0x1E1E2EFF;
const RED:   u32 = 0xFF0000FF;
```

## Font Sizes

| Size code | Pixel dimensions | Use case |
|-----------|-----------------|----------|
| `s` | 4x8 | Fine detail, status bars |
| `m` | 8x16 | Default body text |
| `l` | 16x32 | Headers, titles |

## Example: Complete App Frame

```rust
const WHITE: u32  = 0xFFFFFFFF;
const BG: u32     = 0x1E1E2EFF;
const PURPLE: u32 = 0x7C3AEDFF;
const GRAY: u32   = 0x64748BFF;

fn draw_frame() {
    // Clear background
    println!("VYOMA_DRAW:fill_rect:0,0,960,700,{BG}");

    // Header bar
    println!("VYOMA_DRAW:fill_rect:0,0,960,40,{PURPLE}");
    println!("VYOMA_DRAW:draw_text:10,12,{WHITE},m,My Application");

    // Body text
    println!("VYOMA_DRAW:draw_text:10,60,{WHITE},m,Welcome to VyomaOS!");
    println!("VYOMA_DRAW:draw_text:10,80,{GRAY},s,Press any key to continue...");

    // Commit to screen
    println!("VYOMA_DRAW:flush");
}
```

## Window Chrome

The supervisor automatically adds window chrome around each app's drawing area:

- **Title bar** with app name and deterministic accent color
- **Focus border** (2px) around the active window
- **Status strip** showing app name and uptime

Apps don't need to account for chrome in their coordinates — the supervisor handles offsetting.

## Performance

The supervisor tracks per-app flush rate and logs FPS every 5 seconds. To maintain smooth rendering, apps should aim for consistent flush intervals rather than flooding with draw commands.
