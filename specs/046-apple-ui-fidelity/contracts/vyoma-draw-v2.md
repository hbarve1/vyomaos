# Contract: VYOMA_DRAW Protocol v2

## Overview

VYOMA_DRAW v2 extends the existing line-oriented drawing protocol with scalable font rendering, image display, rounded rectangles with alpha, and compositing control. All v1 commands remain valid. v2 adds new commands; apps detect supervisor support by reading `VYOMA_SYSTEM:draw_version:2` from stdin at startup.

## Backwards Compatibility

All existing v1 commands continue to work unchanged:
- `VYOMA_DRAW:fill_rect:<x>,<y>,<w>,<h>,<rgba>`
- `VYOMA_DRAW:draw_text:<x>,<y>,<rgba>,<size>,<text>` (s/m/l size codes still valid)
- `VYOMA_DRAW:draw_text_wrap:<x>,<y>,<max_w>,<rgba>,<size>,<text>`
- `VYOMA_DRAW:flush`

## New Commands (v2)

### `draw_glyph` — scalable text rendering

```
VYOMA_DRAW:draw_glyph:<x>,<y>,<rgba>,<pt>,<weight>,<text>
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `x`, `y` | u32 | Top-left baseline origin in pixels |
| `rgba` | u32 | Text color packed as (R«24)|(G«16)|(B«8)|A |
| `pt` | u32 | Font size in points (8–96) |
| `weight` | str | `regular` or `bold` or `mono` |
| `text` | str | UTF-8 text; no embedded commas (use draw_glyph_escaped for commas) |

**Example**:
```
VYOMA_DRAW:draw_glyph:40,60,4294967295,16,bold,System Preferences
```

---

### `draw_image` — PNG image blit

```
VYOMA_DRAW:draw_image:<x>,<y>,<w>,<h>,<path>
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `x`, `y` | u32 | Top-left position on screen |
| `w`, `h` | u32 | Target width/height (image is scaled to fit) |
| `path` | str | Absolute path to PNG file in initramfs |

Supervisor decodes PNG on first use and caches the decoded RGBA buffer. Image is scaled with nearest-neighbor (speed) or bilinear (quality) based on supervisor config.

**Example**:
```
VYOMA_DRAW:draw_image:8,8,48,48,/apps/settings/icon.png
```

---

### `fill_rect_r` — rounded rectangle with alpha

```
VYOMA_DRAW:fill_rect_r:<x>,<y>,<w>,<h>,<rgba>,<radius>
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `x`,`y`,`w`,`h` | u32 | Rectangle bounds |
| `rgba` | u32 | Fill color with alpha channel |
| `radius` | u32 | Corner radius in pixels (0 = square, same as fill_rect) |

Uses distance-field coverage mask for anti-aliased corners. Alpha channel of `rgba` is composited using Porter-Duff "over".

**Example**:
```
VYOMA_DRAW:fill_rect_r:20,20,400,300,2566914303,12
```
(2566914303 = 0x9900FFFF = purple at ~60% alpha, 12px radius)

---

### `set_layer_alpha` — per-surface compositing alpha

```
VYOMA_DRAW:set_layer_alpha:<value>
```

Sets the alpha multiplier for this app's entire compositing layer (0–255). Used for fade-in/fade-out animations driven by the supervisor, but apps may also set it directly.

---

## Manifest Extensions (vyoma.toml)

```toml
[app]
name    = "settings"
version = "1.0.0"
wasm    = "settings.wasm"
icon    = "icon.png"          # NEW: path relative to app wasm directory

[capabilities]
display = true

[window]
x = 720
y = 52
w = 700
h = 600
z = 10
corner_radius = 12            # NEW: window chrome rounded corner radius

[[menu_items]]                # NEW: menu bar items declared by this app
label   = "File"
action  = "menu:file"

[[menu_items]]
label   = "Edit"
action  = "menu:edit"
```

## Supervisor → App Protocol Extensions

At app startup (if display=true), supervisor writes capability broadcasts:

```
VYOMA_SYSTEM:draw_version:2
VYOMA_SYSTEM:screen:1440,900
VYOMA_SYSTEM:display_profile:desktop
```

`display_profile` is new in v2 — apps can adapt layout to the active form factor.
