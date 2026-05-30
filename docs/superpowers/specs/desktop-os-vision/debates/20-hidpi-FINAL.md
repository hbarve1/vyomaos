# FINAL Spec: HiDPI & Multi-Resolution Display (Round 20)

**Subsystem**: HiDPI & Multi-Resolution Display  
**macOS Analogue**: `NSScreen.backingScaleFactor`, `NSScreen.frame` vs `NSScreen.visibleFrame`  
**Depends on**: R11 (Surface buffers), R13 (fonts), R17 (VYOMA_DRAW v3), R19 (virtual display)  
**Status**: FINAL — all blocking issues resolved

---

## 1. Overview

HiDPI support allows VyomaOS apps to render at high physical resolution while working in
logical point coordinates. Scale factor 1x = logical point equals physical pixel. Scale
factor 2x = one logical point equals a 2×2 block of physical pixels.

QEMU virtio-gpu does not expose EDID; scale factor defaults to 1x and can be overridden
in `boot.toml`. Integer scales only (1x, 2x) in v1. No fractional scaling.

### Non-Goals (v1)

- Fractional scale factors (1.5x, 1.25x)
- Sub-pixel rendering (ClearType equivalent)
- Per-window scale factor (all windows on same display share the display's scale factor)
- Automatic EDID-based scale detection (requires R6 driver update; deferred)

---

## 2. Scale Factor Model

### 2.1 Display Scale Factor

```rust
// supervisor/src/display_info.rs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleFactor { X1 = 1, X2 = 2 }

pub struct DisplayConfig {
    pub physical_width:  u32,   // physical fb pixels (e.g. 1920)
    pub physical_height: u32,   // physical fb pixels (e.g. 1080)
    pub scale_factor:    ScaleFactor,
    pub logical_width:   u32,   // physical_width / scale_factor
    pub logical_height:  u32,   // physical_height / scale_factor
}

impl DisplayConfig {
    pub fn new(physical_width: u32, physical_height: u32, scale_factor: ScaleFactor) -> Self {
        let sf = scale_factor as u32;
        Self {
            physical_width,
            physical_height,
            scale_factor,
            logical_width:  physical_width  / sf,
            logical_height: physical_height / sf,
        }
    }
}
```

### 2.2 boot.toml Override

```toml
[display]
scale_factor = 2   # 1 or 2; default 1
```

Validated at startup: if value is not 1 or 2, supervisor exits with error. If omitted,
defaults to 1.

### 2.3 Platform Defaults

| Profile | Default scale | Configurable |
|---------|--------------|-------------|
| desktop-full | 1 | yes (boot.toml) |
| mobile | 2 | yes |
| server-headless | 1 | no (always 1) |
| robotics-rt / iot-edge / mcu-minimal | 1 | no |

---

## 3. HiDPI Awareness Flag (B1 fix)

### 3.1 `hidpi_aware` Manifest Flag

```toml
[capabilities]
display     = true
hidpi_aware = true   # opt-in to logical point coordinate system
```

This is the key backward-compatibility mechanism. All pre-R20 apps have `hidpi_aware`
absent (defaults to `false`).

### 3.2 Legacy App Behavior at 2x (B1 fix — Surface size defined)

**Legacy apps** (`hidpi_aware = false` or absent) on a 2x display:

- Surface is allocated at **logical pixel size** (e.g. 960×540 at 2x on a 1920×1080 fb)
- `VYOMA_DISPLAY:window_info` reports `(960, 540, 1)` — scale_factor reported as 1
- `VYOMA_DRAW:` coordinates are treated as physical pixels (no scaling applied)
- The compositor **pixel-doubles** the 960×540 Surface to 1920×1080 before blitting to fb
  (nearest-neighbor 2x upscale)

This means: a legacy app's `fill_rect:0,0,960,540,bg` fills the entire 960×540 Surface.
The compositor doubles it to fill the physical screen. The UI looks blocky at 2x, but
correct — not zoomed, not clipped.

**Surface size for legacy app on 2x display**:
- Surface width = logical_width (physical_width / 2)
- Surface height = logical_height (physical_height / 2)
- `window_info` reports these logical dimensions with scale_factor=1

**HiDPI-aware apps** (`hidpi_aware = true`) on a 2x display:

- Surface is allocated at **physical pixel size** (1920×1080)
- `VYOMA_DISPLAY:window_info` reports `(960, 540, 2)` — logical dims + true scale
- `VYOMA_DRAW:` coordinates are in logical points; supervisor multiplies by 2 before drawing
- App fills the full 1920×1080 Surface with full-resolution content

**Summary**:

| App type | Scale 1x | Scale 2x |
|----------|----------|----------|
| Legacy | Surface = physical (960×540=960×540) | Surface = logical (960×540), upscaled by compositor |
| HiDPI-aware | Surface = physical | Surface = physical (1920×1080), draw coords scaled by 2 |

---

## 4. Coordinate Scaling (B2 fix — saturating multiply)

### 4.1 `scale_coord`

```rust
// supervisor/src/draw_cmd.rs

fn scale_coord(v: u32, sf: ScaleFactor) -> u32 {
    // saturating_mul: on overflow, clamps to u32::MAX
    // u32::MAX will be caught by the out-of-bounds clipper and discarded
    v.saturating_mul(sf as u32)
}
```

This is the **only** place coordinate scaling occurs. All `VYOMA_DRAW:` x, y, w, h values
pass through `scale_coord` for HiDPI-aware apps before any drawing.

### 4.2 Rounding Policy

For integer scale factors, `point × scale_factor` is always exact (no fractional part).
`scale_coord` never rounds — it is exact multiplication. The rounding policy for future
fractional scale factors is deferred.

### 4.3 Out-of-Bounds Clip

After scaling, coordinates are clamped to `[0, physical_width)` and `[0, physical_height)`:

```rust
fn clip_rect(x: u32, y: u32, w: u32, h: u32, fb_w: u32, fb_h: u32)
    -> Option<(u32, u32, u32, u32)>
{
    if x >= fb_w || y >= fb_h { return None; }
    let w = w.min(fb_w - x);
    let h = h.min(fb_h - y);
    if w == 0 || h == 0 { return None; }
    Some((x, y, w, h))
}
```

A saturated coordinate (u32::MAX) fails `x >= fb_w` and is discarded — no draw, no panic.

---

## 5. Surface Allocation

### 5.1 Physical Surfaces

All Surfaces are always in physical pixels. The allocation differs only by app type:

```rust
// supervisor/src/display.rs — allocate_surface_for_app

fn allocate_surface(config: &DisplayConfig, manifest: &AppManifest) -> Surface {
    let (w, h) = if manifest.capabilities.hidpi_aware {
        (config.physical_width, config.physical_height)
    } else {
        // Legacy: logical size; compositor will upscale
        (config.logical_width, config.logical_height)
    };
    Surface { width: w, height: h, pixels: vec![0u8; (w * h * 4) as usize] }
}
```

### 5.2 AppState Window Info

```rust
pub struct AppState {
    // ...
    pub surface: Surface,
    pub logical_width: u32,
    pub logical_height: u32,
    pub scale_factor: ScaleFactor,
    pub hidpi_aware: bool,
}
```

---

## 6. Startup Ordering Contract (B4 fix)

### 6.1 window_info Delivery

The supervisor sends `VYOMA_DISPLAY:window_info:<logical_w>,<logical_h>,<scale_factor>`
to the app's stdin immediately after spawning the Wasmtime child. This line is in the
app's stdin buffer before the WASM binary starts executing.

However, VyomaOS provides **no guarantee** that the app reads this line before emitting
its first `VYOMA_DRAW:` command. The supervisor processes `VYOMA_DRAW:` commands as they
arrive, regardless of whether the app has consumed `window_info`.

### 6.2 Required Initialization Pattern for HiDPI-aware Apps

Apps with `hidpi_aware = true` MUST wait for `window_info` before their first draw:

```rust
// Required pattern for hidpi_aware apps
fn main() {
    // 1. Read stdin until VYOMA_DISPLAY:window_info is received
    let (logical_w, logical_h, scale_factor) = wait_for_window_info();
    // 2. All subsequent VYOMA_DRAW: coordinates are in logical points
    // 3. surface is physical_w × physical_h = logical × scale_factor
    draw_ui(logical_w, logical_h);
}

fn wait_for_window_info() -> (u32, u32, u8) {
    for line in std::io::stdin().lines() {
        let line = line.unwrap();
        if let Some(rest) = line.strip_prefix("VYOMA_DISPLAY:window_info:") {
            let parts: Vec<&str> = rest.split(',').collect();
            if parts.len() >= 3 {
                let w = parts[0].parse().unwrap_or(960);
                let h = parts[1].parse().unwrap_or(540);
                let sf = parts[2].parse().unwrap_or(1);
                return (w, h, sf);
            }
        }
    }
    (960, 540, 1)  // fallback if stdin closes
}
```

**Consequence**: An `hidpi_aware` app that draws without waiting for `window_info` will
produce a provisional frame that may be mis-sized. This is accepted behavior; the app is
expected to redraw when the `window_info` line is eventually processed.

**Legacy apps** do not need to wait — they receive `scale_factor=1` and their coordinate
system is unchanged.

---

## 7. WIT Interface `vyoma:display-info@1.0.0`

```wit
package vyoma:display-info@1.0.0;

record screen-metrics {
    physical-width:  u32,
    physical-height: u32,
    logical-width:   u32,
    logical-height:  u32,
    scale-factor:    u8,     // 1 or 2
}

interface display-info {
    /// Get current display metrics for the calling app's display.
    screen-info: func() -> result<screen-metrics, string>;

    /// Get only the backing scale factor.
    backing-scale-factor: func() -> result<u8, string>;

    /// Get logical screen dimensions (physical / scale).
    logical-screen-size: func() -> result<tuple<u32, u32>, string>;

    /// Get physical screen dimensions.
    physical-screen-size: func() -> result<tuple<u32, u32>, string>;
}

world display-info-world {
    import display-info;
}
```

Capability gate: `display = true` required (link-time per R18 pattern).

---

## 8. VYOMA_DISPLAY: Stdout Protocol

```
VYOMA_DISPLAY:screen_info            → VYOMA_DISPLAY_INFO:<pw>,<ph>,<lw>,<lh>,<sf>
VYOMA_DISPLAY:scale_factor           → VYOMA_DISPLAY_SF:<sf>
VYOMA_DISPLAY:logical_size           → VYOMA_DISPLAY_LSIZE:<lw>,<lh>
VYOMA_DISPLAY:physical_size          → VYOMA_DISPLAY_PSIZE:<pw>,<ph>
```

Push notifications (supervisor → app stdin):
```
VYOMA_DISPLAY:window_info:<logical_w>,<logical_h>,<scale_factor>   ← at spawn
VYOMA_DISPLAY:scale_changed:<new_sf>                                ← if scale changes at runtime
```

---

## 9. VYOMA_VDISP_SCREEN Protocol Update (B3 fix)

### 9.1 Four-Field Format

R19 defined `VYOMA_VDISP_SCREEN:<display_id>,<w>,<h>` (3 fields). R20 adds:
```
VYOMA_VDISP_SCREEN:<display_id>,<physical_w>,<physical_h>,<scale_factor>
```

### 9.2 Migration Requirement (B3 fix — not backward-compatible)

The four-field format is a **breaking change** for apps using strict-length parsing.
All apps that handle `VYOMA_VDISP_SCREEN` must be updated to accept ≥3 fields, defaulting
`scale_factor` to 1 when the fourth field is absent:

```rust
// Required parser update for any app handling VYOMA_VDISP_SCREEN:
fn parse_vdisp_screen(line: &str) -> Option<(u32, u32, u32, u8)> {
    let rest = line.strip_prefix("VYOMA_VDISP_SCREEN:")?;
    let parts: Vec<&str> = rest.split(',').collect();
    if parts.len() < 3 { return None; }
    let display_id = parts[0].parse().ok()?;
    let pw = parts[1].parse().ok()?;
    let ph = parts[2].parse().ok()?;
    let sf: u8 = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    Some((display_id, pw, ph, sf))
}
```

**Migration note**: Any app from R19 that uses `parts.len() != 3` as an error check
must change to `parts.len() < 3`. The existing VyomaOS apps (`gui-demo`, `shell`) must
be audited and updated for this check before R20 is shipped.

---

## 10. Font Scaling (B5 fix — scale-invariant logical sizes)

### 10.1 Font Specifier Physical Dimensions

| Specifier | Base bitmap | 1x physical | 2x physical | Logical pts (always) |
|-----------|------------|------------|------------|---------------------|
| `s` | 4×8 | 4×8 px | 8×16 px | 4×8 pts |
| `m` | 8×16 | 8×16 px | 16×32 px | 8×16 pts |
| `l` | 16×32 | 16×32 px | 32×64 px | 16×32 pts |

**Font logical point size is scale-invariant**: an `m` glyph is always 8×16 logical
points regardless of scale factor. At 2x, the physical rendering is 16×32 pixels.

### 10.2 Layout Rule

Apps must use logical point dimensions for layout. To avoid overflow at `y = logical_h - N`,
use the logical glyph height from the table above:

```rust
// Safe bottom-aligned text (any scale factor):
let m_glyph_h_pts = 16;  // always 16 logical points for 'm' specifier
let y = logical_h - m_glyph_h_pts;  // guaranteed to not overflow at any scale
println!("VYOMA_DRAW:draw_text:{},{},{color},m,{text}", 8, y);
```

At 2x: `y_physical = (logical_h - 16) * 2 = physical_h - 32`. Physical glyph
height = 32. Bottom edge = `physical_h - 32 + 32 = physical_h`. Exactly fills to bottom.

### 10.3 At 2x: Auto-Promote vs Pixel-Double

At 2x, the `m` specifier renders at 16×32 physical pixels (same as 1x `l` size in
pixels). The supervisor does NOT auto-promote `m` to the `l` bitmap — it pixel-doubles
the 8×16 bitmap. The rendering quality is equivalent to `l` at 1x (same physical pixels,
same bitmap source). The `l` specifier at 2x pixel-doubles the 16×32 bitmap to 32×64
physical pixels.

For best quality at 2x: apps should use `m` where they would use `s` at 1x, and `l`
where they would use `m`, since physical sizes are doubled. This is a recommendation,
not a requirement.

### 10.4 Font Rendering Implementation

```rust
// supervisor/src/font.rs — draw_text with scale

pub fn draw_text_scaled(
    fb: &mut FrameBuffer,
    x: u32, y: u32,
    color: u32,
    size: FontSize,
    text: &str,
    scale: ScaleFactor,
) {
    let sf = scale as u32;
    let (glyph_w, glyph_h) = size.base_dims();  // base bitmap size
    let mut cursor_x = x;
    for ch in text.chars() {
        let bitmap = FONT_DATA.glyph(ch, size);
        // Pixel-double: each source pixel → sf×sf block
        for row in 0..glyph_h {
            for col in 0..glyph_w {
                if bitmap.bit(row, col) {
                    for dy in 0..sf {
                        for dx in 0..sf {
                            let px = scale_coord(cursor_x + col, scale).saturating_add(dx);
                            let py = scale_coord(y + row, scale).saturating_add(dy);
                            fb.set_pixel_checked(px, py, color);
                        }
                    }
                }
            }
        }
        cursor_x = cursor_x.saturating_add(glyph_w);
    }
}
```

`set_pixel_checked` silently discards any px/py that exceeds framebuffer bounds.

---

## 11. Compositor Changes for Legacy App Upscaling

```rust
// supervisor/src/compositor.rs — blit step for legacy apps on 2x display

fn blit_surface(fb: &mut FrameBuffer, app: &AppState, config: &DisplayConfig) {
    if !app.hidpi_aware && config.scale_factor == ScaleFactor::X2 {
        // Nearest-neighbor 2x upscale: each logical pixel → 2×2 block
        for y in 0..app.surface.height {
            for x in 0..app.surface.width {
                let pixel = app.surface.get_pixel(x, y);
                let px = x.saturating_mul(2).saturating_add(app.window_x.saturating_mul(2));
                let py = y.saturating_mul(2).saturating_add(app.window_y.saturating_mul(2));
                fb.set_pixel_checked(px,     py,     pixel);
                fb.set_pixel_checked(px + 1, py,     pixel);
                fb.set_pixel_checked(px,     py + 1, pixel);
                fb.set_pixel_checked(px + 1, py + 1, pixel);
            }
        }
    } else {
        // Normal 1:1 blit (both 1x display and 2x HiDPI-aware apps)
        blit_surface_1x1(fb, app);
    }
}
```

---

## 12. File Layout

```
supervisor/src/display_info.rs   (DisplayConfig, ScaleFactor, scale_coord)
supervisor/src/draw_cmd.rs       (updated: scale_coord applied for hidpi_aware apps)
supervisor/src/font.rs           (updated: draw_text_scaled with pixel-doubling)
supervisor/src/compositor.rs     (updated: blit_surface with legacy 2x upscale)
supervisor/src/display.rs        (updated: allocate_surface respects hidpi_aware)
supervisor/src/wit_handlers.rs   (display-info WIT closures)
```

---

## 13. Blocking Issue Resolution Summary

| Issue | Resolution |
|-------|-----------|
| B1: Legacy app Surface size undefined | Legacy apps get `logical_w × logical_h` Surface; compositor upscales 2×. HiDPI apps get `physical_w × physical_h` Surface. `window_info` reports logical dims + true scale_factor. |
| B2: u32 coordinate overflow at 2x | `scale_coord` uses `saturating_mul` — overflow clamps to u32::MAX, caught by out-of-bounds clipper, discarded |
| B3: VYOMA_VDISP_SCREEN backward compat | Explicitly a breaking change; apps must update to accept ≥3 fields; migration note provided with exact code |
| B4: Startup race before window_info | Documented contract: HiDPI apps MUST wait for `window_info` before first draw; provisional frame behavior defined; idiomatic startup pattern provided |
| B5: Font `l` overflow at 2x | Font logical point sizes defined as scale-invariant; layout guide provided; overflow case handled by `set_pixel_checked` clipping |

---

## 14. Open Questions (deferred)

1. **EDID-based auto-detection**: R6 (drivers) needs an extension to read EDID via DRM.
   When available, supervisor auto-sets `scale_factor` from EDID `preferred_mode`.
2. **Fractional scaling**: 1.5x requires sub-pixel position math. Deferred to a post-v1 round.
3. **Per-window scale factor**: All windows share display scale in v1. When R21 (Window
   Manager) adds window-to-display mapping, per-window override becomes possible.
4. **`hidpi_aware` + virtual display**: Apps rendering to a virtual display from R19 use
   the virtual display's scale factor. The `current-display` WIT function (R19) returns the
   display_id; apps can call `screen-info` with that ID to get scale_factor.
