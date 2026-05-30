# Round 20 — HiDPI & Multi-Resolution Display (Architect)

**Status**: Draft
**Round**: 20
**Subsystem**: HiDPI & Multi-Resolution Display
**Analogue**: macOS Retina Display (`NSScreen.backingScaleFactor`, `NSScreen.frame` vs `NSScreen.visibleFrame`)
**Author**: Architect
**Date**: 2026-05-29

---

## 1. Overview and Motivation

VyomaOS's display stack was built against a single coordinate system: physical pixels on
a framebuffer device. Every `VYOMA_DRAW:` command emitted by a WASM app specifies x, y,
width, height directly in pixels. Every Surface buffer (introduced in R11) is sized in
pixels. Every bitmap font glyph (R13) is measured in pixels. This was correct and
sufficient for the single-display, single-resolution, 1x world that Phase 1–19 targeted.

HiDPI (high dots-per-inch) displays — colloquially Retina displays on Apple hardware —
change the relationship between the physical pixel grid and the logical coordinate space an
application author reasons about. A 1920×1080 physical framebuffer on a high-DPI panel
might present visually as a 960×540 logical workspace, with the compositor filling each
logical point with a 2×2 block of physical pixels. This preserves layout dimensions (a
100-point-wide button stays 100 points wide) while doubling the sharpness of rasterized
content that is resolution-aware.

Without a HiDPI model, three failure modes appear as VyomaOS moves toward real hardware:

1. **Tiny text**: On a physical 2560×1600 HiDPI panel at 1x mapping, an 8-pixel-tall font
   character is visually 2mm tall — unreadable.
2. **Coordinate fragmentation**: Apps that query their logical window dimensions and
   receive physical pixel sizes must do their own DPI math, producing divergent
   per-app logic that breaks consistency.
3. **Font fidelity loss**: The R13 bitmap font library contains glyphs designed at specific
   physical pixel pitches (8×16, 16×32). At 2x scale, pixel-doubling is the correct
   strategy; applying 1x font metrics to a 2x screen produces muddy, over-scaled text that
   does not take advantage of the available pixel density.

The macOS analogue is `NSScreen.backingScaleFactor`: a property that returns 1.0 on
standard displays and 2.0 on Retina panels. Apps read this value and provide 2x assets
(e.g., `image@2x.png`) so that the compositor can render crisply without upscaling. Apps
that ignore it are upscaled by the compositor, producing a blurry but functional result.
VyomaOS v1 adopts a similar model: apps that opt in to HiDPI receive crisp rendering; apps
that do not opt in are pixel-doubled by the compositor unchanged.

### In Scope (v1)

- Integer scale factors only: 1x and 2x.
- Per-display scale factor stored in supervisor state; virtual displays (R19) have their
  own independent scale factors.
- `VYOMA_DISPLAY:` stdout protocol for apps to query display info.
- `vyoma:display-info@1.0.0` WIT interface for WASM apps compiled against the new ABI.
- `display_scale_factor` boot.toml override for QEMU environments where EDID is absent.
- Font bitmap scaling: pixel-doubling at 2x (repeat each pixel as a 2×2 block).
- Backward compatibility: apps without `hidpi_aware = true` in vyoma.toml continue to
  receive 1x coordinate semantics regardless of the physical scale factor.
- Minimal new source files: `supervisor/src/display_info.rs`; changes to `display.rs`,
  `draw_cmd.rs`, and `font.rs`.

### Out of Scope (v1)

- Fractional scale factors (1.25x, 1.5x, 1.75x) — requires sub-pixel rasterization.
- Sub-pixel antialiasing (ClearType / LCD rendering) — reserved for a future font round.
- Per-window scale factor (all windows on the same physical display share one scale factor).
- Dynamic scale-factor change at runtime without supervisor restart (boot.toml change
  requires reboot in v1).
- EDID parsing from real hardware — the infrastructure is designed for it but the
  implementation defers to a future hardware-bring-up round.
- Multi-monitor spanning (two physical displays with different scale factors composited
  into one coordinate space) — reserved for a multi-monitor round.

---

## 2. Scale Factor Model

### 2.1 Integer-Only Scale Factors

VyomaOS v1 supports exactly two scale factor values:

| Value | Meaning |
|-------|---------|
| `1` | Standard density: 1 logical point = 1 physical pixel |
| `2` | HiDPI: 1 logical point = 2×2 physical pixels |

No other values are accepted. The `display_scale_factor` field in boot.toml is validated at
supervisor startup; any value other than `1` or `2` causes a fatal error with a descriptive
log message.

### 2.2 Per-Display Scale Factor

Each display in the supervisor display registry carries its own `scale_factor: u8` field.
This applies to both physical displays and virtual displays created by R19. Two virtual
displays may have different scale factors (e.g., a 2x virtual display for a CI screenshot
harness that simulates a Retina screen alongside a 1x virtual display for a remote viewer
on a standard monitor).

The physical display's scale factor is determined by the following priority order:

1. `display_scale_factor` field in the `[display]` section of boot.toml (highest priority).
2. EDID-reported DPI — reserved, not implemented in v1 (the code path exists but always
   returns `None` for virtio-gpu).
3. Default: `1`.

Virtual displays created via `vdisp create` (R19) accept an optional `scale_factor`
argument. If not specified, they inherit the physical display's scale factor.

### 2.3 Scale Factor in Supervisor State

The `DisplayConfig` struct (in `display.rs`) gains one field:

```rust
pub struct DisplayConfig {
    pub width_px:      u32,   // physical framebuffer width in pixels
    pub height_px:     u32,   // physical framebuffer height in pixels
    pub scale_factor:  u8,    // 1 or 2; validated at startup
}
```

Derived properties computed on-demand:

```rust
impl DisplayConfig {
    pub fn width_pts(&self)  -> u32 { self.width_px  / self.scale_factor as u32 }
    pub fn height_pts(&self) -> u32 { self.height_px / self.scale_factor as u32 }
}
```

These are always integers because `width_px` and `height_px` for a 2x display are always
even (the QEMU virtio-gpu resolution is chosen by the operator; the convention is to use
even dimensions). If `width_px` is odd and `scale_factor == 2`, the supervisor logs a
warning and treats `width_pts` as `width_px / 2` rounded down (floor division).

---

## 3. Point vs Pixel Coordinate System

### 3.1 Definitions

| Term | Definition |
|------|-----------|
| **Physical pixel** | One addressable element in the framebuffer (`/dev/fb0`). Every pixel in the fb is `4 bytes` (BGRA32). |
| **Logical point** | The coordinate unit used by applications. At 1x, one point = one pixel. At 2x, one point = a 2×2 pixel block. |
| **Scale factor** | The integer multiplier from points to pixels: `pixel = point × scale_factor`. |

### 3.2 App Coordinate Space

WASM apps that declare `hidpi_aware = true` in their `vyoma.toml` work entirely in logical
points. When such an app issues:

```
VYOMA_DRAW:fill_rect:0,0,960,540,<rgba>
```

on a 2x display (physical fb: 1920×1080), the supervisor interprets `x=0, y=0, w=960,
h=540` as points and converts to pixels before writing to the fb:

```
x_px = 0 × 2 = 0
y_px = 0 × 2 = 0
w_px = 960 × 2 = 1920
h_px = 540 × 2 = 1080
```

The fill covers the entire 1920×1080 physical framebuffer.

### 3.3 Coordinate Rounding Policy

When scaling point coordinates to pixels, the supervisor applies the following rounding
rules uniformly across all `VYOMA_DRAW:` parameters:

- **x, y (origin)**: `floor(point × scale_factor)`. This ensures the drawn region starts
  at or before the logical position, preventing one-pixel gaps at aligned boundaries.
- **w, h (dimensions)**: `floor(point × scale_factor)`. This ensures drawn regions never
  exceed the app's logical allocation.

Because the scale factor is always an integer (1 or 2) in v1, `point × scale_factor` is
always an exact integer — rounding is a no-op for all valid inputs. The floor policy is
specified explicitly so that future fractional scale support has a defined precedent to
extend or override.

### 3.4 Backward Compatibility — Non-HiDPI-Aware Apps

Apps that do NOT declare `hidpi_aware = true` in their manifest receive unchanged
treatment: their `VYOMA_DRAW:` coordinates are treated as physical pixel coordinates and
written directly to the fb without scaling, regardless of `scale_factor`. On a 2x display
(1920×1080 physical), a legacy app that draws to the full `0,0,960,540` range occupies
the top-left quadrant of the physical screen.

This is intentionally correct behavior: legacy apps continue to work without modification.
They occupy a fraction of the physical screen at the density they were designed for. The
supervisor may (in a future round) upscale the legacy app's Surface using pixel-doubling
so it fills the full logical screen; that upscaling path is out of scope for v1.

The supervisor determines HiDPI awareness at app startup by checking the manifest field.
The `AppState` struct gains a boolean `hidpi_aware: bool` mirroring the manifest value.
All draw-command scaling is gated on this flag.

---

## 4. Font Scaling

### 4.1 R13 Bitmap Font Sizes

R13 defined three font size specifiers, each mapping to a fixed bitmap glyph size:

| Specifier | Glyph width × height | Use case |
|-----------|---------------------|----------|
| `s` | 4×8 pixels | Dense data tables, log output |
| `m` | 8×16 pixels | Standard UI text |
| `l` | 16×32 pixels | Headings, status bar |

These sizes are physical pixel sizes. At 1x, they render crisply. At 2x on a physical
1920×1080 screen, a glyph drawn at 8×16 physical pixels occupies 4×8 logical points —
half the logical size it did at 1x. Text appears tiny.

### 4.2 Scaling Strategy: Pixel-Doubling

For HiDPI-aware apps at 2x, the font renderer doubles the glyph bitmap: each source pixel
is written as a 2×2 block in the output. This is the correct, lossless strategy for bitmap
fonts. There is no sub-pixel interpolation in v1.

Effective physical sizes at 2x:

| Specifier | 1x physical | 2x physical |
|-----------|------------|------------|
| `s` | 4×8 | 8×16 |
| `m` | 8×16 | 16×32 |
| `l` | 16×32 | 32×64 |

The `draw_text` compositor path in `draw_cmd.rs` receives a `scale_factor: u8` parameter.
When `scale_factor == 2` and the app is HiDPI-aware, the glyph loop doubles each pixel
write:

```rust
for row in 0..glyph_h {
    for col in 0..glyph_w {
        let on = glyph_bit(glyph_data, row, col);
        if on {
            let px = base_x + col * sf;
            let py = base_y + row * sf;
            for dy in 0..sf {
                for dx in 0..sf {
                    fb_write_pixel(px + dx, py + dy, color);
                }
            }
        }
    }
}
```

where `sf = scale_factor as usize`.

### 4.3 Font Size Auto-Promotion

When `scale_factor == 2` and a HiDPI-aware app requests font specifier `s` or `m`, the
supervisor does NOT automatically promote to the next larger specifier. Auto-promotion
would change layout dimensions in ways the app did not request and cannot anticipate.
Instead, pixel-doubling is applied to whatever specifier the app requested. The app
controls the trade-off: if it wants the visual appearance of an `m`-glyph on a 2x screen,
it requests `m` and receives a 16×32 physical pixel glyph via pixel-doubling.

An app that explicitly requests `l` on a 2x display receives a 32×64 physical pixel glyph.
This is intentionally large. The spec documents this as correct: on a 2x display, the `l`
specifier means "large heading text appropriate for this display's density."

### 4.4 R17 `set-font` Interaction

R17 VYOMA_DRAW v3 introduced a `set-font` command that sets the active font for subsequent
`draw_text` calls. The `set-font` command now accepts an optional `hidpi` hint:

```
VYOMA_DRAW:set_font:<family>,<specifier>,<hidpi_hint>
```

where `<hidpi_hint>` is `0` (render at declared pixel size, no scaling) or `1` (scale with
display scale factor). For backward compatibility, the `<hidpi_hint>` field is optional and
defaults to `0`. HiDPI-aware apps that want crisp font rendering must pass `hidpi_hint=1`.

---

## 5. Surface Resolution

### 5.1 Surfaces Are Physical-Pixel Buffers

Per the R11 design, each AppState has a `Surface` that serves as the app's backing store
for compositing. The Surface is a flat BGRA32 pixel array of `width × height` bytes × 4.

This remains true in R20. **Surfaces are always sized in physical pixels.** There is no
logical-pixel surface. This design avoids a category of compositor complexity: the R11
compositor reads raw pixel data from each Surface and blits it to the framebuffer; if
Surfaces were in logical pixels, the compositor would need to scale them, introducing
interpolation choices.

### 5.2 Logical vs Physical Surface Dimensions

For a HiDPI-aware app on a 2x display where the logical window is 960×540 points:

- Logical dimensions: 960×540 (what the app reasons about for layout)
- Physical Surface size: 1920×1080 pixels (what the app must fill)

The app queries its logical dimensions and the scale factor, then allocates and fills the
Surface at physical resolution. An app that fills only 960×540 pixels of its 1920×1080
Surface will display in the top-left quadrant of its window; the rest will be whatever
the Surface was initialized to (black / transparent).

### 5.3 How Apps Learn Their Logical Dimensions

At app startup (and on any display resize or reassignment), the supervisor sends a stdin
notification to HiDPI-aware apps:

```
VYOMA_DISPLAY:window_info:<logical_w>,<logical_h>,<scale_factor>
```

Example for a 960×540 logical window on a 2x display:

```
VYOMA_DISPLAY:window_info:960,540,2
```

The app derives its required Surface allocation:

```rust
let phys_w = logical_w * scale_factor;
let phys_h = logical_h * scale_factor;
// Allocate Surface at phys_w × phys_h
```

For legacy (non-HiDPI-aware) apps, the supervisor sends the physical pixel dimensions as
before (matching R11 behavior), with `scale_factor=1` to avoid breaking existing parsing
code that may read only two fields.

### 5.4 Surface Initialization

Surfaces are zero-initialized (all bytes 0, which is BGRA `(0,0,0,0)` — fully transparent
black). The compositor treats zero-alpha pixels as transparent, so an app that has not yet
drawn a frame shows the desktop background through its window region.

---

## 6. WIT Interface `vyoma:display-info@1.0.0`

### 6.1 Interface Definition

New file: `wit/display-info.wit`

```wit
package vyoma:display-info@1.0.0;

interface display-info {
    /// Scale factor for the display this app is currently assigned to.
    /// Returns 1 or 2.
    backing-scale-factor: func() -> u8;

    /// Logical screen dimensions in points.
    logical-screen-size: func() -> tuple<u32, u32>;

    /// Physical screen dimensions in pixels.
    physical-screen-size: func() -> tuple<u32, u32>;

    /// Full screen-info bundle (avoids three round-trips).
    record screen-info {
        logical-width:   u32,
        logical-height:  u32,
        physical-width:  u32,
        physical-height: u32,
        scale-factor:    u8,
    }
    screen-info: func() -> screen-info;
}

world display-info-world {
    import display-info;

    /// Push event: sent by supervisor when the display's scale factor changes.
    /// In v1 this is only sent on initial assignment; dynamic changes are out of scope.
    export display-changed: func(info: display-info::screen-info);
}
```

### 6.2 Host Implementation

The host implementation of `vyoma:display-info@1.0.0` lives in
`supervisor/src/display_info.rs`. It reads from the per-app `AppState`'s
`display_config: DisplayConfig` reference and returns the appropriate values.

The `display-changed` export is called by the supervisor when:

1. The app is first spawned (initial assignment to a display).
2. The app is reassigned to a different display (R19 `vdisp assign` command).
3. The app's display scale factor changes (future: dynamic scale change; v1 sends this
   only at spawn and reassignment).

### 6.3 WIT Capability Gating

Apps access `vyoma:display-info@1.0.0` only if they declare `display = true` in their
`vyoma.toml`. The capability gating follows the R18 WIT link-time pattern: if `display`
is not in the manifest, the `display-info` import is stubbed to return default values
(scale_factor=1, logical_size=physical_size). This ensures existing apps compiled without
the new WIT interface continue to link and run.

---

## 7. VYOMA_DISPLAY: Stdout Protocol

For WASM apps that use the stdout protocol rather than WIT, the following lines are defined
under the `VYOMA_DISPLAY:` namespace. All lines are sent by the supervisor on the app's
stdin; apps do not send `VYOMA_DISPLAY:` lines (those are supervisor → app).

### 7.1 window_info

Sent at startup and on reassignment:

```
VYOMA_DISPLAY:window_info:<logical_w>,<logical_h>,<scale_factor>
```

Fields:
- `logical_w`: window width in logical points (u32)
- `logical_h`: window height in logical points (u32)
- `scale_factor`: 1 or 2 (u8)

### 7.2 display_changed

Sent when the app is moved to a display with a different scale factor:

```
VYOMA_DISPLAY:display_changed:<new_logical_w>,<new_logical_h>,<new_scale_factor>
```

Apps that do not handle this line gracefully (e.g., old apps that ignore unknown stdin
prefixes) will simply not adapt to the new scale. Their Surface will be mis-sized until
restart. This is acceptable behavior in v1.

### 7.3 Querying Display Info

Apps may request current display info by writing to stdout:

```
VYOMA_DISPLAY:query_info
```

The supervisor responds with a `VYOMA_DISPLAY:window_info:...` line on the app's stdin.
This round-trip query is provided for apps that lose state (e.g., after a soft reset) and
need to re-sync without a full restart.

### 7.4 Integration with R19 VYOMA_VDISP_SCREEN

R19 introduced `VYOMA_VDISP_SCREEN:<display_id>,<w>,<h>` for virtual display assignment.
In R20, this message is extended to include the scale factor:

```
VYOMA_VDISP_SCREEN:<display_id>,<physical_w>,<physical_h>,<scale_factor>
```

The additional `<scale_factor>` field is appended for backward compatibility. Apps that
parsed the R19 three-field format and ignore trailing fields continue to work correctly.
The `<physical_w>` and `<physical_h>` fields remain physical pixels as in R19 (this is
required for the R19 contract to hold). Apps that want logical dimensions compute them as
`physical / scale_factor`.

---

## 8. Compositor Changes

### 8.1 Compositor Is Scale-Agnostic

The R11/R12 compositor reads per-app Surface pixel buffers and blits them to the
framebuffer in Z-order. It operates entirely in physical pixels. This design is preserved
unchanged in R20.

At 2x, a HiDPI-aware app produces a 1920×1080 Surface for a 960×540 logical window. The
compositor sees a 1920×1080 Surface, reads it, and blits its pixels to the 1920×1080
physical framebuffer. No scaling is performed by the compositor.

At 1x, the same app produces a 960×540 Surface for a 960×540 logical window. The
compositor blits 960×540 pixels to the 960×540 physical framebuffer. No scaling is
performed.

This "scale early, composite raw" model means the compositor never needs to know the scale
factor. All scaling decisions are made at two points: (a) inside the `draw_cmd.rs`
dispatcher (scales `VYOMA_DRAW:` coordinates before rendering to the Surface) and (b) in
the app itself (sizes its Surface buffer at physical resolution).

### 8.2 Blit Path Unchanged

The `blit_surface` function (introduced in R11) copies a Surface's pixel buffer to the
framebuffer at a specified (x, y) offset. The offset and dimensions are in physical pixels.
This function does not change in R20.

### 8.3 vsync_lock Interaction

The `vsync_lock: Arc<RwLock<()>>` (write-lock during DRM blit, read-lock otherwise) is
unchanged. Scale-factor computation adds no lock-dependent paths; scale factor is a static
value read from `DisplayConfig` without synchronization.

---

## 9. VYOMA_DRAW: Protocol at HiDPI

### 9.1 Coordinate Interpretation for HiDPI-Aware Apps

For apps with `hidpi_aware = true`, the supervisor multiplies x, y, w, h values in all
`VYOMA_DRAW:` commands by `scale_factor` before executing the draw operation on the
physical Surface. This applies to all geometric commands:

| Command | Scaled fields |
|---------|--------------|
| `fill_rect:x,y,w,h,rgba` | x, y, w, h |
| `draw_text:x,y,rgba,size,text` | x, y (size handled separately, see §4) |
| `draw_text_wrap:x,y,max_w,rgba,size,text` | x, y, max_w |
| `draw_line:x1,y1,x2,y2,rgba` | x1, y1, x2, y2 |
| `draw_rect:x,y,w,h,rgba` | x, y, w, h |
| `blit_image:x,y,w,h,...` | x, y, w, h |

### 9.2 Text Size Scaling

Text size specifiers `s`, `m`, `l` are scaled by pixel-doubling the glyph bitmap when
`scale_factor == 2` and the app is HiDPI-aware. The x, y text origin is also multiplied
by `scale_factor` as with geometric commands.

### 9.3 Non-HiDPI-Aware Apps

For apps without `hidpi_aware = true`, all `VYOMA_DRAW:` coordinates are treated as
physical pixels. The scale factor is ignored for these apps. This is the backward
compatibility guarantee: the draw pipeline is unchanged for legacy apps.

### 9.4 Protocol Version Field

The `VYOMA_DRAW:` protocol gains a no-op version declaration line that apps may optionally
emit at startup to signal their protocol expectations:

```
VYOMA_DRAW:version:3.1
```

Version 3.1 implies HiDPI-aware coordinate semantics. The supervisor uses this line only
for diagnostic logging in v1; the actual HiDPI-awareness gate remains the `hidpi_aware`
manifest flag, not this line. A future round may promote this to a runtime switch.

---

## 10. Platform Matrix

Scale factor applicability by platform profile:

| Platform | Scale Factor | Notes |
|----------|-------------|-------|
| `mcu-minimal` | Always 1 | 128KB RAM; no framebuffer complexity |
| `iot-edge` | Always 1 | SBC panels are 1x; GPIO-driven LCD displays are 1x |
| `robotics-rt` | Always 1 | Industrial panels, not HiDPI consumer hardware |
| `mobile` | 1 or 2 | Tablet panels may be HiDPI; configurable via boot.toml |
| `desktop-full` | 1 or 2 | Workstation monitor may be HiDPI; configurable via boot.toml |
| `server-headless` | Always 1 | No display hardware; virtual displays default to 1x |

For `mcu-minimal`, `iot-edge`, and `robotics-rt`, the `display_scale_factor` boot.toml
field is ignored if present (a warning is logged). The `DisplayConfig.scale_factor` field
is hardcoded to `1` in the platform profile for these targets.

For `mobile` and `desktop-full`, the default is `1` unless overridden. In QEMU, the
virtio-gpu driver does not provide EDID, so the operator must set `display_scale_factor = 2`
explicitly in boot.toml to enable 2x mode.

---

## 11. boot.toml Override

### 11.1 Field Definition

The `[display]` section of boot.toml gains a new optional field:

```toml
[display]
width  = 1920
height = 1080
display_scale_factor = 2   # optional; default = 1; must be 1 or 2
```

### 11.2 Validation

At supervisor startup, the manifest parser validates `display_scale_factor`:

- If absent: `scale_factor = 1`.
- If `1` or `2`: accepted.
- If any other value: fatal error, supervisor exits with message:
  ```
  [display] display_scale_factor must be 1 or 2, got: <value>
  ```

### 11.3 Interaction with Display Dimensions

When `display_scale_factor = 2`, the supervisor expects `width` and `height` to be the
physical framebuffer dimensions (e.g., 1920×1080). The logical dimensions are derived:
960×540 points. If `width` or `height` is odd and `scale_factor = 2`, the supervisor
logs a warning and uses floor division for the logical dimension calculation.

### 11.4 Example boot.toml for QEMU HiDPI Testing

```toml
[display]
width  = 1920
height = 1080
display_scale_factor = 2

[apps]
gui-demo   = { manifest = "/etc/vyoma/gui-demo/vyoma.toml" }
shell      = { manifest = "/etc/vyoma/shell/vyoma.toml" }
```

And in the `gui-demo` vyoma.toml:

```toml
[app]
name    = "gui-demo"
version = "0.1.0"
wasm    = "gui-demo.wasm"

[capabilities]
display    = true
hidpi_aware = true
```

---

## 12. File Layout

### 12.1 New Files

#### `supervisor/src/display_info.rs`

Responsibilities:
- Defines `DisplayConfig` struct (width_px, height_px, scale_factor).
- Implements helper methods: `width_pts()`, `height_pts()`, `logical_to_physical_coord()`.
- Implements `scale_draw_params()`: takes a `DrawCmd` and a `scale_factor`, returns a new
  `DrawCmd` with all geometric fields multiplied. For `scale_factor == 1`, returns the
  input unchanged (zero-cost path).
- Implements the WIT host bindings for `vyoma:display-info@1.0.0`.
- Sends `VYOMA_DISPLAY:window_info:` on app stdin at spawn time.

Target size: ≤ 200 lines.

#### `wit/display-info.wit`

WIT interface definition as specified in §6.1.

Target size: ≤ 50 lines.

### 12.2 Modified Files

#### `supervisor/src/display.rs`

Changes:
- Import `DisplayConfig` from `display_info.rs`.
- Replace the inline `width`, `height` fields in any existing display state struct with
  `DisplayConfig`.
- Read `display_scale_factor` from parsed boot.toml and populate `DisplayConfig`.
- Pass `DisplayConfig` reference into `draw_cmd.rs` dispatcher.

Estimated delta: +30 lines, −10 lines.

#### `supervisor/src/draw_cmd.rs`

Changes:
- All `VYOMA_DRAW:` command dispatchers receive the app's `hidpi_aware` flag and a
  `scale_factor: u8`.
- Add `scale_coord(v: u32, sf: u8) -> u32 { v * sf as u32 }` helper.
- Apply scaling to geometric parameters before Surface writes.
- Pass `scale_factor` into font rendering call.

Estimated delta: +60 lines, −5 lines.

#### `supervisor/src/font.rs`

Changes:
- `render_glyph(fb, x, y, glyph_data, color, scale_factor: u8)` — the inner pixel write
  loop is generalized from the single-pixel write to a `scale_factor × scale_factor` block
  write as specified in §4.2.
- The glyph stride computation is multiplied by `scale_factor`.

Estimated delta: +25 lines, −10 lines.

#### `supervisor/src/main.rs` / `app_state.rs`

Changes:
- Add `hidpi_aware: bool` field to `AppState`, populated from manifest parsing.
- Pass `DisplayConfig` reference to app startup handler.
- Call `display_info::send_window_info()` at app spawn time.

Estimated delta: +20 lines.

### 12.3 No Changes Required

- `compositor.rs` / `blit_surface` — no changes. Compositor is scale-agnostic.
- `ipc.rs` — no changes.
- `vsync_lock` usage — no changes.
- Existing app source files — no changes required for backward compatibility.

---

## 13. Manifest Changes

### 13.1 New Capability Field

`vyoma.toml` gains one new optional boolean field under `[capabilities]`:

```toml
[capabilities]
display     = true
hidpi_aware = true   # optional; default = false
```

`hidpi_aware = true` is only meaningful when `display = true`. If `hidpi_aware = true`
but `display = false`, the supervisor logs a warning and ignores `hidpi_aware`.

### 13.2 Schema Validation

The manifest schema validator (`check-manifests` target) is updated to accept
`hidpi_aware` as a known field of type `bool` under `[capabilities]`. Unknown-field
errors for `hidpi_aware` in existing validation runs are eliminated.

---

## 14. Testing Strategy

### 14.1 Unit Tests

New tests in `supervisor/tests/display_info_tests.rs`:

- `test_scale_1x_coords_unchanged()` — `scale_coord(10, 1) == 10`
- `test_scale_2x_coords_doubled()` — `scale_coord(10, 2) == 20`
- `test_logical_size_1x()` — 960×540 display at 1x: `width_pts() == 960`
- `test_logical_size_2x()` — 1920×1080 display at 2x: `width_pts() == 960`
- `test_odd_width_floor_division()` — 1921 physical width at 2x: `width_pts() == 960`,
  warning logged.
- `test_invalid_scale_factor_in_boot_toml()` — `display_scale_factor = 3` triggers fatal
  error (test catches the error return).
- `test_non_hidpi_app_coords_unscaled()` — app without `hidpi_aware` at 2x: draw commands
  use raw pixel coordinates.
- `test_hidpi_app_coords_scaled()` — app with `hidpi_aware = true` at 2x: draw commands
  have doubled coordinates.
- `test_font_pixel_doubling_2x()` — glyph rendered at 2x produces 2×2 pixel blocks.
- `test_window_info_message_format()` — `VYOMA_DISPLAY:window_info:960,540,2` parsed
  correctly.
- `test_vdisp_screen_backward_compat()` — three-field `VYOMA_VDISP_SCREEN` message still
  parses correctly (scale_factor defaults to 1).

### 14.2 Smoke Test

The smoke test (`make smoke`) is unaffected — it boots headless without a framebuffer and
does not exercise display paths.

### 14.3 Manual QEMU Test

Using boot.toml with `display_scale_factor = 2` and `make run-gui`, verify:

1. `gui-demo` (with `hidpi_aware = true`) renders at 2x with doubled glyph sizes.
2. `shell` (without `hidpi_aware`) renders in the top-left quadrant of the 2x screen,
   unscaled.
3. `VYOMA_DISPLAY:window_info:960,540,2` appears in `log gui-demo` output.
4. Font `m` text on 2x display matches visual size of font `m` text on a 1x display at
   half the physical resolution (same logical size).

---

## 15. Error Handling and Edge Cases

### 15.1 App Draws Outside Logical Bounds

At 2x, the logical screen is 960×540 points. A HiDPI-aware app that emits:

```
VYOMA_DRAW:fill_rect:950,530,20,20,<rgba>
```

has `x_px = 1900, y_px = 1060, w_px = 40, h_px = 40`. The right edge is at pixel 1940
and the bottom edge is at pixel 1100, both exceeding the 1920×1080 physical framebuffer.

The supervisor's existing out-of-bounds clipping (introduced in R11) handles this: any
write that extends beyond `DisplayConfig.width_px` or `DisplayConfig.height_px` is clipped
to the framebuffer boundary. The clipping operates on physical pixels after scaling, so
the overflow is handled uniformly for both 1x and 2x apps.

### 15.2 Scale Factor Change During App Lifetime

In v1, scale factor changes require supervisor restart (boot.toml change + reboot). There
is no dynamic scale-factor change path. Apps receive `VYOMA_DISPLAY:window_info` once, at
spawn, and that value does not change during their lifetime.

This constraint is explicitly noted in the `display-changed` WIT event definition: in v1
the event fires only at spawn and at R19 display-reassignment; it does not fire for a
mid-session scale-factor change (because that path is not implemented).

### 15.3 Virtual Display Scale Factor Mismatch

When an app is assigned to a virtual display (R19) with a different scale factor than the
physical display it previously rendered on, the supervisor sends
`VYOMA_DISPLAY:display_changed:` and (for R19 compat) `VYOMA_VDISP_SCREEN:`. The app is
expected to resize its Surface and repaint. If the app does not respond to the
`display_changed` notification, it continues to render with the wrong scale factor; the
compositor blits whatever the Surface contains without error.

### 15.4 Non-Display-Capable Apps

Apps without `display = true` do not receive `VYOMA_DISPLAY:window_info:` notifications.
Sending display info to a non-display app would be confusing and wasteful. The supervisor
checks `app_state.capabilities.display` before sending any `VYOMA_DISPLAY:` stdin lines.

### 15.5 QEMU Width/Height Alignment Requirement

Operators configuring `display_scale_factor = 2` in boot.toml for QEMU testing are
encouraged to use even-valued physical dimensions. The QEMU `-vga virtio` or
`-device virtio-gpu-pci` options accept arbitrary framebuffer sizes, so this is trivially
satisfied by choosing 1920×1080, 2560×1440, or similar standard resolutions.

The Makefile's `run-gui` target is updated for the `desktop-full` platform to include a
note in the comments (not a change to default behavior) that operators can override with:

```bash
DISPLAY_WIDTH=1920 DISPLAY_HEIGHT=1080 DISPLAY_SCALE=2 make run-gui
```

These make variables are forwarded to the QEMU invocation and to boot.toml generation.
This is a convenience addition; the core implementation does not depend on it.

---

## 16. Interaction with Previous Rounds

### 16.1 R11 — Per-Window Surface Buffers

R11 introduced Surface buffers sized in physical pixels and a compositor that blits them.
R20 does not change the Surface memory layout or the compositor blit path. The only change
is that HiDPI-aware apps size their Surface at `logical_w × scale_factor` × `logical_h ×
scale_factor` physical pixels instead of the logical dimensions.

### 16.2 R13 — Bitmap Font Library

R13 defined the glyph bitmaps and the three specifier sizes (s/m/l). R20 adds a
`scale_factor` parameter to the glyph render function and adds the inner 2×2 pixel-block
write loop. The glyph data arrays in `font.rs` are not changed; they remain the compact
1-bit-per-pixel encoding.

### 16.3 R17 — VYOMA_DRAW v3 / `set-font`

R17's `set-font` command is extended with the optional `hidpi_hint` field as specified in
§4.4. Apps compiled against R17 that emit `set-font` without the third field continue to
work (the parser treats a missing field as `hidpi_hint=0`).

### 16.4 R18 — WIT Link-Time Capability Gating

R20's `vyoma:display-info@1.0.0` follows the R18 pattern exactly: the WIT import is
present only when `display = true` is in the manifest; otherwise it is stubbed. No new
capability-gating infrastructure is required.

### 16.5 R19 — Virtual Displays

R20 extends the `VYOMA_VDISP_SCREEN` message with a fourth `scale_factor` field. The R19
virtual display registry (in `vdisp.rs`) gains a `scale_factor: u8` per entry, defaulting
to the physical display's scale factor when a virtual display is created without an
explicit scale specification.

---

## 17. Sequence Diagrams

### 17.1 HiDPI App Startup (2x Display)

```
Supervisor                              gui-demo (hidpi_aware=true)
    |                                         |
    |-- spawn (wasmtime) ------------------->  |
    |                                         |
    |-- stdin: VYOMA_DISPLAY:window_info:     |
    |         960,540,2 ------------------->  |
    |                                         |
    |  <-- stdout: VYOMA_DRAW:fill_rect:      |
    |              0,0,960,540,<bg> --------  |
    |                                         |
    | scale: x=0*2=0, y=0*2=0,               |
    |        w=960*2=1920, h=540*2=1080       |
    |                                         |
    | write_pixels(Surface, 0,0,1920,1080)    |
    |                                         |
    |  <-- stdout: VYOMA_DRAW:flush --------  |
    |                                         |
    | compositor: blit Surface → fb           |
```

### 17.2 Legacy App at 2x (Backward Compatible)

```
Supervisor                              shell (hidpi_aware=false)
    |                                         |
    |-- spawn (wasmtime) ------------------->  |
    |                                         |
    |-- stdin: VYOMA_DISPLAY:window_info:     |
    |         960,540,1  ------------------>  |
    |  (scale_factor reported as 1 for        |
    |   non-hidpi apps)                       |
    |                                         |
    |  <-- stdout: VYOMA_DRAW:fill_rect:      |
    |              0,0,960,540,<bg> --------  |
    |                                         |
    | no scaling (hidpi_aware=false)          |
    | write_pixels(Surface, 0,0,960,540)      |
    |                                         |
    | compositor: blit 960×540 Surface        |
    |   into top-left of 1920×1080 fb         |
```

### 17.3 Virtual Display with Different Scale Factor

```
Supervisor                           app-demo (hidpi_aware=true)
    |                                         |
    | vdisp create vd0 1920 1080 2            |
    |                                         |
    | vdisp assign vd0 app-demo               |
    |                                         |
    |-- stdin: VYOMA_DISPLAY:display_changed: |
    |          960,540,2 ------------------>  |
    |-- stdin: VYOMA_VDISP_SCREEN:            |
    |          vd0,1920,1080,2 ------------>  |
    |                                         |
    |  <-- stdout: VYOMA_DRAW:fill_rect:      |
    |              0,0,960,540,<bg> --------  |
    |                                         |
    | scale: 0,0,1920,1080                    |
    | write to vd0 Surface                    |
```

---

## 18. Future Considerations

### 18.1 Fractional Scale Factors

When sub-pixel rendering is added (a future font round), fractional scale factors (1.25,
1.5, 1.75) become possible. The `scale_factor: u8` type would need to change to `f32` or
a fixed-point rational. The coordinate rounding policy (§3.3) would then become
non-trivial: floor vs round vs ceil would produce different visual results for different
command types. The current spec's explicit floor-policy statement is intended to establish
a documented default that a future spec can reference and override.

### 18.2 Per-Window Scale Factor

A future multi-monitor round may assign different physical displays to different app
windows. In that scenario, app A may be on a 1x display and app B on a 2x display within
the same supervisor session. The current per-display `DisplayConfig` is already designed
for this: each display (physical or virtual) has its own `scale_factor`. The per-app
`hidpi_aware` flag and the `VYOMA_DISPLAY:window_info:` message already carry the scale
factor for the specific display the app is on, so per-window scale factor is a natural
extension of the v1 model without protocol changes.

### 18.3 EDID-Based Scale Detection

When VyomaOS runs on real HiDPI hardware (e.g., a laptop panel with EDID reporting 220
DPI), the supervisor should auto-detect the appropriate scale factor. The code path for
this is stubbed in `display_info.rs` as `detect_scale_from_edid() -> Option<u8>`, always
returning `None` in v1. A future hardware-bring-up round replaces this stub with EDID
parsing via `/sys/class/drm/<connector>/edid` on real Linux hardware.

### 18.4 Asset Resolution Hints

A future round may define a `VYOMA_DRAW:load_image_2x:` variant or a `@2x` asset naming
convention analogous to iOS/macOS `image@2x.png`. This round intentionally defers that
problem: v1 HiDPI is purely about coordinate scaling and font pixel-doubling. Image
assets at 2x density require an image decoding pipeline (R14 territory) and are out of
scope here.

---

## 19. Design Decisions Summary

| Decision | Rationale |
|----------|-----------|
| Integer scale only (1x, 2x) | Fractional scaling requires sub-pixel rendering. The bitmap font library is not designed for non-integer scales. QEMU virtio-gpu cannot provide fractional EDID hint anyway. |
| Surfaces in physical pixels | Compositor is simpler; no scaling in the hot compositing path. Apps that know the scale factor produce correct-density content. |
| Opt-in `hidpi_aware` flag | Prevents coordinate doubling from breaking the 10+ existing WASM apps that emit physical-pixel coordinates. Backward compatibility is a hard requirement. |
| boot.toml override for scale factor | QEMU virtio-gpu has no EDID. A compile-time or runtime configuration hook is required for any HiDPI testing. boot.toml is the established configuration surface. |
| Pixel-doubling for fonts (no antialiasing) | Sub-pixel antialiasing requires knowledge of LCD subpixel layout. The bitmap font renderer uses a simple bit-per-pixel model. Pixel-doubling is lossless and correct. |
| Font size auto-promotion disabled | Auto-promotion changes layout in ways apps cannot predict. Apps request the specifier they need; the scale factor determines physical size. This is the macOS model: apps supply 2x assets, not auto-upscaled 1x assets. |
| `VYOMA_VDISP_SCREEN` extended with scale_factor | R19 contract preserved; new field appended; backward-compatible with R19 parsers. |
| No compositor changes | Compositor operates on physical pixels. Moving scale logic to draw_cmd.rs keeps the hot compositing path unchanged and avoids a performance regression. |
