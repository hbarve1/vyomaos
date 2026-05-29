# Round 17 — Advanced Rendering API (VYOMA_DRAW v3)
## VyomaOS Subsystem 17: 2D Rendering API (macOS equiv: Quartz 2D / CoreGraphics)

**Date**: 2026-05-29  
**Status**: Architect Draft  
**Prior rounds**: R11 (Surface/VSync), R12 (GPU WIT), R13 (Font/watchdog), R14 (ImageData), R15 (ColorSpace), R16 (Animation Engine)

---

## 1. Design Philosophy

### The Three-Version Strategy

VyomaOS inherits a working text-line drawing protocol from Round 11. That protocol (`VYOMA_DRAW:`) is battle-tested, legible in logs, and trivially implemented in any language that can write to stdout. It must not be broken. At the same time, production UI applications — a settings panel, a file browser, a browser chrome — require rounded rectangles, drop shadows, gradient fills, and composited layers. Text-line parsing is not a viable bottleneck for those workloads.

The solution is three protocol tiers that co-exist permanently:

**v1 — Text Protocol (R11, unchanged)**  
Commands: `fill_rect`, `draw_text`, `draw_text_wrap`, `flush`. No path, no gradient, no transform. Every platform supports it. Simple status widgets, boot screens, log viewers, and terminal overlays use v1 exclusively. The supervisor parser for v1 never changes.

**v2 — Extended Text Protocol (this spec)**  
Adds path construction, affine transforms, gradients, and image blitting to the same line-oriented stdout protocol. Fully backwards-compatible: a v1 app that never writes `begin_path` never triggers v2 parsing. Suitable for desktop applications on `mobile` and `desktop-full` platforms that want richer graphics without adopting the WIT component model. v2 parsing is handled by `supervisor/src/draw/protocol.rs`, a separate module from the R11 display parser.

**v3 — WIT Interface `vyoma:draw@3.0.0` (this spec)**  
A typed, binary-efficient WIT interface that eliminates all text serialization overhead. Apps call drawing functions as direct WASM imports rather than writing to stdout. The supervisor's Wasmtime linker registers the `vyoma:draw@3.0.0` world, and calls are dispatched synchronously into `supervisor/src/draw/wit_handlers.rs`. Only `mobile` and `desktop-full` platforms expose this interface; embedded platforms (mcu-minimal, iot-edge, robotics-rt) and server-headless do not need it.

### Which Apps Use Which Version

| App type | Protocol | Rationale |
|----------|----------|-----------|
| Boot splash, status bar | v1 | Zero dependencies, works on mcu-minimal |
| IoT sensor dashboard | v1 | 4 MB RAM ceiling; no path rasterizer |
| Simple text terminal | v1 | fill_rect + draw_text is sufficient |
| Transitional UI (graphs, charts) | v2 | Paths needed; no WIT component model yet |
| Settings panel, file browser | v3 | Full graphics model, glyph atlas, blends |
| Animation-driven UI (R16 layers) | v3 | draw_image + transform per frame |

### Relationship to Prior Rounds

R11 contributed: `DrawSurface`, per-window surface routing, vsync compositor.  
R12 contributed: GPU WIT world, wgpu surface handles.  
R13 contributed: `TextLayout`, `LayoutSession`, glyph atlas, watchdog.  
R14 contributed: `ImageHandle`, decoded pixel buffers, `blit_surface`.  
R15 contributed: linear-light sRGB pipeline, `ColorConvert`.  
R16 contributed: `Layer`, `PresentationLayer`, animation transaction model.  

R17 sits below R16: a layer's `contents` is rasterized via R17 when it holds a `DrawSurface` with a v2/v3 paint sequence. R17's `GraphicsContext` writes into a `DrawSurface` pixel buffer; R16 then composites that surface onto the display.

---

## 2. Core Rust Types

All types live in `supervisor/src/draw/mod.rs` (exported from there; implementations are in submodules).

```rust
// supervisor/src/draw/mod.rs

pub mod blend;
pub mod context;
pub mod gradient;
pub mod image;
pub mod protocol;
pub mod rasterizer;
pub mod stroke;
pub mod text;
pub mod transform;
pub mod wit_handlers;

use crate::display::DrawSurface;
use crate::images::ImageHandle;

// ── Path ─────────────────────────────────────────────────────────────────────

/// A sequence of path segments forming one or more subpaths.
/// Segments are always in the GraphicsContext's current user space.
#[derive(Debug, Clone, Default)]
pub struct VyomaPath {
    pub segments: Vec<PathSegment>,
    /// Cached bounding box in user space (None = dirty).
    pub(crate) bbox: Option<[f32; 4]>,
}

impl VyomaPath {
    pub fn new() -> Self { Self::default() }

    pub fn move_to(&mut self, x: f32, y: f32) {
        self.bbox = None;
        self.segments.push(PathSegment::MoveTo(x, y));
    }

    pub fn line_to(&mut self, x: f32, y: f32) {
        self.bbox = None;
        self.segments.push(PathSegment::LineTo(x, y));
    }

    pub fn curve_to(&mut self, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, ex: f32, ey: f32) {
        self.bbox = None;
        self.segments.push(PathSegment::CurveTo(cp1x, cp1y, cp2x, cp2y, ex, ey));
    }

    pub fn quad_to(&mut self, cpx: f32, cpy: f32, ex: f32, ey: f32) {
        self.bbox = None;
        self.segments.push(PathSegment::QuadTo(cpx, cpy, ex, ey));
    }

    pub fn arc_to(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, start_deg: f32, sweep_deg: f32) {
        self.bbox = None;
        // Clamp sweep to ±360°, rx/ry to (0, 65536]
        let sweep = sweep_deg.clamp(-360.0, 360.0);
        let rx = rx.clamp(f32::EPSILON, 65536.0);
        let ry = ry.clamp(f32::EPSILON, 65536.0);
        self.segments.push(PathSegment::ArcTo { cx, cy, rx, ry, start: start_deg, sweep });
    }

    pub fn close(&mut self) {
        self.segments.push(PathSegment::Close);
    }

    /// True if the path has no segments (empty or only MoveTo with no following segments).
    pub fn is_empty(&self) -> bool {
        self.segments.iter().all(|s| matches!(s, PathSegment::MoveTo(..)))
    }
}

/// Maximum segments per path. Guards against memory exhaustion from malicious apps.
pub const MAX_PATH_SEGMENTS: usize = 10_000;

#[derive(Debug, Clone, PartialEq)]
pub enum PathSegment {
    /// Begin a new subpath at (x, y). Does not draw.
    MoveTo(f32, f32),
    /// Straight line from current point to (x, y).
    LineTo(f32, f32),
    /// Cubic Bézier: control point 1 (cp1x, cp1y), control point 2 (cp2x, cp2y), end (ex, ey).
    CurveTo(f32, f32, f32, f32, f32, f32),
    /// Quadratic Bézier: control point (cpx, cpy), end (ex, ey).
    QuadTo(f32, f32, f32, f32),
    /// Elliptical arc centered at (cx, cy), radii (rx, ry), start_deg, sweep_deg.
    ArcTo { cx: f32, cy: f32, rx: f32, ry: f32, start: f32, sweep: f32 },
    /// Close the current subpath with a straight line back to the last MoveTo.
    Close,
}

// ── Paint ─────────────────────────────────────────────────────────────────────

/// A gradient color stop. Position in [0.0, 1.0]. Color in linear-light sRGB.
#[derive(Debug, Clone)]
pub struct GradientStop {
    pub position: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// Maximum gradient stops per gradient. Prevents O(N) stop evaluation at scale.
pub const MAX_GRADIENT_STOPS: usize = 64;

#[derive(Debug, Clone)]
pub struct LinearGradientSpec {
    /// Start and end points in user space.
    pub x0: f32, pub y0: f32,
    pub x1: f32, pub y1: f32,
    pub stops: Vec<GradientStop>,
    pub extend: GradientExtend,
}

#[derive(Debug, Clone)]
pub struct RadialGradientSpec {
    /// Center of the gradient circle in user space.
    pub cx: f32, pub cy: f32,
    /// Inner radius (focal circle); set to 0 for simple radial.
    pub focal_x: f32, pub focal_y: f32,
    pub radius: f32,
    pub stops: Vec<GradientStop>,
    pub extend: GradientExtend,
}

/// How gradient color is extended outside the [0, 1] stop range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GradientExtend {
    /// Clamp to endpoint color.
    Pad,
    /// Reflect: 0→1, 1→0, 0→1, …
    Reflect,
    /// Repeat: 0→1, 0→1, …
    Repeat,
}

/// A tiled image pattern. The image is repeated at `tile_width × tile_height`.
#[derive(Debug, Clone)]
pub struct PatternSpec {
    pub image: ImageHandle,
    pub tile_width: f32,
    pub tile_height: f32,
    pub transform: AffineTransform,
}

/// An opaque reference to a stored pattern (index into per-context pattern table).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PatternRef(pub u32);

/// Fill or stroke paint specification.
#[derive(Debug, Clone)]
pub enum VyomaPaint {
    /// Solid color in linear-light sRGB with premultiplied alpha.
    SolidColor(f32, f32, f32, f32),
    LinearGradient(LinearGradientSpec),
    RadialGradient(RadialGradientSpec),
    /// Tiled image pattern.
    Pattern(PatternRef),
}

// ── Blend modes ───────────────────────────────────────────────────────────────

/// Porter-Duff compositing operators + W3C compositing blend modes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BlendMode {
    /// Standard Porter-Duff src-over.
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    SoftLight,
    Difference,
    Exclusion,
    /// Set destination alpha to zero in source region (erase).
    Clear,
}

// ── Line style ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineCap { Butt, Round, Square }

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LineJoin { Miter, Round, Bevel }

#[derive(Debug, Clone)]
pub struct StrokeStyle {
    pub line_width: f32,
    pub cap: LineCap,
    pub join: LineJoin,
    pub miter_limit: f32,
    pub dash_pattern: Vec<f32>,
    pub dash_offset: f32,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            line_width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            dash_pattern: vec![],
            dash_offset: 0.0,
        }
    }
}

// ── Fill rule ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FillRule { EvenOdd, NonZero }

// ── Draw commands ─────────────────────────────────────────────────────────────

/// High-level draw commands dispatched from v2 protocol parser or v3 WIT handlers
/// into the GraphicsContext execution engine.
#[derive(Debug, Clone)]
pub enum DrawCommand {
    FillPath   { path: VyomaPath, paint: VyomaPaint, rule: FillRule },
    StrokePath { path: VyomaPath, paint: VyomaPaint, style: StrokeStyle },
    FillRect   { x: f32, y: f32, w: f32, h: f32, paint: VyomaPaint },
    StrokeRect { x: f32, y: f32, w: f32, h: f32, paint: VyomaPaint, style: StrokeStyle },
    FillEllipse { cx: f32, cy: f32, rx: f32, ry: f32, paint: VyomaPaint },
    FillText   { text: String, font_size: f32, x: f32, y: f32, paint: VyomaPaint },
    DrawImage  {
        image: ImageHandle,
        src_x: f32, src_y: f32, src_w: f32, src_h: f32,
        dst_x: f32, dst_y: f32, dst_w: f32, dst_h: f32,
        alpha: f32,
    },
    ClipPath   { path: VyomaPath, rule: FillRule },
    ClipRect   { x: f32, y: f32, w: f32, h: f32 },
    SaveState,
    RestoreState,
    SetTransform  (AffineTransform),
    ConcatTransform(AffineTransform),
    ResetTransform,
    Flush,
}

// ── Surface ────────────────────────────────────────────────────────────────────

/// A software render target. Pixel format: BGRA8 premultiplied, linear-light sRGB.
/// This is the same DrawSurface definition established in R11 and extended in R16.
#[derive(Debug)]
pub struct DrawSurface {
    pub width: u32,
    pub height: u32,
    /// Bytes per row. May be > width * 4 for alignment.
    pub stride: u32,
    /// BGRA8 pixel buffer. Length == stride * height.
    pub pixels: Vec<u8>,
}

impl DrawSurface {
    pub fn new(width: u32, height: u32) -> Self {
        let stride = width * 4;
        Self {
            width,
            height,
            stride,
            pixels: vec![0u8; (stride * height) as usize],
        }
    }

    #[inline]
    pub fn pixel_at_mut(&mut self, x: u32, y: u32) -> &mut [u8] {
        let offset = (y * self.stride + x * 4) as usize;
        &mut self.pixels[offset..offset + 4]
    }
}
```

---

## 3. Text Rendering Integration (R13)

R13 defined `TextLayout` and `LayoutSession`. R17's `FillText` command calls into R13 as follows:

### Call Chain for `FillText`

```
DrawCommand::FillText { text, font_size, x, y, paint }
    │
    ▼
draw/text.rs :: draw_text_at(ctx, &text, font_size, x, y, &paint)
    │
    ├─ 1. Resolve font descriptor from ctx.current_font (family, weight, style)
    ├─ 2. Look up glyph atlas: atlas.get_or_rasterize(font_desc, font_size, glyph_id)
    │       returns GlyphBitmap { alpha_pixels, advance_x, bearing_x, bearing_y }
    │
    ├─ 3. TextLayout::from_string(&text, &font_desc, font_size)
    │       returns LayoutSession { glyphs: Vec<PositionedGlyph> }
    │       where PositionedGlyph { id: GlyphId, x: f32, y: f32 }
    │
    └─ 4. For each PositionedGlyph:
            let bmp = atlas.get_or_rasterize(font_desc, font_size, glyph.id);
            match &paint {
                VyomaPaint::SolidColor(r, g, b, a) =>
                    blit_glyph_alpha(surface, &bmp, glyph.x + x, glyph.y + y, r, g, b, a),
                _ =>
                    fill_glyph_as_path(ctx, surface, &bmp, glyph.x + x, glyph.y + y, &paint),
            }
```

### `blit_glyph_alpha`

For solid color text (the common case), the glyph alpha mask is blitted directly onto the destination surface. Each pixel:

```rust
// Per-pixel glyph blit (solid color fast path)
let coverage = bmp.alpha_pixels[py * bmp.width + px] as f32 / 255.0;
let src_alpha = a * coverage;
// Porter-Duff src-over, premultiplied
let dst = surface.pixel_at_mut(dx, dy);
let dst_a = dst[3] as f32 / 255.0;
let out_a = src_alpha + dst_a * (1.0 - src_alpha);
let blend = |src_c: f32, dst_c: f32| src_c * src_alpha + dst_c * dst_a * (1.0 - src_alpha);
dst[0] = (blend(b, dst[0] as f32 / 255.0) / out_a * 255.0) as u8;
dst[1] = (blend(g, dst[1] as f32 / 255.0) / out_a * 255.0) as u8;
dst[2] = (blend(r, dst[2] as f32 / 255.0) / out_a * 255.0) as u8;
dst[3] = (out_a * 255.0) as u8;
```

### `fill_glyph_as_path`

For gradient or pattern text, the glyph bitmap is used as an alpha mask. The fill paint is evaluated at each pixel position and multiplied by the glyph coverage value:

```rust
for py in 0..bmp.height {
    for px in 0..bmp.width {
        let coverage = bmp.alpha_pixels[py * bmp.width + px] as f32 / 255.0;
        if coverage < 1.0 / 255.0 { continue; }
        let world_x = x + px as f32;
        let world_y = y + py as f32;
        let (pr, pg, pb, pa) = evaluate_paint(paint, world_x, world_y);
        blend_pixel(surface, px as u32, py as u32, pr, pg, pb, pa * coverage, blend_mode);
    }
}
```

### Glyph Atlas Integration

The glyph atlas (R13) is stored per-font-size. The R17 draw/text module holds a shared reference (`Arc<Mutex<GlyphAtlas>>`). The atlas key is `(FontDescriptor, font_size_rounded_to_half_pt, GlyphId)`. Cache hits avoid all rasterization cost.

---

## 4. WIT Interface `vyoma:draw@3.0.0`

```wit
// supervisor/wit/draw.wit
package vyoma:draw@3.0.0;

interface drawing {
    // ── Blend modes ───────────────────────────────────────────────────────────
    enum blend-mode {
        normal,
        multiply,
        screen,
        overlay,
        darken,
        lighten,
        color-dodge,
        color-burn,
        hard-light,
        soft-light,
        difference,
        exclusion,
        clear,
    }

    enum fill-rule {
        even-odd,
        non-zero,
    }

    enum line-cap {
        butt,
        round,
        square,
    }

    enum line-join {
        miter,
        round-join,
        bevel,
    }

    enum gradient-extend {
        pad,
        reflect,
        repeat,
    }

    record rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
    }

    record affine {
        a: f32,  // scale-x
        b: f32,  // skew-y
        c: f32,  // skew-x
        d: f32,  // scale-y
        tx: f32, // translate-x
        ty: f32, // translate-y
    }

    // ── Context lifecycle ─────────────────────────────────────────────────────

    /// Create a graphics context bound to a surface handle (from R11 surface-create).
    /// Returns an opaque context handle, or an error string.
    create-context: func(surface-id: u32) -> result<u32, string>;

    /// Destroy a graphics context and release its state stack.
    destroy-context: func(ctx: u32) -> result<_, string>;

    // ── Path construction ─────────────────────────────────────────────────────

    /// Create a new empty path. Returns an opaque path handle.
    new-path: func(ctx: u32) -> result<u32, string>;

    /// Release a path handle.
    drop-path: func(ctx: u32, path: u32) -> result<_, string>;

    move-to:    func(ctx: u32, path: u32, x: f32, y: f32) -> result<_, string>;
    line-to:    func(ctx: u32, path: u32, x: f32, y: f32) -> result<_, string>;
    curve-to:   func(ctx: u32, path: u32, cp1x: f32, cp1y: f32, cp2x: f32, cp2y: f32, ex: f32, ey: f32) -> result<_, string>;
    quad-to:    func(ctx: u32, path: u32, cpx: f32, cpy: f32, ex: f32, ey: f32) -> result<_, string>;
    arc-to:     func(ctx: u32, path: u32, cx: f32, cy: f32, rx: f32, ry: f32, start-deg: f32, sweep-deg: f32) -> result<_, string>;
    close-path: func(ctx: u32, path: u32) -> result<_, string>;

    // ── Paint / style ─────────────────────────────────────────────────────────

    set-fill-color:   func(ctx: u32, r: f32, g: f32, b: f32, a: f32) -> result<_, string>;
    set-stroke-color: func(ctx: u32, r: f32, g: f32, b: f32, a: f32) -> result<_, string>;
    set-line-width:   func(ctx: u32, width: f32) -> result<_, string>;
    /// dash-array is a list of on/off lengths in user units. Empty = solid line.
    set-line-dash:    func(ctx: u32, dash-array: list<f32>, offset: f32) -> result<_, string>;
    set-line-cap:     func(ctx: u32, cap: line-cap) -> result<_, string>;
    set-line-join:    func(ctx: u32, join: line-join) -> result<_, string>;
    set-blend-mode:   func(ctx: u32, mode: blend-mode) -> result<_, string>;
    set-global-alpha: func(ctx: u32, alpha: f32) -> result<_, string>;

    // ── Fill paint setters ────────────────────────────────────────────────────

    set-fill-gradient: func(ctx: u32, gradient: u32) -> result<_, string>;
    set-fill-pattern:  func(ctx: u32, pattern: u32) -> result<_, string>;

    // ── Gradient construction ─────────────────────────────────────────────────

    /// Create a linear gradient from (x0,y0) to (x1,y1) in user space.
    create-linear-gradient: func(ctx: u32, x0: f32, y0: f32, x1: f32, y1: f32, extend: gradient-extend) -> result<u32, string>;

    /// Create a radial gradient centered at (cx,cy) with the given radius.
    create-radial-gradient: func(ctx: u32, cx: f32, cy: f32, radius: f32, focal-x: f32, focal-y: f32, extend: gradient-extend) -> result<u32, string>;

    /// Add a stop to a gradient handle. Colors in linear-light sRGB.
    add-gradient-stop: func(ctx: u32, gradient: u32, position: f32, r: f32, g: f32, b: f32, a: f32) -> result<_, string>;

    /// Release a gradient handle.
    drop-gradient: func(ctx: u32, gradient: u32) -> result<_, string>;

    // ── Pattern construction ──────────────────────────────────────────────────

    /// Create a tiled image pattern from an image handle (R14).
    create-pattern: func(ctx: u32, image: u32, tile-w: f32, tile-h: f32, transform: affine) -> result<u32, string>;
    drop-pattern:   func(ctx: u32, pattern: u32) -> result<_, string>;

    // ── Draw operations ───────────────────────────────────────────────────────

    fill-path:    func(ctx: u32, path: u32, rule: fill-rule) -> result<_, string>;
    stroke-path:  func(ctx: u32, path: u32) -> result<_, string>;
    fill-rect:    func(ctx: u32, x: f32, y: f32, w: f32, h: f32) -> result<_, string>;
    stroke-rect:  func(ctx: u32, x: f32, y: f32, w: f32, h: f32) -> result<_, string>;
    fill-ellipse: func(ctx: u32, cx: f32, cy: f32, rx: f32, ry: f32) -> result<_, string>;

    /// Render text at (x, y) using the current fill paint and font.
    draw-text: func(ctx: u32, text: string, font-size: f32, x: f32, y: f32) -> result<_, string>;

    /// Blit a subregion of an image (src-rect) into a destination rectangle (dst-rect).
    draw-image: func(ctx: u32, image: u32, src-x: f32, src-y: f32, src-w: f32, src-h: f32, dst-x: f32, dst-y: f32, dst-w: f32, dst-h: f32) -> result<_, string>;

    // ── Clipping ─────────────────────────────────────────────────────────────

    clip-path: func(ctx: u32, path: u32, rule: fill-rule) -> result<_, string>;
    clip-rect: func(ctx: u32, x: f32, y: f32, w: f32, h: f32) -> result<_, string>;

    // ── State stack ───────────────────────────────────────────────────────────

    /// Push the current graphics state (transform, clip, paint, style) onto the stack.
    /// Returns error if depth would exceed 64.
    save-state:    func(ctx: u32) -> result<_, string>;
    restore-state: func(ctx: u32) -> result<_, string>;

    // ── Transforms ───────────────────────────────────────────────────────────

    /// Replace the current transform matrix.
    set-transform:    func(ctx: u32, m: affine) -> result<_, string>;
    /// Pre-multiply the current transform by m.
    concat-transform: func(ctx: u32, m: affine) -> result<_, string>;
    reset-transform:  func(ctx: u32) -> result<_, string>;

    // ── Flush ─────────────────────────────────────────────────────────────────

    /// Commit all pending draw commands to the surface pixel buffer.
    /// The surface becomes visible at the next vsync tick (R11).
    flush: func(ctx: u32) -> result<_, string>;
}

world draw-guest {
    import drawing;
}
```

---

## 5. Extended Text Protocol (v2 Backwards-Compatible Commands)

The following commands extend the existing `VYOMA_DRAW:` line protocol. They are parsed in `supervisor/src/draw/protocol.rs`. Apps that never write these commands are not affected.

```
# Path construction — must be bracketed by begin_path / fill or stroke
VYOMA_DRAW:begin_path
VYOMA_DRAW:move_to:<x>,<y>
VYOMA_DRAW:line_to:<x>,<y>
VYOMA_DRAW:curve_to:<cp1x>,<cp1y>,<cp2x>,<cp2y>,<ex>,<ey>
VYOMA_DRAW:quad_to:<cpx>,<cpy>,<ex>,<ey>
VYOMA_DRAW:arc:<cx>,<cy>,<rx>,<ry>,<start_deg>,<sweep_deg>
VYOMA_DRAW:close_path

# Paint and fill — issue after path construction
VYOMA_DRAW:fill:<r>,<g>,<b>,<a>
VYOMA_DRAW:stroke:<r>,<g>,<b>,<a>,<line_width>
VYOMA_DRAW:fill_rule:<even_odd|non_zero>

# Gradient paint — build gradient before fill_gradient
VYOMA_DRAW:linear_gradient:<x0>,<y0>,<x1>,<y1>,<extend>
    # extend = pad | reflect | repeat
VYOMA_DRAW:radial_gradient:<cx>,<cy>,<radius>,<focal_x>,<focal_y>,<extend>
VYOMA_DRAW:gradient_stop:<pos>,<r>,<g>,<b>,<a>
    # pos in [0.0, 1.0]; up to MAX_GRADIENT_STOPS (64) stops
VYOMA_DRAW:fill_gradient
    # fills current path with the most recently defined gradient

# Image blit
VYOMA_DRAW:draw_image:<image_handle>,<sx>,<sy>,<sw>,<sh>,<dx>,<dy>,<dw>,<dh>
    # image_handle = u32 from R14 image-load WIT call
    # src rect: sx,sy,sw,sh in image pixels
    # dst rect: dx,dy,dw,dh in surface pixels

# Affine transform (2×3 matrix: a b c d tx ty)
VYOMA_DRAW:set_transform:<a>,<b>,<c>,<d>,<tx>,<ty>
VYOMA_DRAW:concat_transform:<a>,<b>,<c>,<d>,<tx>,<ty>
VYOMA_DRAW:reset_transform

# Clipping
VYOMA_DRAW:clip_rect:<x>,<y>,<w>,<h>
VYOMA_DRAW:clip_path
    # clips to the currently built path (built with begin_path...close_path)

# State stack
VYOMA_DRAW:save_state
VYOMA_DRAW:restore_state

# Blend mode
VYOMA_DRAW:set_blend_mode:<normal|multiply|screen|overlay|darken|lighten|color_dodge|color_burn|hard_light|soft_light|difference|exclusion|clear>

# Global alpha (applied on top of per-paint alpha)
VYOMA_DRAW:set_alpha:<value>
    # value in [0.0, 1.0]
```

### State Machine Rules (v2 Protocol)

The v2 parser maintains state in `ProtocolState`:

```
Idle → begin_path → PathBuilding → (fill / stroke / fill_gradient / clip_path) → Idle
```

If the app sends a `fill_rect` or `draw_text` (v1 commands) while in `PathBuilding`, the supervisor logs a warning and discards the current path state, returning to `Idle`. This prevents dangling path state from crashing the supervisor when an app dies mid-sequence (see Security section for full crash handling).

---

## 6. Supervisor-Side Rendering Engine — Module Layout

The rendering engine is split into ten focused files, each under the 500-line limit.

### `supervisor/src/draw/mod.rs`
Public types and re-exports. Defines `DrawCommand`, `VyomaPath`, `VyomaPaint`, `GraphicsContext` (struct shell), `DrawSurface`. Contains `draw_command` dispatch function that calls into submodules. Target: ≤ 300 lines of types; implementation delegates immediately.

### `supervisor/src/draw/context.rs`
`GraphicsContext` implementation: save/restore stack, clip stack management, current paint/style accessors. The state stack has a hard cap of 64 entries. `restore_state` when the stack is empty is a no-op (logged as warning, not panic). `clip_stack` stores a stack of `Option<VyomaPath>` clip masks; intersection of the active clips is computed at paint time.

### `supervisor/src/draw/rasterizer.rs`
Scanline polygon fill. Takes a `VyomaPath` + `FillRule` + `DrawSurface` + `VyomaPaint` + `AffineTransform` + `BlendMode`. Returns `Ok(())` or `Err(RasterizerError)`. Internal helpers: `flatten_path` (converts all segments to line segments via de Casteljau), `build_edge_table`, `scanline_fill`. Target: ≤ 480 lines including tests.

### `supervisor/src/draw/stroke.rs`
Converts a stroked path into a filled path (stroke expansion). Inputs: `VyomaPath` + `StrokeStyle` + `AffineTransform`. Output: a new `VyomaPath` representing the stroke outline, which is then passed to `rasterizer::fill`. Handles dash expansion, butt/round/square caps, miter/round/bevel joins. Miter fallback to bevel when `miter_length > miter_limit * line_width`.

### `supervisor/src/draw/gradient.rs`
`evaluate_linear(grad, x, y)` → `(r, g, b, a)` and `evaluate_radial(grad, x, y)` → `(r, g, b, a)`. All interpolation in linear-light sRGB (R15 `ColorConvert` used for stop values if input is tagged sRGB). `interpolate_stops(stops, t)`: binary search for enclosing stops, linear lerp in linear-light space. The gradient color space is always linear-light sRGB regardless of the display color space. Apps wishing P3 gamut should pre-convert stop colors via R15 WIT.

### `supervisor/src/draw/blend.rs`
`blend_pixel(dst: &mut [u8; 4], src_r: f32, src_g: f32, src_b: f32, src_a: f32, mode: BlendMode)`. Implements all 13 blend modes. All math in linear-light f32; final result converted to u8 with rounding. The `Normal` mode uses standard Porter-Duff src-over on premultiplied values.

### `supervisor/src/draw/text.rs`
`draw_text_at(ctx, surface, text, font_size, x, y, paint)` — see Section 3. Also owns the `GlyphAtlas` reference (`Arc<Mutex<GlyphAtlas>>`) and exposes `measure_text(text, font_size) -> (f32, f32)` for layout queries. Target: ≤ 400 lines.

### `supervisor/src/draw/image.rs`
`draw_image(surface, image, src_rect, dst_rect, alpha, blend_mode)`. Bilinear interpolation when scaling. Clamps src_rect to image bounds; clamps dst_rect to surface bounds. Handles integer-pixel blits as a fast path (no interpolation). Uses R14 `ImageHandle` to read decoded pixel data.

### `supervisor/src/draw/transform.rs`
`AffineTransform`: a 2×3 matrix `[[a, b, tx], [c, d, ty]]`. Operations: `identity()`, `translate(tx, ty)`, `rotate(angle_rad)`, `scale(sx, sy)`, `concat(other)`, `apply_to_point(x, y) -> (f32, f32)`, `apply_to_path(path) -> VyomaPath`. The transform stack is managed in `context.rs`; `transform.rs` is pure math.

### `supervisor/src/draw/protocol.rs`
Parses v2 text lines from `VYOMA_DRAW:`. Entry point: `parse_v2_line(state: &mut ProtocolState, line: &str) -> Option<DrawCommand>`. Extends the R11 display parser by recognizing the new commands listed in Section 5. Unknown commands fall through to the existing R11 handler (preserving v1 compatibility). The `ProtocolState` struct holds in-progress path, in-progress gradient, and the current parser state machine enum.

### `supervisor/src/draw/wit_handlers.rs`
Links the `vyoma:draw@3.0.0` WIT world into the Wasmtime linker. Each WIT function maps to a host function that validates handles, checks bounds, and calls the appropriate `GraphicsContext` method. Context handles are stored in a `DrawHandleTable` (same pattern as R16 `LayerHandleTable`): a `HashMap<u32, GraphicsContext>` keyed by a monotonically incrementing `u32` handle. Path and gradient handles are per-context sub-tables. Handle 0 is always invalid.

---

## 7. Rasterizer Algorithm

### Overview

The scanline fill algorithm is the industry-standard approach for software rasterization of filled polygons. It operates in three passes: (1) flatten all curves to line segments, (2) build a sorted edge table, (3) sweep scanlines top-to-bottom maintaining an active edge table.

### Pass 1: Path Flattening (de Casteljau)

Every non-linear segment is recursively subdivided until the control polygon is flat:

```
fn flatten_cubic(p0, p1, p2, p3, tolerance, out: &mut Vec<(f32,f32)>) {
    // Flatness criterion: distance from line (p0→p3) to farthest control point
    let d1 = point_line_dist(p1, p0, p3);
    let d2 = point_line_dist(p2, p0, p3);
    if d1 < tolerance && d2 < tolerance {
        out.push(p3);
        return;
    }
    // de Casteljau split at t = 0.5
    let m01 = midpoint(p0, p1);
    let m12 = midpoint(p1, p2);
    let m23 = midpoint(p2, p3);
    let m012 = midpoint(m01, m12);
    let m123 = midpoint(m12, m23);
    let m0123 = midpoint(m012, m123);
    flatten_cubic(p0, m01, m012, m0123, tolerance, out);
    flatten_cubic(m0123, m123, m23, p3, tolerance, out);
}
```

Tolerance is `0.5 / scale_factor` where `scale_factor` is the larger of `|a|` and `|d|` from the current affine transform. This keeps flatness in device pixels, not user units.

Arc segments are converted to cubic Bézier approximations (Maisonobe method: max error < 0.001 user units per quadrant) before entering the same flatten pass.

### Pass 2: Edge Table Construction

The flattened polygon edges are sorted into a bucket array indexed by `min_y`:

```
struct Edge {
    y_max: i32,
    x_at_y_min: f32,    // x-intercept at the start scanline
    inv_slope: f32,     // Δx per scanline
    winding: i8,        // +1 (upward) or -1 (downward) for non-zero rule
}
```

Multiple subpaths (from `MoveTo`) each contribute their own edges. The `Close` segment adds an edge back to the last `MoveTo` point.

### Pass 3: Scanline Fill

For each scanline `y` from `y_min` to `y_max`:

1. Add all edges from bucket `y` to the Active Edge Table (AET).
2. Remove edges from AET where `y >= y_max`.
3. Sort AET by `x_at_y_min`.
4. Traverse AET pairs (or use winding count for non-zero), filling horizontal spans on the `DrawSurface`.
5. Advance all `x_at_y_min += inv_slope`.

### Anti-Aliasing (4×4 MSAA)

For edges: instead of a hard on/off test, the pixel is sampled at a 4×4 grid of subpixel positions. The coverage value (0.0–1.0) is the fraction of samples that fall inside the path. This coverage multiplies the paint alpha before blend.

For large interior spans (more than 2 pixels from any edge), coverage = 1.0 and the fast fill path is used (no subsampling).

### Complexity

A path with `N` segments produces `O(N)` edges. Each scanline processes its active edges in `O(k log k)` time where `k` is active edge count. For typical UI paths (< 100 segments, < 800px height), rasterization completes in under 1ms on a single core.

---

## 8. Performance

### Baseline

Software path rasterization on a single 3GHz x86-64 core achieves approximately 100–300 million pixel-fill operations per second for simple polygons. A 200×100 filled rounded rectangle = 20,000 pixels ≈ 0.1ms. A 960×540 full-screen gradient fill ≈ 2ms. These are acceptable for a UI application frame budget of 16.6ms at 60Hz.

### GPU Acceleration (R12 integration)

When R12's `wgpu` context is available on `desktop-full` or `mobile`, large solid-color fills and gradient fills can be offloaded:

- Fills larger than `GPU_FILL_THRESHOLD = 4096` pixels route to `draw/gpu_fill.rs` (future spec, stubbed in R17 as a compile-time feature flag `#[cfg(feature = "gpu-fill")]`).
- Path rasterization remains CPU-side; only the pixel fill operation is GPU-accelerated.
- The `DrawSurface` pixel buffer is uploaded to a wgpu staging buffer for the GPU fill, then downloaded back. Round-trip overhead is ≈ 0.3ms on discrete GPU; only beneficial for fills > 50,000 pixels.

### Glyph Cache

The glyph atlas (R13) eliminates redundant rasterization. Atlas key = `(family, weight, style, size_f32_bits, glyph_id)`. Cache size limit: 4 MB per app. On cache miss, rasterization takes ≈ 10–50µs per glyph depending on complexity.

### Dirty Region Tracking

Each `DrawSurface` maintains a `dirty_rect: Option<[u32; 4]>` (x, y, w, h). Every draw operation expands the dirty rect to the bounding box of the operation. At `flush`, only the dirty region is copied to the framebuffer (R11 vsync compositor). For a typical status-bar update (a small text redraw), this limits compositing work to < 1,000 pixels.

### Path Caching

Apps that draw the same path every frame (e.g., a fixed icon shape) can cache the rasterized edge table. The edge table is invalidated when the path's transform changes. This is a future optimization; R17 does not implement path caching, but the `VyomaPath::bbox` field is a prerequisite for it.

---

## 9. Platform Matrix

| Feature | mcu-minimal (128KB) | iot-edge (4MB) | robotics-rt (8MB) | server-headless (1GB) | mobile (256MB) | desktop-full (512MB) |
|---------|:-------------------:|:--------------:|:------------------:|:---------------------:|:--------------:|:--------------------:|
| v1 text protocol | yes | yes | yes | yes | yes | yes |
| v2 path commands | no | basic¹ | basic¹ | no | yes | yes |
| v3 WIT interface | no | no | no | no | yes | yes |
| Affine transform | no | no | no | no | yes | yes |
| Gradients | no | no | no | no | yes | yes |
| Image blit (draw_image) | no | yes | yes | no | yes | yes |
| Blend modes | no | no | no | no | yes | yes |
| Text (FillText / draw_text) | v1 only | v1 only | v1 only | v1 only | v1+v3 | v1+v3 |
| Glyph atlas (R13) | no | no | no | no | yes | yes |
| GPU-accelerated fill | no | no | no | no | yes² | yes² |
| Anti-aliased edges | no | no | no | no | yes | yes |
| Clip path | no | no | no | no | yes | yes |
| State stack (save/restore) | no | no | no | no | yes | yes |
| MAX_PATH_SEGMENTS | — | 500 | 500 | — | 10,000 | 10,000 |

¹ `basic`: `move_to`, `line_to`, `close_path`, `fill` only. No curves, no gradients, no transforms.  
² GPU fill is a compile-time feature flag; not enabled by default in R17 builds.

### mcu-minimal (128KB RAM)

The entire `supervisor/src/draw/` subtree is excluded from the mcu-minimal build via `#[cfg(not(target = "mcu-minimal"))]`. The v1 `fill_rect` and `draw_text` handlers remain compiled. Total draw subsystem footprint on mcu-minimal: < 8KB.

### server-headless

Server-headless has no display device (`/dev/fb0` absent). The v1 protocol is accepted syntactically (for apps that share source with display-capable builds) but commands are no-ops. The draw subtree compiles but `DrawSurface::flush()` returns immediately. No v2/v3 parsers are linked.

---

## 10. Security

### Handle Isolation

Context handles, path handles, and gradient handles are integers valid only within the app's `DrawHandleTable`. Handle values are allocated from a per-app counter starting at 1 (0 = always invalid). When an app is killed, its entire `DrawHandleTable` is dropped, releasing all associated `GraphicsContext`, `VyomaPath`, and gradient allocations. There is no way for App A to pass its context handle to App B (WIT handles are not transmittable across component boundaries).

### Path Segment Cap

`VyomaPath::move_to`, `line_to`, etc., check `segments.len() < MAX_PATH_SEGMENTS` before pushing. If the cap is reached, the call returns `Err("path segment limit reached")` via WIT, or the v2 protocol silently discards the segment and logs a warning. The cap prevents a single app from allocating O(N) memory or O(N) CPU time in the rasterizer.

### Gradient Stop Cap

`add_gradient_stop` returns an error if the gradient already has `MAX_GRADIENT_STOPS` (64) stops. Existing stops are unchanged. Evaluation still proceeds with the stops that were accepted.

### State Stack Cap

`save_state` returns `Err("state stack depth limit 64 exceeded")` via WIT if the stack already has 64 entries. The state is NOT pushed; the current graphics state is unchanged. For v2 text protocol, a warning is logged and the `save_state` command is ignored. A corresponding `restore_state` later will pop whatever is on the stack (which may be one level less than the app expects). This is a defined, recoverable behavior, not a crash.

`restore_state` when the stack is empty: returns `Err("state stack underflow")` via WIT, or logs a warning and is a no-op in v2. The context state is unchanged (no reset to default).

### Malformed Arc

`ArcTo` parameters are clamped: `sweep_deg` → `[-360.0, 360.0]`, `rx`/`ry` → `[ε, 65536.0]`. A zero or negative radius cannot produce a valid arc and is rejected with `Err("arc radius must be positive")`.

### App Crash Mid-Path (v2)

When an app's Wasmtime process exits, the supervisor's stdout reader closes. The `ProtocolState` for that app is stored in `AppState` (keyed by `AppId`). On app exit, the supervisor calls `app_state.draw_protocol_state = ProtocolState::default()`. Any in-progress path or gradient is dropped. The associated `DrawSurface` is not cleared; its last committed frame remains visible until the app restarts or is explicitly cleared by the session manager.

### Integer Overflow in Rasterizer

All scanline indices are checked against surface bounds before writing. The edge table is built with `i32` y-coordinates clipped to `[0, surface.height as i32 - 1]`. Horizontal span writes are clamped to `[0, surface.width)`. A path that maps entirely outside the surface produces no writes (clipped out at edge-table build time).

### Denial-of-Service via Complex Path

The flatten pass has a depth limit of `MAX_FLATTEN_DEPTH = 32` recursive subdivisions per cubic segment. This limits the worst-case flattening time to O(2^32) per segment in theory, but in practice any bezier flattens to tolerance in < 20 levels. The cap is a backstop against degenerate inputs where endpoints are numerically identical (producing no flatness improvement on subdivision). If the depth limit is hit, the segment is added as a straight line to its endpoint.

---

## Summary

VYOMA_DRAW v3 brings VyomaOS's rendering capabilities to parity with a subset of macOS Quartz 2D/CoreGraphics — path filling and stroking, gradients, image compositing, affine transforms, and text rendering via the R13 glyph atlas — while maintaining strict backwards compatibility with the v1 text protocol used by all existing apps. The three-tier design (v1/v2/v3) allows the system to serve a 128KB MCU and a 512MB desktop from a single codebase, with each tier adding capability without forcing migration. The rasterizer is a pure-software scanline fill suitable for UI frame rates on all display-capable platforms; GPU acceleration is a future upgrade path already scaffolded in the platform matrix.
